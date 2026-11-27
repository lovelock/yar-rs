//! Yar RPC client — wire-compatible with the C yar_client implementation.
//!
//! Supports TCP (`host:port`) and Unix domain socket (`/path/to/sock`)
//! connections using synchronous I/O, matching the C client's blocking API.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::error::{YarError, YarResult};
use crate::logger;
use crate::pack::Packager;
use crate::protocol::{YarHeader, HEADER_SIZE, PROTOCOL_MAGIC, PROTOCOL_PERSISTENT};
use crate::request::YarRequest;
use crate::response::YarResponse;
use crate::{PACKAGER_NAME, PACKAGER_NAME_STR};

/// Client name sent in the header provider field.
pub const CLIENT_NAME: &str = concat!("Yar(Rust)-", env!("CARGO_PKG_VERSION"));

/// Default request ID (matching C client's dummy id).
const DEFAULT_REQUEST_ID: u64 = 1000;

/// Default read/write timeout in seconds (matching C client's 1 s default).
const DEFAULT_TIMEOUT_SECS: u64 = 1;

/// Default maximum response body size accepted by the client (10 MiB,
/// matching `YAR_MAX_BODY_SIZE` in the C implementation).
pub const DEFAULT_MAX_RESPONSE_SIZE: usize = 10 * 1024 * 1024;

/// Transport connection type.
enum Connection {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Connection {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Connection::Tcp(s) => s.set_read_timeout(timeout),
            #[cfg(unix)]
            Connection::Unix(s) => s.set_read_timeout(timeout),
        }
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        match self {
            Connection::Tcp(s) => s.set_write_timeout(timeout),
            #[cfg(unix)]
            Connection::Unix(s) => s.set_write_timeout(timeout),
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Connection::Tcp(s) => s.read(buf),
            #[cfg(unix)]
            Connection::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Connection::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Connection::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Connection::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Connection::Unix(s) => s.flush(),
        }
    }
}

/// Yar RPC client, wire-compatible with C `yar_client`.
pub struct YarClient {
    conn: Connection,
    hostname: String,
    persistent: bool,
    timeout: Duration,
    max_response_size: usize,
    /// Set after a failed call: the stream may still contain a late response,
    /// so it can no longer be trusted for another request.
    failed: bool,
}

impl std::fmt::Debug for YarClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YarClient")
            .field("hostname", &self.hostname)
            .field("persistent", &self.persistent)
            .field("timeout", &self.timeout)
            .field("max_response_size", &self.max_response_size)
            .field("failed", &self.failed)
            .finish()
    }
}

impl YarClient {
    /// Connect to a Yar server using the default connect timeout.
    ///
    /// Hostname formats:
    /// - `host:port` — TCP connection
    /// - `/path/to/socket` — Unix domain socket
    ///
    /// HTTP/HTTPS URLs are rejected since Yar uses a raw TCP protocol.
    pub fn connect(hostname: &str) -> YarResult<Self> {
        Self::connect_with_timeout(hostname, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
    }

    /// Connect to a Yar server with an explicit connect timeout and the
    /// default read/write timeout.
    ///
    /// The connect timeout bounds DNS-resolved TCP connection attempts. Unix
    /// domain socket connects are immediate and not affected.
    pub fn connect_with_timeout(hostname: &str, connect_timeout: Duration) -> YarResult<Self> {
        if hostname.starts_with("http://") || hostname.starts_with("https://") {
            return Err(YarError::Connection(
                "HTTP/HTTPS not supported; use host:port or /path for Unix socket".into(),
            ));
        }

        let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);

        let conn = if hostname.starts_with('/') {
            #[cfg(unix)]
            {
                // Unix domain socket
                logger::log_global(
                    logger::LogLevel::Debug,
                    &format!("connecting to unix socket: {}", hostname),
                );
                let stream = UnixStream::connect(hostname).map_err(|e| {
                    YarError::Connection(format!("failed to connect to {}: {}", hostname, e))
                })?;
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                Connection::Unix(stream)
            }
            #[cfg(not(unix))]
            {
                let _ = connect_timeout;
                return Err(YarError::Connection(
                    "Unix domain sockets are not supported on this platform".into(),
                ));
            }
        } else {
            // TCP — require host:port
            if !hostname.contains(':') {
                return Err(YarError::Connection(
                    "hostname must include port (host:port)".into(),
                ));
            }

            logger::log_global(
                logger::LogLevel::Debug,
                &format!("connecting to tcp: {}", hostname),
            );

            let addrs = hostname.to_socket_addrs().map_err(|e| {
                YarError::Connection(format!(
                    "failed to connect to {}: resolution failed: {}",
                    hostname, e
                ))
            })?;

            let mut last_err = None;
            let mut connected = None;
            for addr in addrs {
                match TcpStream::connect_timeout(&addr, connect_timeout) {
                    Ok(stream) => {
                        connected = Some(stream);
                        break;
                    }
                    Err(e) => last_err = Some(e),
                }
            }
            let stream = connected.ok_or_else(|| {
                YarError::Connection(format!(
                    "failed to connect to {}: {}",
                    hostname,
                    last_err
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "no addresses resolved".into())
                ))
            })?;

            stream.set_read_timeout(Some(timeout))?;
            stream.set_write_timeout(Some(timeout))?;
            stream.set_nodelay(true)?;
            Connection::Tcp(stream)
        };

        Ok(YarClient {
            conn,
            hostname: hostname.to_string(),
            persistent: false,
            timeout,
            max_response_size: DEFAULT_MAX_RESPONSE_SIZE,
            failed: false,
        })
    }

    /// Sets the maximum response body size accepted from the server.
    pub fn set_max_response_size(&mut self, bytes: usize) {
        self.max_response_size = bytes;
    }

    /// Set the read/write timeout for the connection.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
        let _ = self.conn.set_read_timeout(Some(timeout));
        let _ = self.conn.set_write_timeout(Some(timeout));
    }

    /// Enable or disable persistent connection mode.
    ///
    /// When persistent, the `PROTOCOL_PERSISTENT` flag is set in the header
    /// reserved field, signaling the server to keep the connection alive.
    pub fn set_persistent(&mut self, persistent: bool) {
        self.persistent = persistent;
    }

    /// Call a remote method on the server.
    ///
    /// This sends a request and reads the response, matching the C client's
    /// `yar_client_call` flow:
    ///
    /// Wire format sent:
    ///   [82-byte header][8-byte "MSGPACK\0"][msgpack request body]
    ///
    /// Wire format received:
    ///   [82-byte header][8-byte packager name][msgpack response body]
    pub fn call(&mut self, method: &str, params: Vec<Packager>) -> YarResult<YarResponse> {
        if self.failed {
            return Err(YarError::Connection(
                "client connection is unusable after a failed call; reconnect with YarClient::connect"
                    .into(),
            ));
        }

        let result = self.call_inner(method, params);
        if result.is_err() {
            // A read timeout or protocol error can leave a late response in the
            // stream. Reusing the socket would let the next call consume it.
            self.failed = true;
        }
        result
    }

    fn call_inner(&mut self, method: &str, params: Vec<Packager>) -> YarResult<YarResponse> {
        // Build the request
        let mut request = YarRequest::new(DEFAULT_REQUEST_ID, method);
        if !params.is_empty() {
            let mut arr = Packager::start_array(params.len() as u32)?;
            for p in params {
                arr.push_packager(&p)?;
            }
            request.set_parameters(arr);
        }

        // Pack the request with extra bytes for header + packager name
        let extra_bytes = HEADER_SIZE + PACKAGER_NAME.len();
        let mut payload = request.pack(extra_bytes)?;

        // Build and render the header
        let body_len = (payload.len() - HEADER_SIZE) as u32;
        let reserved = if self.persistent {
            PROTOCOL_PERSISTENT
        } else {
            0
        };

        let mut header = YarHeader::new();
        header.render(
            DEFAULT_REQUEST_ID as u32,
            Some(CLIENT_NAME),
            None,
            body_len,
            reserved,
        );

        // Copy header bytes into the payload prefix.
        let header_bytes = header.to_bytes();
        payload[..HEADER_SIZE].copy_from_slice(&header_bytes);

        // Copy packager name after header
        payload[HEADER_SIZE..HEADER_SIZE + PACKAGER_NAME.len()].copy_from_slice(PACKAGER_NAME);

        // Send the full payload
        send_all(&mut self.conn, &payload)?;

        logger::log_global(
            logger::LogLevel::Debug,
            &format!(
                "sent request: method={:?}, body_len={}, total={}",
                method,
                body_len,
                payload.len()
            ),
        );

        // Read the response header
        let mut resp_header_buf = [0u8; HEADER_SIZE];
        recv_exact(&mut self.conn, &mut resp_header_buf)?;

        // Parse and validate the response header
        let resp_header =
            YarHeader::from_bytes(&resp_header_buf).map_err(|_| YarError::InvalidMagic)?;

        if resp_header.magic_num != PROTOCOL_MAGIC {
            return Err(YarError::InvalidMagic);
        }

        let resp_body_len = resp_header.body_len as usize;

        // Never allocate from an untrusted length before validating it.
        if resp_body_len > self.max_response_size {
            return Err(YarError::Protocol(format!(
                "response body size {} exceeds limit {}",
                resp_body_len, self.max_response_size
            )));
        }
        if resp_body_len < PACKAGER_NAME.len() {
            return Err(YarError::Protocol(format!(
                "response body too short for packager name ({} bytes)",
                resp_body_len
            )));
        }

        // Read the full response body (packager name + msgpack)
        let mut resp_body = vec![0u8; resp_body_len];
        recv_exact(&mut self.conn, &mut resp_body)?;

        let resp_id = resp_header.id;
        logger::log_global(
            logger::LogLevel::Debug,
            &format!("recv response: id={}, body_len={}", resp_id, resp_body_len),
        );

        // Verify the packager before parsing its payload.
        if &resp_body[..PACKAGER_NAME.len()] != PACKAGER_NAME.as_slice() {
            return Err(YarError::UnsupportedPackager(format!(
                "response packager is not {}",
                PACKAGER_NAME_STR
            )));
        }

        // Unpack the response directly from the received body.
        let response = YarResponse::unpack(&resp_body, PACKAGER_NAME.len())?;

        if response.id != DEFAULT_REQUEST_ID {
            return Err(YarError::Protocol(format!(
                "response id {} does not match request id {}",
                response.id, DEFAULT_REQUEST_ID
            )));
        }

        Ok(response)
    }

    /// Close the connection (consumes the client).
    pub fn close(self) {
        // Dropping the connection closes it.
        drop(self);
    }
}

/// Write all bytes to a stream, handling partial writes and EINTR.
fn send_all(conn: &mut Connection, data: &[u8]) -> YarResult<()> {
    let mut sent = 0;
    while sent < data.len() {
        match conn.write(&data[sent..]) {
            Ok(0) => {
                return Err(YarError::Connection(
                    "connection closed during write".into(),
                ));
            }
            Ok(n) => sent += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // A blocking socket reports an expired SO_SNDTIMEO as WouldBlock;
            // retrying would spin forever.
            Err(ref e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(YarError::Timeout)
            }
            Err(e) => return Err(YarError::Io(e)),
        }
    }
    conn.flush()?;
    Ok(())
}

/// Read exactly `buf.len()` bytes from a stream, handling partial reads and EINTR.
fn recv_exact(conn: &mut Connection, buf: &mut [u8]) -> YarResult<()> {
    let mut received = 0;
    while received < buf.len() {
        match conn.read(&mut buf[received..]) {
            Ok(0) => {
                return Err(YarError::Connection(format!(
                    "connection closed: expected {} bytes, got {}",
                    buf.len(),
                    received
                )));
            }
            Ok(n) => received += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // A blocking socket reports an expired SO_RCVTIMEO as WouldBlock;
            // treating it as "retry" turned every timed-out call into an
            // infinite spin (the profiler then could not count timeouts).
            Err(ref e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(YarError::Timeout)
            }
            Err(e) => return Err(YarError::Io(e)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_connect_invalid_host() {
        let result = YarClient::connect("nonexistent.host.invalid:9999");
        assert!(result.is_err());
        match result.unwrap_err() {
            YarError::Connection(msg) => {
                assert!(msg.contains("failed to connect"));
            }
            other => panic!("expected Connection error, got: {:?}", other),
        }
    }

    #[test]
    fn test_client_connect_no_port() {
        let result = YarClient::connect("localhost");
        assert!(result.is_err());
        match result.unwrap_err() {
            YarError::Connection(msg) => {
                assert!(msg.contains("port"));
            }
            other => panic!("expected Connection error, got: {:?}", other),
        }
    }

    #[test]
    fn test_client_http_rejected() {
        let result = YarClient::connect("http://example.com:8080");
        assert!(result.is_err());
        match result.unwrap_err() {
            YarError::Connection(msg) => {
                assert!(msg.contains("HTTP"));
            }
            other => panic!("expected Connection error, got: {:?}", other),
        }

        let result = YarClient::connect("https://example.com:443");
        assert!(result.is_err());
        match result.unwrap_err() {
            YarError::Connection(msg) => {
                assert!(msg.contains("HTTP"));
            }
            other => panic!("expected Connection error, got: {:?}", other),
        }
    }

    #[test]
    fn test_client_name() {
        assert_eq!(
            CLIENT_NAME,
            concat!("Yar(Rust)-", env!("CARGO_PKG_VERSION"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_client_unix_socket_missing() {
        let result = YarClient::connect("/tmp/nonexistent_yar_test.sock");
        assert!(result.is_err());
        match result.unwrap_err() {
            YarError::Connection(msg) => {
                assert!(msg.contains("failed to connect"));
            }
            other => panic!("expected Connection error, got: {:?}", other),
        }
    }
}
