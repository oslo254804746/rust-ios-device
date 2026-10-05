use std::{
    fs::File,
    io::{self, Read},
    os::{
        fd::{AsFd, AsRawFd, RawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{unix::AsyncFd, AsyncRead, ReadBuf};

pub(crate) enum Input {
    Poll(AsyncFd<Flags>),
    Regular(File),
    Empty(File),
}

pub(crate) struct Flags {
    file: File,
    original: Option<libc::c_int>,
}

fn get_flags(fd: RawFd) -> io::Result<libc::c_int> {
    // SAFETY: callers own this open descriptor; F_GETFL has no pointer argument.
    let result = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn set_flags(fd: RawFd, flags: libc::c_int) -> io::Result<()> {
    // SAFETY: callers own this open descriptor and pass the saved status flags.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Flags {
    fn restore(&mut self) -> io::Result<()> {
        if let Some(flags) = self.original {
            set_flags(self.file.as_raw_fd(), flags)?;
            self.original = None;
        }
        Ok(())
    }
}

impl AsRawFd for Flags {
    fn as_raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }
}

impl Drop for Flags {
    fn drop(&mut self) {
        // Explicit finish reports failure. This fallback also covers admission
        // failure, caller cancellation and a dropped owner; it never spawns work.
        let _ = self.restore();
    }
}

impl Input {
    pub(crate) fn open() -> io::Result<Self> {
        Self::from_file(File::from(std::io::stdin().as_fd().try_clone_to_owned()?))
    }

    fn from_file(file: File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if metadata.is_file() {
            // Regular-file reads are synchronous/cooperative. No assertion of
            // hard preemption or a remote filesystem latency bound is made.
            return Ok(Self::Regular(file));
        }
        if metadata.file_type().is_char_device() && metadata.rdev() == libc::makedev(1, 3) {
            return Ok(Self::Empty(file));
        }
        let original = get_flags(file.as_raw_fd())?;
        let mut flags = Flags {
            file,
            original: None,
        };
        if original & libc::O_NONBLOCK == 0 {
            set_flags(flags.as_raw_fd(), original | libc::O_NONBLOCK)?;
            // dup shares status flags with the original stdin description. Hold
            // the descriptor through restoration, including AsyncFd::new failure.
            flags.original = Some(original);
        }
        Ok(Self::Poll(AsyncFd::new(flags)?))
    }

    pub(crate) fn finish(self) -> io::Result<()> {
        match self {
            Self::Poll(fd) => fd.into_inner().restore(),
            Self::Regular(file) | Self::Empty(file) => {
                drop(file);
                Ok(())
            }
        }
    }
}

impl AsyncRead for Input {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        match &mut *self {
            Self::Empty(_) => Poll::Ready(Ok(())),
            Self::Regular(file) => match file.read(buf.initialize_unfilled()) {
                Ok(count) => {
                    buf.advance(count);
                    Poll::Ready(Ok(()))
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Err(error) => Poll::Ready(Err(error)),
            },
            Self::Poll(fd) => loop {
                let mut ready = std::task::ready!(fd.poll_read_ready(cx))?;
                match ready.try_io(|inner| (&inner.get_ref().file).read(buf.initialize_unfilled()))
                {
                    Ok(Ok(count)) => {
                        buf.advance(count);
                        return Poll::Ready(Ok(()));
                    }
                    Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted => {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    Ok(Err(error)) => return Poll::Ready(Err(error)),
                    Err(_) => continue,
                }
            },
        }
    }
}

#[cfg(test)]
mod tests;
