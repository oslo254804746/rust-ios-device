//! Exact, creation-owned private proxy flows. No runtime or task owner cycles.
use super::{runtime_storage::RuntimeRoot, TunnelError};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, SocketAddrV6},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpSocket, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
};

struct Flow {
    peer: SocketAddr,
    endpoint: SocketAddrV6,
    source_port: u16,
    claimed: bool,
    // Same socket as the stream, preventing its exact local tuple being taken
    // by an unregistered client while this private lifetime remains retained.
    loopback: Socket,
    // An actual wildcard IPv6 bind, never a numeric-only port reservation.
    _kernel_port: Option<Socket>,
}
struct Entries {
    values: Vec<Option<Flow>>,
    _storage: OwnedSemaphorePermit,
}
struct State {
    public: bool,
    closed: bool,
    entries: Option<Entries>,
    retiring: bool,
    leases: usize,
}
struct Registry {
    state: Mutex<State>,
    proxy_port: u16,
    reserve_for_kernel: bool,
}

#[derive(Clone)]
pub(crate) struct PrivateConnector(RuntimeRoot<Registry>);

mod filter;
pub(super) use filter::{Direction, FilterOutput, PrivateFilter};

#[derive(Clone, Copy)]
pub(super) enum Access {
    Public,
    Private {
        endpoint: SocketAddrV6,
        source_port: u16,
    },
}

impl PrivateConnector {
    pub(super) fn filter(
        &self,
        client: Ipv6Addr,
        bank: &Arc<Semaphore>,
    ) -> Result<PrivateFilter, TunnelError> {
        let state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        let entries = state
            .entries
            .as_ref()
            .ok_or_else(|| TunnelError::Io(io::Error::other(crate::OperationCancelled::new())))?;
        if !Arc::ptr_eq(entries._storage.semaphore(), bank) {
            return Err(super::owned_buffer::rejected(
                super::owned_buffer::BufferAdmission::Exhausted,
            ));
        }
        drop(state);
        let lease = self.reserve_flows()?;
        Ok(PrivateFilter::new(
            self.clone(),
            lease,
            client,
            bank.clone(),
        ))
    }

    pub(super) fn prepare(
        proxy_port: u16,
        capacity: usize,
        reserve_for_kernel: bool,
        bank: &Arc<Semaphore>,
    ) -> Result<Self, TunnelError> {
        let (storage, mut values) = crate::device::prepared_rsd::exact_vec(bank, capacity)
            .map_err(|source| TunnelError::Io(io::Error::other(source)))?;
        values.resize_with(capacity, || None);
        Ok(Self(RuntimeRoot::new(
            Registry {
                state: Mutex::new(State {
                    public: false,
                    closed: false,
                    entries: Some(Entries {
                        values,
                        _storage: storage,
                    }),
                    retiring: false,
                    leases: 0,
                }),
                proxy_port,
                reserve_for_kernel,
            },
            Some(bank),
        )?))
    }

    pub(crate) async fn connect(&self, endpoint: SocketAddrV6) -> io::Result<PrivateStream> {
        if endpoint.port() == 0 || endpoint.scope_id() != 0 || endpoint.flowinfo() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid private RSD endpoint",
            ));
        }
        let loopback = exclusive_socket(Domain::IPV4)?;
        loopback.set_nonblocking(true)?;
        loopback.bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())?;
        let peer = loopback
            .local_addr()?
            .as_socket()
            .expect("bound IPv4 socket");
        let kernel_port = if self.0.reserve_for_kernel {
            let socket = exclusive_socket(Domain::IPV6)?;
            socket.set_only_v6(true)?;
            socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)).into())?;
            Some(socket)
        } else {
            None
        };
        let source_port = match &kernel_port {
            Some(socket) => socket
                .local_addr()?
                .as_socket()
                .expect("bound IPv6 socket")
                .port(),
            None => peer.port(),
        };
        let duplicate = loopback.try_clone()?;
        let index = {
            let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.closed {
                return Err(io::Error::other(crate::OperationCancelled::new()));
            }
            let entries = state.entries.as_mut().expect("open private registry");
            let index = entries
                .values
                .iter()
                .position(Option::is_none)
                .ok_or_else(|| io::Error::other(super::owned_buffer::BufferAdmission::Exhausted))?;
            entries.values[index] = Some(Flow {
                peer,
                endpoint,
                source_port,
                claimed: false,
                loopback: duplicate,
                _kernel_port: kernel_port,
            });
            index
        };
        // Registration precedes the first connect poll. Even a cancelled or
        // failed dial retains the actual reserved descriptors until retirement.
        let mut owner = FlowOwner {
            registry: self.clone(),
            index,
            transferred: false,
        };
        let stream = TcpSocket::from_std_stream(loopback.into())
            .connect(SocketAddr::from((Ipv4Addr::LOCALHOST, self.0.proxy_port)))
            .await?;
        owner.transferred = true;
        Ok(PrivateStream {
            stream,
            registry: self.clone(),
            index,
        })
    }

    pub(super) fn claim(&self, peer: SocketAddr) -> Option<Access> {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.closed {
            return None;
        }
        if let Some(flow) = state
            .entries
            .as_mut()?
            .values
            .iter_mut()
            .flatten()
            .find(|flow| flow.peer == peer)
        {
            if flow.claimed {
                return None;
            }
            flow.claimed = true;
            return Some(Access::Private {
                endpoint: flow.endpoint,
                source_port: flow.source_port,
            });
        }
        state.public.then_some(Access::Public)
    }

    pub(super) fn open_public(&self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .public = true;
    }

    pub(super) fn close(&self) {
        let entries = {
            let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
            state.closed = true;
            state.retiring = true;
            if state.leases == 0 {
                state.entries.take()
            } else {
                None
            }
        };
        // Socket and backing destruction happen outside the registry guard;
        // the original whole grant follows the actual descriptor array.
        drop(entries);
    }

    /// Actual native/filter capability. Retirement stops new admission but
    /// keeps the original bound descriptors until every such owner ends.
    pub(super) fn reserve_flows(&self) -> Result<FlowLease, TunnelError> {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.retiring || state.entries.is_none() {
            return Err(io::Error::other(crate::OperationCancelled::new()).into());
        }
        state.leases = state.leases.checked_add(1).ok_or_else(|| {
            super::owned_buffer::rejected(super::owned_buffer::BufferAdmission::SizeOverflow)
        })?;
        Ok(FlowLease {
            connector: self.clone(),
        })
    }

    pub(super) fn seal(&self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .closed = true;
    }

    fn end_flow(&self, index: usize) {
        let state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(flow) = state
            .entries
            .as_ref()
            .and_then(|entries| entries.values[index].as_ref())
        {
            let _ = flow.loopback.shutdown(Shutdown::Both);
        }
    }
}

pub(super) struct FlowLease {
    connector: PrivateConnector,
}
impl Drop for FlowLease {
    fn drop(&mut self) {
        let detached = {
            let mut state = self
                .connector
                .0
                .state
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            state.leases = state
                .leases
                .checked_sub(1)
                .expect("actual private flow lease");
            if state.leases == 0 && state.retiring {
                state.entries.take()
            } else {
                None
            }
        };
        drop(detached);
    }
}

fn exclusive_socket(domain: Domain) -> io::Result<Socket> {
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{
            setsockopt, WSAGetLastError, SOCKET_ERROR, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
        };
        let enabled: i32 = 1;
        // SAFETY: the live socket is borrowed for this synchronous call; the
        // option points to an initialized, correctly sized BOOL-sized integer.
        let status = unsafe {
            setsockopt(
                socket.as_raw_socket() as _,
                SOL_SOCKET,
                SO_EXCLUSIVEADDRUSE,
                (&enabled as *const i32).cast(),
                std::mem::size_of_val(&enabled) as i32,
            )
        };
        if status == SOCKET_ERROR {
            // SAFETY: WSAGetLastError has no pointer/lifetime requirements.
            return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
        }
    }
    Ok(socket)
}

struct FlowOwner {
    registry: PrivateConnector,
    index: usize,
    transferred: bool,
}
impl Drop for FlowOwner {
    fn drop(&mut self) {
        if !self.transferred {
            self.registry.end_flow(self.index);
        }
    }
}

pub(crate) struct PrivateStream {
    stream: TcpStream,
    registry: PrivateConnector,
    index: usize,
}
impl Drop for PrivateStream {
    fn drop(&mut self) {
        // On Windows ConnectEx updates the connecting descriptor's context.
        // Request shutdown through that original connected descriptor before
        // dropping it; a pre-connect duplicate alone may report NotConnected.
        let _ = socket2::SockRef::from(&self.stream).shutdown(Shutdown::Both);
        self.registry.end_flow(self.index);
    }
}
impl AsyncRead for PrivateStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}
impl AsyncWrite for PrivateStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
