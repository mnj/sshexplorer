//! Progress and cancellation for the socket while a session opens.
use super::*;
use std::io;
use std::pin::Pin;
use std::sync::Weak;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

pub(super) struct Progress {
    pub stage: &'static str,
    last: Instant,
    waiting: bool,
    pub(super) socket: Option<std::net::TcpStream>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            stage: "Connecting",
            last: Instant::now(),
            waiting: false,
            socket: None,
        }
    }
}

impl Progress {
    pub(super) fn close(&mut self) {
        if let Some(socket) = self.socket.take() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Events {
    pub(super) fn stage(&self, stage: &'static str) {
        let mut progress = self.progress.lock().unwrap();
        progress.stage = stage;
        progress.last = Instant::now();
    }

    pub(super) async fn answer<T>(
        &self,
        reply: oneshot::Receiver<T>,
    ) -> Result<T, oneshot::error::RecvError> {
        self.progress.lock().unwrap().waiting = true;
        let answer = reply.await;
        let mut progress = self.progress.lock().unwrap();
        progress.waiting = false;
        progress.last = Instant::now();
        answer
    }

    pub(super) async fn timeout(&self, limit: Duration) -> Failure {
        loop {
            let remaining = {
                let progress = self.progress.lock().unwrap();
                if progress.waiting {
                    limit
                } else {
                    limit.saturating_sub(progress.last.elapsed())
                }
            };
            if remaining.is_zero() {
                return Failure::Connection("timed out".into());
            }
            tokio::time::sleep(remaining).await;
        }
    }

    pub(super) fn opening_stream(&self, stream: tokio::net::TcpStream) -> io::Result<Stream> {
        let socket = stream.into_std()?;
        self.progress.lock().unwrap().socket = Some(socket.try_clone()?);
        Ok(Stream {
            socket: tokio::net::TcpStream::from_std(socket)?,
            progress: Arc::downgrade(&self.progress),
        })
    }
}

pub(super) struct Stream {
    socket: tokio::net::TcpStream,
    progress: Weak<Mutex<Progress>>,
}

impl Stream {
    fn received(&mut self) {
        if let Some(progress) = self.progress.upgrade() {
            let mut state = progress.lock().unwrap();
            if state.socket.is_some() {
                state.last = Instant::now();
            } else {
                self.progress = Weak::new();
            }
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.socket).poll_read(cx, buf);
        if buf.filled().len() > before {
            self.received();
        }
        result
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}
