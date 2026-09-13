//! Unix-socket binding and connecting.
//!
//! One socket per session (the design's transport #2: `wcode serve` owns the
//! session, a client connects). A stale socket file left by a crashed server is
//! removed before binding, so a restart is not blocked by its predecessor.

use std::io;
use std::path::Path;

use tokio::net::{UnixListener, UnixStream};

/// Bind `path`, creating its parent directory and clearing a stale socket.
pub async fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    // A crashed server leaves the socket file behind; `UnixListener::bind`
    // would then fail with `AddrInUse`. Remove it first.
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    UnixListener::bind(path)
}

/// Connect to a server listening at `path`.
pub async fn connect(path: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(path).await
}
