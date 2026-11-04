//! Socket client for the Herdr host API.
//!
//! `herdr <command>` is a one-shot CLI over the same socket, so polling through
//! it costs a fork + exec of a ~30 MB static binary every time. For the 500 ms
//! shell-directory poll that is pure waste: a direct round trip is one small
//! write and one small read with no process involved.
//!
//! Wire format is newline-delimited JSON:
//!
//! ```text
//! --> {"id":"corral:1","method":"pane.list","params":{}}
//! <-- {"id":"corral:1","result":{…}}
//! ```
//!
//! The host serves exactly one request per connection and closes the socket
//! afterwards, so there is nothing to keep open: each call dials, writes, reads
//! and drops. Every failure surfaces as `Err` and callers fall back to the CLI,
//! so a protocol change can only cost performance, never correctness. Timeouts
//! keep a wedged host from stalling the TUI.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A wedged host must never stall the caller for long.
const RPC_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// Sequentially issued request ids, so responses are easy to correlate in logs.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Client for the Herdr socket API. Cheap to keep around; holds no connection.
pub struct HerdrRpc {
    path: PathBuf,
}

impl HerdrRpc {
    /// Client targeting `path`.
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Path this client dials (useful for diagnostics).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Call `method` with empty params and return the response line verbatim.
    ///
    /// # Errors
    ///
    /// Returns a message when the socket cannot be reached, times out, or
    /// closes before a full line arrives.
    pub fn call(&self, method: &str) -> Result<String, String> {
        #[cfg(not(unix))]
        {
            let _ = method;
            Err("herdr socket API requires a unix host".to_string())
        }

        #[cfg(unix)]
        {
            use std::io::{BufRead as _, Write as _};

            let stream = std::os::unix::net::UnixStream::connect(&self.path)
                .map_err(|error| format!("herdr socket: {error}"))?;
            // Best effort: a host that ignores timeouts is not fatal.
            let _ = stream.set_read_timeout(Some(RPC_TIMEOUT));
            let _ = stream.set_write_timeout(Some(RPC_TIMEOUT));
            let mut reader = std::io::BufReader::new(stream);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let request =
                format!("{{\"id\":\"corral:{id}\",\"method\":\"{method}\",\"params\":{{}}}}\n");
            let stream = reader.get_mut();
            stream
                .write_all(request.as_bytes())
                .and_then(|()| stream.flush())
                .map_err(|error| format!("herdr socket write: {error}"))?;
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .map_err(|error| format!("herdr socket read: {error}"))?;
            if line.trim().is_empty() {
                return Err("herdr socket closed".to_string());
            }
            Ok(line)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn call_returns_one_response_and_survives_the_one_shot_host() {
        use std::io::{BufRead as _, Write as _};
        use std::os::unix::net::UnixListener;

        let path = std::env::temp_dir().join(format!(
            "corral-rpc-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        // The real host serves one request per connection and then closes.
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let id = line
                    .split("\"id\":\"")
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .unwrap()
                    .to_string();
                let body = format!("{{\"id\":\"{id}\",\"result\":{{\"type\":\"pane_list\"}}}}\n");
                reader.get_mut().write_all(body.as_bytes()).unwrap();
                requests.push(line);
            }
            requests
        });

        let rpc = HerdrRpc::new(path.clone());
        assert_eq!(rpc.path(), path.as_path());
        for _ in 0..2 {
            let response = rpc.call("pane.list").unwrap();
            assert!(response.contains("\"type\":\"pane_list\""));
        }

        let requests = server.join().unwrap();
        assert!(requests[0].contains("\"method\":\"pane.list\""));
        // A fresh connection per call still carries a fresh id.
        assert_ne!(
            requests[0].trim_end().to_string(),
            requests[1].trim_end().to_string()
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_socket_is_an_error_and_leaves_the_client_reusable() {
        let path = std::env::temp_dir().join(format!(
            "corral-rpc-missing-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let rpc = HerdrRpc::new(path.clone());
        assert_eq!(rpc.path(), path.as_path());
        assert!(rpc.call("pane.list").is_err());
        assert!(rpc.call("pane.list").is_err());
    }
}
