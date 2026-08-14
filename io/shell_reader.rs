//! ShellReader spawns a ffmpeg subprocess, exposes its stdout as an AsyncRead,
//! captures the tail of stderr in a fixed-size ring, and cleans up when
//! the context is canceled or the process exits.

use crate::models::errors::RustTgCallsError;
use crate::utils::RingBuffer;
use std::io::Result as IoResult;
use std::io::Write;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::{Mutex, Notify};

/// readEOFDrainTimeout bounds the wait Read does on EOF for the reap task
/// to capture the child's exit status. Picked to be long enough that a child
/// that just closed its stdout always has time to be reaped on a non-loaded
/// box (microseconds in practice), and short enough that a pathological reap
/// hang doesn't strand a Read forever.
pub const READ_EOF_DRAIN_TIMEOUT: Duration = Duration::from_millis(200);

pub type OnExitFn = Arc<dyn Fn(Option<RustTgCallsError>) + Send + Sync>;

/// ShellReader spawns a ffmpeg subprocess, exposes its stdout as an AsyncRead,
/// captures the tail of stderr in a fixed-size ring, and cleans up when
/// the process exits.
///
/// ShellReader is safe to use across tasks for Close and Err. Read
/// must be serialized by a single consumer (the convention for AsyncRead).
pub struct ShellReader {
    pid: Option<u32>,
    child: Arc<Mutex<Option<Child>>>,
    stdout: ChildStdout,
    stderr_ring: Arc<Mutex<RingBuffer>>,
    on_exit: Arc<Mutex<Option<OnExitFn>>>,
    closed: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    exit_error: Arc<Mutex<Option<RustTgCallsError>>>,
    done_notify: Arc<Notify>,
}

impl ShellReader {
    /// pid returns the OS process ID of the spawned child process.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// NewShellReader spawns program with args and starts the process.
    pub fn new(
        program: &str,
        args: &[String],
        stream_stderr: bool,
    ) -> Result<Self, RustTgCallsError> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.stdout(Stdio::piped());

        if stream_stderr {
            cmd.stderr(Stdio::piped());
        } else {
            cmd.stderr(Stdio::null());
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| RustTgCallsError::FFmpegSpawn(e.to_string()))?;
        let pid = child.id();
        let stdout = child.stdout.take().ok_or_else(|| {
            RustTgCallsError::FFmpegSpawn("failed to capture ffmpeg stdout".to_string())
        })?;

        let stderr_ring = Arc::new(Mutex::new(RingBuffer::new(4096)));
        let on_exit: Arc<Mutex<Option<OnExitFn>>> = Arc::new(Mutex::new(None));
        let closed = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_error: Arc<Mutex<Option<RustTgCallsError>>> = Arc::new(Mutex::new(None));
        let done_notify = Arc::new(Notify::new());

        if stream_stderr && let Some(mut stderr) = child.stderr.take() {
            let ring_clone = stderr_ring.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let chunk = &buf[..n];
                    let mut guard = ring_clone.lock().await;
                    let _ = guard.write(chunk);
                }
            });
        }

        let child_arc = Arc::new(Mutex::new(Some(child)));
        let child_clone = child_arc.clone();
        let exit_err_clone = exit_error.clone();
        let on_exit_clone = on_exit.clone();
        let done_notify_clone = done_notify.clone();
        let exited_clone = exited.clone();

        tokio::spawn(async move {
            let res = {
                let mut guard = child_clone.lock().await;
                if let Some(child) = guard.as_mut() {
                    child.wait().await
                } else {
                    return;
                }
            };

            let err = match res {
                Ok(status) if status.success() => None,
                Ok(status) => Some(RustTgCallsError::FFmpegCrashed(format!(
                    "ffmpeg exited with code {}",
                    status
                ))),
                Err(e) => Some(RustTgCallsError::FFmpegCrashed(e.to_string())),
            };

            *exit_err_clone.lock().await = err.clone();
            let cb = on_exit_clone.lock().await.clone();
            if let Some(cb) = cb {
                cb(err);
            }
            exited_clone.store(true, Ordering::SeqCst);
            done_notify_clone.notify_waiters();
        });

        Ok(Self {
            pid,
            child: child_arc,
            stdout,
            stderr_ring,
            on_exit,
            closed,
            exited,
            exit_error,
            done_notify,
        })
    }

    /// SetOnExit registers a callback invoked once the subprocess exits and the
    /// reap task has captured its exit error.
    pub async fn set_on_exit(&self, fn_exit: OnExitFn) {
        *self.on_exit.lock().await = Some(fn_exit);
    }

    /// Close terminates the ffmpeg subprocess.
    pub async fn close(&self) -> Result<(), RustTgCallsError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.kill().await;
        }
        self.exited.store(true, Ordering::SeqCst);
        self.done_notify.notify_waiters();
        Ok(())
    }

    /// StderrTail returns a snapshot of stderr bytes recorded in the RingBuffer.
    pub async fn stderr_tail(&self) -> Vec<u8> {
        self.stderr_ring.lock().await.snapshot()
    }

    /// ExitError returns the subprocess exit error if the process has finished.
    pub async fn exit_error(&self) -> Option<RustTgCallsError> {
        self.exit_error.lock().await.clone()
    }

    /// WaitForExit waits until the ffmpeg subprocess has completed and been reaped.
    pub async fn wait_for_exit(&self) {
        if self.exited.load(Ordering::SeqCst) {
            return;
        }
        self.done_notify.notified().await;
    }
}

impl AsyncRead for ShellReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}
