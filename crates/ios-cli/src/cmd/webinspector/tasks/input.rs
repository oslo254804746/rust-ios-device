//! Directly owned Linux stdin readiness; no detached blocking read task.
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub(super) use linux::Input;

// Other platforms retain Tokio's existing input path. Cancellation of its
// blocking read and runtime shutdown need platform-specific acceptance.
#[cfg(not(target_os = "linux"))]
pub(super) struct Input(tokio::io::Stdin);

#[cfg(not(target_os = "linux"))]
impl Input {
    pub(super) fn open() -> std::io::Result<Self> {
        Ok(Self(tokio::io::stdin()))
    }

    pub(super) fn finish(self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
impl tokio::io::AsyncRead for Input {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
    }
}
