//! Single-instance socket: fast `crabmd <file:line:col>` via forward-to-daemon.
//!
//! Zed-style: one process (one dock icon), N windows. The first CLI call
//! becomes the daemon (detached child, listens on the socket); later calls
//! forward a small JSON request and exit in ~ms. Default behavior opens a
//! tab in the existing window; `-n` opens a new window in the same process.
//!
//! Unix-only. Other platforms fall back to one-process-per-window.

use std::path::PathBuf;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpenRequest {
    pub path: String,
    pub line: Option<usize>,
    pub col: Option<usize>,
    /// "new" | "existing" | "add" | "reuse"
    pub behavior: String,
}

#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join(".local/share/crabmd");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("crabmd.sock")
}

#[cfg(not(unix))]
pub fn socket_path() -> PathBuf {
    PathBuf::from("/tmp/crabmd.sock")
}

/// Best-effort socket removal for `cmd-q` / last-window-quit.
pub fn cleanup() {
    #[cfg(unix)]
    let _ = std::fs::remove_file(socket_path());
}

/// Forward to a live daemon. Returns true if the daemon ACKed.
#[cfg(unix)]
pub fn try_forward(
    path: &str,
    line: Option<usize>,
    col: Option<usize>,
    behavior: &str,
) -> bool {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let Ok(stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let req = OpenRequest {
        path: path.to_string(),
        line,
        col,
        behavior: behavior.to_string(),
    };
    let Ok(mut bytes) = serde_json::to_vec(&req) else {
        return false;
    };
    bytes.push(b'\n');
    let mut stream = stream;
    if stream.write_all(&bytes).is_err() {
        return false;
    }
    let mut ack = String::new();
    match BufReader::new(&stream).read_line(&mut ack) {
        Ok(_) => ack.trim() == "ok",
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub fn try_forward(
    _path: &str,
    _line: Option<usize>,
    _col: Option<usize>,
    _behavior: &str,
) -> bool {
    false
}

/// Bind as the daemon and spawn the accept loop. Returns the channel the
/// GPUI foreground task polls, or `None` if a live daemon already holds
/// the socket (caller should forward instead).
#[cfg(unix)]
pub fn start_listener() -> Option<std::sync::mpsc::Receiver<OpenRequest>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};

    match UnixListener::bind(socket_path()) {
        Ok(listener) => Some(spawn_accept(listener)),
        Err(_) => {
            // Bound by someone else: live daemon or stale file?
            if UnixStream::connect(socket_path()).is_ok() {
                return None;
            }
            let _ = std::fs::remove_file(socket_path());
            match UnixListener::bind(socket_path()) {
                Ok(listener) => Some(spawn_accept(listener)),
                Err(_) => None,
            }
        }
    }
}

#[cfg(unix)]
fn spawn_accept(
    listener: std::os::unix::net::UnixListener,
) -> std::sync::mpsc::Receiver<OpenRequest> {
    use std::io::{BufRead, BufReader, Write as _};

    let (tx, rx) = std::sync::mpsc::channel::<OpenRequest>();
    std::thread::Builder::new()
        .name("crabmd-ipc".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                let req = match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => None,
                    Ok(_) => serde_json::from_str::<OpenRequest>(&line).ok(),
                };
                if let Some(req) = req {
                    let _ = tx.send(req);
                    let _ = (&stream).write_all(b"ok\n");
                }
            }
        })
        .ok();
    rx
}

#[cfg(not(unix))]
pub fn start_listener() -> Option<std::sync::mpsc::Receiver<OpenRequest>> {
    None
}
