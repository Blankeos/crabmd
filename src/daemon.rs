//! Single-instance socket: fast `crabmd <file:line:col>` via forward-to-daemon.
//!
//! Zed-style: one process (one dock icon), N windows. Later calls forward
//! JSON and exit; `-n` opens a new window in the same process.
//!
//! Unix-only. Only the `start_listener` owner may call `cleanup`.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct OpenRequest {
    pub path: String,
    pub line: Option<usize>,
    pub col: Option<usize>,
    /// "new" | "existing" | "add" | "reuse"
    pub behavior: String,
}

impl OpenRequest {
    pub(crate) fn new(path: &str, line: Option<usize>, col: Option<usize>, behavior: &str) -> Self {
        Self {
            path: path.to_string(),
            line,
            col,
            behavior: behavior.to_string(),
        }
    }
}

#[cfg(unix)]
pub(crate) fn socket_path_for_home(home: &str) -> PathBuf {
    PathBuf::from(home).join(".local/share/crabmd/crabmd.sock")
}

#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let path = socket_path_for_home(&home);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    path
}

#[cfg(not(unix))]
pub fn socket_path() -> PathBuf {
    PathBuf::from("/tmp/crabmd.sock")
}

/// Remove the socket. Call only when this process owns it.
pub fn cleanup() {
    #[cfg(unix)]
    cleanup_at(&socket_path());
}

#[cfg(unix)]
pub(crate) fn cleanup_at(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Cold-start election file next to the socket. The single winner launches
/// via `open --args`; losers wait for the socket and forward. Prevents a
/// second `open --args` from being ignored while the app is launching.
#[cfg(unix)]
pub(crate) fn coldstart_lock_path() -> PathBuf {
    socket_path().parent().map(|d| d.join("coldstart.lock")).unwrap_or_else(|| PathBuf::from("/tmp/crabmd-coldstart.lock"))
}

#[cfg(unix)]
pub(crate) fn clear_coldstart_lock() {
    cleanup_at(&coldstart_lock_path());
}

/// True when the lock is older than `max_age` (crashed starter).
#[cfg(unix)]
pub(crate) fn coldstart_lock_is_stale(max_age: std::time::Duration) -> bool {
    let path = coldstart_lock_path();
    std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|d| d > max_age)
}

/// Claim the cold-start election. False when another CLI already holds it.
/// A stale lock (>30s, crashed starter) is reclaimed once.
#[cfg(unix)]
pub(crate) fn try_acquire_coldstart_lock() -> bool {
    use std::fs::OpenOptions;
    use std::io::Write as _;
    let path = coldstart_lock_path();
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            let _ = writeln!(f, "{} {}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
            true
        }
        Err(_) => {
            if coldstart_lock_is_stale(std::time::Duration::from_secs(30)) {
                let _ = std::fs::remove_file(&path);
                return OpenOptions::new().write(true).create_new(true).open(&path).map(|_| true).unwrap_or(false);
            }
            false
        }
    }
}

/// Poll `try_forward_to` until the launching daemon answers or timeout.
/// Used when the app is running but its socket is not ready yet; `open
/// --args` would be ignored there, so waiting preserves the request.
#[cfg(unix)]
pub(crate) fn wait_and_forward_to(
    socket: &Path,
    path: &str,
    line: Option<usize>,
    col: Option<usize>,
    behavior: &str,
    timeout: std::time::Duration,
) -> bool {
    let start = std::time::Instant::now();
    loop {
        if try_forward_to(socket, path, line, col, behavior) {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(unix)]
pub fn wait_and_forward(
    path: &str,
    line: Option<usize>,
    col: Option<usize>,
    behavior: &str,
    timeout: std::time::Duration,
) -> bool {
    wait_and_forward_to(&socket_path(), path, line, col, behavior, timeout)
}

/// Forward to a live daemon. Returns true if the daemon ACKed.
#[cfg(unix)]
pub fn try_forward(path: &str, line: Option<usize>, col: Option<usize>, behavior: &str) -> bool {
    try_forward_to(&socket_path(), path, line, col, behavior)
}

#[cfg(unix)]
pub(crate) fn try_forward_to(
    socket: &Path,
    path: &str,
    line: Option<usize>,
    col: Option<usize>,
    behavior: &str,
) -> bool {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let Ok(stream) = UnixStream::connect(socket) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let req = OpenRequest::new(path, line, col, behavior);
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

#[cfg(not(unix))]
pub fn wait_and_forward(
    _path: &str,
    _line: Option<usize>,
    _col: Option<usize>,
    _behavior: &str,
    _timeout: std::time::Duration,
) -> bool {
    false
}

/// Bind as the daemon and spawn the accept loop. Returns the channel the
/// GPUI foreground task polls, or `None` if a live daemon already holds
/// the socket (caller should forward instead).
#[cfg(unix)]
pub fn start_listener() -> Option<std::sync::mpsc::Receiver<OpenRequest>> {
    start_listener_at(&socket_path())
}

/// Stale-file takeover: `bind` fails + `connect` fails means the owner is
/// gone; remove and retry once. `connect` success means a live owner holds
/// it, so return `None` untouched.
#[cfg(unix)]
pub(crate) fn start_listener_at(socket: &Path) -> Option<std::sync::mpsc::Receiver<OpenRequest>> {
    use std::os::unix::net::{UnixListener, UnixStream};

    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match UnixListener::bind(socket) {
        Ok(listener) => {
            // Socket is live; cold-start election is over.
            if socket == &socket_path() {
                clear_coldstart_lock();
            }
            Some(spawn_accept(listener))
        }
        Err(_) => {
            // Bound by someone else: live daemon or stale file?
            if UnixStream::connect(socket).is_ok() {
                return None;
            }
            let _ = std::fs::remove_file(socket);
            match UnixListener::bind(socket) {
                Ok(listener) => {
                    if socket == &socket_path() {
                        clear_coldstart_lock();
                    }
                    Some(spawn_accept(listener))
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_request_roundtrip() {
        let req = OpenRequest::new("/tmp/notes.md", Some(10), Some(3), "existing");
        let json = serde_json::to_string(&req).unwrap();
        let back: OpenRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
        assert_eq!(back.path, "/tmp/notes.md");
        assert_eq!(back.line, Some(10));
        assert_eq!(back.col, Some(3));
        assert_eq!(back.behavior, "existing");
    }

    #[test]
    fn open_request_empty_path_for_untitled() {
        let req = OpenRequest::new("", None, None, "new");
        assert_eq!(req.path, "");
        let json = serde_json::to_string(&req).unwrap();
        let back: OpenRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[cfg(unix)]
    #[test]
    fn socket_path_joins_home() {
        assert_eq!(
            socket_path_for_home("/Users/me"),
            PathBuf::from("/Users/me/.local/share/crabmd/crabmd.sock")
        );
        assert_eq!(
            socket_path_for_home("/tmp"),
            PathBuf::from("/tmp/.local/share/crabmd/crabmd.sock")
        );
    }

    #[cfg(unix)]
    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crabmd-ipc-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("crabmd.sock")
    }

    #[cfg(unix)]
    #[test]
    fn forward_fails_with_no_daemon() {
        let sock = temp_socket("nodaemon");
        let _ = std::fs::remove_file(&sock);
        assert!(!try_forward_to(&sock, "/tmp/a.md", None, None, "existing"));
    }

    #[cfg(unix)]
    #[test]
    fn forward_reaches_live_daemon() {
        let sock = temp_socket("live");
        let _ = std::fs::remove_file(&sock);
        let rx = start_listener_at(&sock).expect("bind temp socket");
        assert!(try_forward_to(&sock, "/tmp/notes.md", Some(4), None, "new"));
        let req = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("daemon receives request");
        assert_eq!(req.path, "/tmp/notes.md");
        assert_eq!(req.line, Some(4));
        assert_eq!(req.behavior, "new");
        cleanup_at(&sock);
    }

    #[cfg(unix)]
    #[test]
    fn second_listener_defers_to_live_owner() {
        let sock = temp_socket("twoowners");
        let _ = std::fs::remove_file(&sock);
        let _first = start_listener_at(&sock).expect("first binds");
        // Live owner answers connects, so the second must not steal it.
        assert!(start_listener_at(&sock).is_none());
        assert!(try_forward_to(&sock, "", None, None, "existing"));
        cleanup_at(&sock);
    }

    #[cfg(unix)]
    #[test]
    fn stale_regular_file_is_reclaimed() {
        let sock = temp_socket("stale");
        let _ = std::fs::remove_file(&sock);
        std::fs::write(&sock, b"stale").unwrap();
        let rx = start_listener_at(&sock).expect("stale file reclaimed");
        assert!(try_forward_to(&sock, "/tmp/b.md", None, None, "existing"));
        let req = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("reclaimed listener receives");
        assert_eq!(req.path, "/tmp/b.md");
        cleanup_at(&sock);
    }

    #[cfg(unix)]
    #[test]
    fn wait_forward_times_out_without_daemon() {
        let sock = temp_socket("wait-timeout");
        let _ = std::fs::remove_file(&sock);
        assert!(!wait_and_forward_to(
            &sock,
            "/tmp/a.md",
            None,
            None,
            "existing",
            std::time::Duration::from_millis(250)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn wait_forward_reaches_delayed_daemon() {
        let sock = temp_socket("wait-delayed");
        let _ = std::fs::remove_file(&sock);
        let sock_clone = sock.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let rx = start_listener_at(&sock_clone).expect("delayed bind");
            let req = rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("receives");
            assert_eq!(req.path, "/tmp/delayed.md");
            cleanup_at(&sock_clone);
        });
        assert!(wait_and_forward_to(
            &sock,
            "/tmp/delayed.md",
            None,
            None,
            "existing",
            std::time::Duration::from_secs(5)
        ));
        let _ = std::fs::remove_file(&sock);
    }
}
