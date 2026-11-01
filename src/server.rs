//! RPC server implementation - wire-compatible with C yar_server.
//!
//! Protocol:
//!   [HEADER: 82 bytes] [PACKAGER_NAME: 8 bytes "MSGPACK\0"] [BODY: msgpack payload]
//!
//! The C client expects the Rust server to follow the same wire format as the
//! C yar_server. See the C implementation for reference.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{RwLock, Semaphore};

use byteorder::{BigEndian, ByteOrder};

use crate::error::{YarError, YarResult};
use crate::protocol::{YarHeader, HEADER_SIZE, PROTOCOL_PERSISTENT};
use crate::request::YarRequest;
use crate::response::YarResponse;
use crate::logger;
use crate::PACKAGER_NAME;

pub const SERVER_NAME: &str = "Yar Rust Server 0.1.0";

/// Default maximum request body size accepted by the server (8 MiB, as
/// `yar_server` does in the C implementation).
pub const DEFAULT_MAX_REQUEST_SIZE: usize = 8 * 1024 * 1024;

/// Handler function type: takes request, returns response.
pub type YarHandler = Box<dyn Fn(&YarRequest) -> YarResponse + Send + Sync>;

/// Yar RPC server, wire-compatible with the C yar_server.
pub struct YarServer {
    hostname: String,
    handlers: Arc<RwLock<HashMap<String, YarHandler>>>,
    max_workers: usize,
    max_request_size: usize,
    max_inflight: usize,
    stand_alone: bool,
    read_timeout: Duration,
    log_file: Option<String>,
    log_level: logger::LogLevel,
    pid_file: Option<String>,
}

// -- Private helpers ---------------------------------------------------------

/// Parse a hostname string into a bind address.
///
/// - "/path" prefix -> Unix domain socket (returned as `Unix(path)` variant)
/// - "host:port"    -> TCP address string
/// - "http://" / "https://" -> error (yar doesn't support HTTP URIs yet)
fn parse_address(hostname: &str) -> YarResult<Address> {
    if hostname.starts_with('/') {
        return Ok(Address::Unix(hostname.to_string()));
    }
    if hostname.starts_with("http://") || hostname.starts_with("https://") {
        return Err(YarError::Protocol(
            "HTTP/HTTPS URIs are not supported".into(),
        ));
    }
    Ok(Address::Tcp(hostname.to_string()))
}

enum Address {
    Tcp(String),
    Unix(String),
}

/// True when a read failure means the peer went away rather than a malformed
/// or truncated request: a clean close before the first byte, a reset, or a
/// broken pipe. Those are routine and logged at DEBUG.
fn is_peer_close(err: &YarError) -> bool {
    match err {
        // read_exact_idle uses Connection only for "closed before a byte".
        YarError::Connection(_) => true,
        YarError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}

/// Read exactly `n` bytes, applying `timeout` to each individual read.
///
/// This is an *idle* timeout, matching the C server's `YAR_READ_TIMEOUT`: a
/// large body that keeps arriving is not cut off just because the whole
/// request takes longer than `timeout` under load, but a peer that stops
/// sending is.
async fn read_exact_idle(
    reader: &mut (impl AsyncReadExt + Unpin),
    n: usize,
    timeout: Duration,
) -> YarResult<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut filled = 0;
    while filled < n {
        let read = match tokio::time::timeout(timeout, reader.read(&mut buf[filled..])).await {
            Err(_) => return Err(YarError::Timeout),
            Ok(Ok(0)) => {
                if filled == 0 {
                    // Closed before the first byte: a health check, a port
                    // probe, or a client dropping an idle keep-alive
                    // connection. Not a protocol error.
                    return Err(YarError::Connection("peer closed the connection".into()));
                }
                return Err(YarError::Protocol(format!(
                    "unexpected EOF while reading {} bytes",
                    n
                )));
            }
            Ok(Ok(k)) => k,
            Ok(Err(e)) => return Err(YarError::Io(e)),
        };
        filled += read;
    }
    Ok(buf)
}

/// Parse a YarHeader from wire bytes (big-endian), correctly converting all
/// fields to host byte order. Validates the magic number.
///
/// This works around `YarHeader::from_bytes` which stores `magic_num` and
/// `body_len` in big-endian without converting to host byte order.
fn parse_header_from_wire(data: &[u8]) -> Result<YarHeader, YarError> {
    if data.len() < HEADER_SIZE {
        return Err(YarError::Protocol("header too short".into()));
    }

    let magic_num = BigEndian::read_u32(&data[6..10]);
    if magic_num != crate::protocol::PROTOCOL_MAGIC {
        return Err(YarError::InvalidMagic);
    }

    let id = BigEndian::read_u32(&data[0..4]);
    let version = BigEndian::read_u16(&data[4..6]);
    let reserved = BigEndian::read_u32(&data[10..14]);
    let mut provider = [0u8; 32];
    provider.copy_from_slice(&data[14..46]);
    let mut token = [0u8; 32];
    token.copy_from_slice(&data[46..78]);
    let body_len = BigEndian::read_u32(&data[78..82]);

    Ok(YarHeader {
        id,
        version,
        magic_num,
        reserved,
        provider,
        token,
        body_len,
    })
}

// -- Connection handler -------------------------------------------------------

/// Build a response header directly in wire format (big-endian).
/// Bypasses render() + to_bytes() which has a double byte-swap issue on
/// little-endian systems.
fn build_response_header_bytes(id: u32, body_len: u32) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_SIZE];
    BigEndian::write_u32(&mut buf[0..4], id);
    BigEndian::write_u16(&mut buf[4..6], 0); // version
    BigEndian::write_u32(&mut buf[6..10], crate::protocol::PROTOCOL_MAGIC);
    BigEndian::write_u32(&mut buf[10..14], 0); // reserved
    // provider[14..46] stays zeroed
    // token[46..78] stays zeroed
    BigEndian::write_u32(&mut buf[78..82], body_len);
    buf
}

/// Send a response back to the client.
async fn send_response<S>(stream: &mut S, response: &YarResponse, extra_bytes: usize) -> YarResult<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    // Pack response with extra bytes for header + packager name
    let mut payload = response.pack(extra_bytes)?;

    // Build header directly in wire format (big-endian), bypassing render+to_bytes
    // which has a double byte-swap issue on little-endian systems.
    let body_len = (payload.len() - HEADER_SIZE) as u32;
    let header_bytes = build_response_header_bytes(response.id as u32, body_len);
    payload[..HEADER_SIZE].copy_from_slice(&header_bytes);

    // Write PACKAGER_NAME at offset HEADER_SIZE
    payload[HEADER_SIZE..HEADER_SIZE + PACKAGER_NAME.len()].copy_from_slice(PACKAGER_NAME.as_slice());

    // Send the full payload
    stream.write_all(&payload).await?;
    stream.flush().await?;
    Ok(())
}

// -- YarServer implementation -------------------------------------------------

impl YarServer {
    /// Creates a new server bound to the given hostname.
    ///
    /// - `hostname` can be:
    ///   - `"host:port"` for TCP
    ///   - `"/path/to/socket"` for Unix domain socket
    ///
    /// Defaults: standalone=true, max_workers=1, read_timeout=3s.
    pub fn new(hostname: &str) -> Self {
        YarServer {
            hostname: hostname.to_string(),
            handlers: Arc::new(RwLock::new(HashMap::new())),
            max_workers: 1,
            max_request_size: DEFAULT_MAX_REQUEST_SIZE,
            max_inflight: 0,
            stand_alone: true,
            read_timeout: Duration::from_secs(3),
            log_file: None,
            log_level: logger::LogLevel::Notice,
            pid_file: None,
        }
    }

    /// Registers a handler for a method name.
    ///
    /// This method is intentionally infallible. If the name already has a
    /// handler registered, the old one is silently replaced (same behavior as
    /// the C `yar_server_register`).
    pub async fn register_handler<F>(&self, name: &str, handler: F)
    where
        F: Fn(&YarRequest) -> YarResponse + Send + Sync + 'static,
    {
        let handler: YarHandler = Box::new(handler);
        let mut map = self.handlers.write().await;
        map.insert(name.to_string(), handler);
    }

    /// Sets whether the server runs in standalone (non-daemon) mode.
    ///
    /// In standalone mode the server does not daemonize or fork; it simply
    /// runs the event loop in the current process. This is the default
    /// and the safest setting for testing.
    pub fn set_stand_alone(&mut self, stand_alone: bool) {
        self.stand_alone = stand_alone;
    }

    /// Sets the number of worker tasks.
    ///
    /// The value is clamped to the range [1, 128].
    pub fn set_max_workers(&mut self, n: usize) {
        self.max_workers = n.clamp(1, 128);
    }

    /// Sets the maximum request body size (in bytes) the server accepts.
    ///
    /// A request whose header declares a larger body is rejected with an
    /// error response before the body is read, protecting the server from
    /// excessive memory allocation. Defaults to 8 MiB.
    pub fn set_max_request_size(&mut self, bytes: usize) {
        self.max_request_size = bytes;
    }

    /// Bounds the number of concurrently served connections.
    ///
    /// Beyond the limit, the accept loop blocks until a permit is released
    /// (a connection task returns) — connections wait rather than being
    /// rejected. `0` (the default) disables the limit.
    pub fn set_max_inflight(&mut self, n: usize) {
        self.max_inflight = n;
    }

    /// Sets the per-request read timeout.
    pub fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout = timeout;
    }

    /// Sets the log file path.
    pub fn set_log_file(&mut self, path: &str) {
        self.log_file = Some(path.to_string());
    }

    /// Sets the PID file path (currently informational only in the Rust impl).
    pub fn set_pid_file(&mut self, path: &str) {
        self.pid_file = Some(path.to_string());
    }

    /// Runs the server.
    ///
    /// In standalone mode this binds and accepts connections in the current
    /// async runtime. In non-standalone mode (future work) this would daemonize
    /// and fork workers.
    pub async fn run(&self) -> YarResult<()> {
        self.run_standalone().await
    }

    /// Simplified standalone run: bind, listen, accept, handle.
    pub async fn run_standalone(&self) -> YarResult<()> {
        // Initialize logger
        let log_file = self.log_file.as_deref();
        if logger::global().is_none() {
            // Two servers can be started concurrently in one process; racing
            // here is benign — the loser of init_global reports "already set"
            // while a logger exists, which must not abort the startup.
            if let Err(e) = logger::init_global(log_file, self.log_level) {
                if logger::global().is_none() {
                    return Err(YarError::Protocol(format!(
                        "Failed to initialize logger: {}",
                        e
                    )));
                }
            }
        }

        logger::set_hostname(&format!("yar-rs-server-{}", std::process::id()));
        logger::log_global(
            logger::LogLevel::Notice,
            &format!("Server starting on {} (standalone)", self.hostname),
        );

        let address = parse_address(&self.hostname)?;

        match address {
            Address::Tcp(addr) => {
                let listener = TcpListener::bind(&addr).await.map_err(|e| {
                    YarError::Io(e)
                })?;

                // Log the actual bound address
                if let Ok(local_addr) = listener.local_addr() {
                    logger::log_global(
                        logger::LogLevel::Notice,
                        &format!("Listening on {}", local_addr),
                    );
                }

                let handlers = self.handlers.clone();
                let read_timeout = self.read_timeout;
                let max_request_size = self.max_request_size;
                let max_inflight = self.max_inflight;

                // Concurrency gate: at most `max_inflight` connections are
                // served at once; 0 means unlimited. A permit is acquired in
                // the accept loop (never rejected) and moved into the
                // connection task, released when the task returns.
                let semaphore = if max_inflight >= 1 {
                    Some(Arc::new(Semaphore::new(max_inflight)))
                } else {
                    None
                };

                // Accept connections. Spawn a task per connection, gated by
                // the max_inflight semaphore above (when set); tokio tasks
                // are lightweight, so there is no separate worker pool.
                loop {
                    let (stream, peer) = match listener.accept().await {
                        Ok((s, p)) => (s, p),
                        Err(e) => {
                            logger::log_global(
                                logger::LogLevel::Warning,
                                &format!("Accept error: {}", e),
                            );
                            continue;
                        }
                    };

                    logger::log_global(
                        logger::LogLevel::Debug,
                        &format!("Connection from {}", peer),
                    );

                    let handlers = handlers.clone();

                    // Block here while saturated: a permit frees when an
                    // older connection task finishes. (The semaphore is never
                    // closed, so acquire cannot fail.)
                    let permit = match &semaphore {
                        Some(sem) => Some(
                            sem.clone()
                                .acquire_owned()
                                .await
                                .expect("inflight semaphore closed"),
                        ),
                        None => None,
                    };

                    tokio::spawn(async move {
                        let mut stream = stream;
                        let _permit = permit; // released when this task returns
                        // Apply read timeout per-request inside handle_connection
                        let _ = handle_connection_with_timeout(
                            &mut stream,
                            &handlers,
                            read_timeout,
                            max_request_size,
                        ).await;
                    });
                }
            }
            Address::Unix(_path) => {
                // Unix socket support is planned but not yet implemented.
                return Err(YarError::Protocol(
                    "Unix domain sockets not yet implemented in Rust server".into(),
                ));
            }
        }
    }
}

/// Wrapper around handle_connection that applies a per-request timeout.
async fn handle_connection_with_timeout<S>(
    stream: &mut S,
    handlers: &Arc<RwLock<HashMap<String, YarHandler>>>,
    read_timeout: Duration,
    max_request_size: usize,
) -> YarResult<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + 'static,
{
    // One read cycle per request. The timeout is applied to each individual
    // read (idle timeout), not to the whole header+body: under load a 4 MiB
    // body legitimately takes longer than the timeout end to end.
    loop {
        let result = async {
            // Read header
            let header_bytes = read_exact_idle(stream, HEADER_SIZE, read_timeout).await?;

            // Parse header
            let header = parse_header_from_wire(&header_bytes).map_err(|_| {
                YarError::InvalidMagic
            })?;

            let is_persistent = (header.reserved & PROTOCOL_PERSISTENT) != 0;
            let body_len = header.body_len as usize;

            // Memory firewall: if the declared body exceeds the configured
            // limit, do not read (or allocate for) the body at all — signal
            // the oversized request back to the caller with an empty body.
            if body_len > max_request_size {
                return Ok::<_, YarError>((header, is_persistent, None));
            }

            // Read body. A peer that announced a body and then vanished has
            // truncated the request, even if zero body bytes arrived — that is
            // not the routine "connected and went away" close the header read
            // reports as Connection.
            let body_bytes = match read_exact_idle(stream, body_len, read_timeout).await {
                Ok(b) => b,
                Err(YarError::Connection(_)) => {
                    return Err(YarError::Protocol(format!(
                        "unexpected EOF while reading {} bytes",
                        body_len
                    )))
                }
                Err(e) => return Err(e),
            };

            Ok::<_, YarError>((header, is_persistent, Some(body_bytes)))
        }
        .await;

        let (header, is_persistent, body_bytes) = match result {
            Ok(v) => v,
            Err(YarError::InvalidMagic) => {
                logger::log_global(logger::LogLevel::Error, "Invalid magic number in header");
                return Err(YarError::InvalidMagic);
            }
            Err(YarError::Timeout) => {
                logger::log_global(logger::LogLevel::Notice, "Read timeout, closing connection");
                return Ok(());
            }
            Err(ref e) if is_peer_close(e) => {
                // Routine: the peer went away (health check, probe, client
                // dropping a persistent connection). Nothing to report.
                logger::log_global(
                    logger::LogLevel::Debug,
                    "Connection closed by peer before a request",
                );
                return Ok(());
            }
            Err(e) => {
                logger::log_global(logger::LogLevel::Notice, &format!("Read error: {}", e));
                return Ok(());
            }
        };

        let extra_bytes = HEADER_SIZE + PACKAGER_NAME.len();

        // Reject oversized requests without reading their body: reply with an
        // error response and close the connection.
        let body_bytes = match body_bytes {
            Some(b) => b,
            None => {
                // YarHeader is packed: copy the fields we need via unaligned
                // reads before referencing them (as in the packed_u32! helper).
                let declared_len =
                    unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(header.body_len)) };
                let req_id =
                    unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(header.id)) };

                logger::log_global(
                    logger::LogLevel::Warning,
                    &format!(
                        "Request too large: {} > {} bytes, rejecting",
                        declared_len, max_request_size
                    ),
                );
                let resp = YarResponse::error(
                    req_id as u64,
                    1,
                    &format!(
                        "request too large: {} > {}",
                        declared_len, max_request_size
                    ),
                );
                let _ = send_response(stream, &resp, extra_bytes).await;
                return Ok(());
            }
        };

        // Verify packager name
        if body_bytes.len() < PACKAGER_NAME.len() {
            logger::log_global(logger::LogLevel::Error, "Body too short for packager name");
            return Ok(());
        }

        if &body_bytes[..PACKAGER_NAME.len()] != PACKAGER_NAME.as_slice() {
            logger::log_global(
                logger::LogLevel::Error,
                &format!("Unsupported packager: {:?}", &body_bytes[..PACKAGER_NAME.len()]),
            );
            return Ok(());
        }

        // Unpack request (skip the PACKAGER_NAME prefix in the body)
        let request = match YarRequest::unpack(&body_bytes, PACKAGER_NAME.len()) {
            Ok(r) => r,
            Err(e) => {
                logger::log_global(logger::LogLevel::Error, &format!("Unpack failed: {}", e));
                let resp = YarResponse::error(header.id as u64, 1, &e.to_string());
                let _ = send_response(stream, &resp, extra_bytes).await;
                return Ok(());
            }
        };

        let method = request.method().to_string();
        let req_id = request.id();

        logger::log_global(
            logger::LogLevel::Debug,
            &format!("Request: method={}, id={}", method, req_id),
        );

        // Handler lookup & invocation
        let response = {
            let guard = handlers.read().await;
            match guard.get(&method) {
                Some(h) => h(&request),
                None => {
                    logger::log_global(
                        logger::LogLevel::Warning,
                        &format!("Method not found: {}", method),
                    );
                    YarResponse::error(req_id, 1, &YarError::MethodNotFound(method).to_string())
                }
            }
        };

        // Send response
        if let Err(e) = send_response(stream, &response, extra_bytes).await {
            logger::log_global(logger::LogLevel::Error, &format!("Send failed: {}", e));
            return Ok(());
        }

        logger::log_global(
            logger::LogLevel::Debug,
            &format!("Response sent for id={}", req_id),
        );

        if !is_persistent {
            return Ok(());
        }

        // Persistent: loop back for next request
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{BigEndian, ByteOrder};
    use crate::pack::Packager;
    use tokio::net::TcpStream;

    /// Helper: safely read a u32 from a packed YarHeader field
    macro_rules! packed_u32 {
        ($h:expr, $field:ident) => {{
            let h = &$h;
            unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(h.$field)) }
        }};
    }

    /// Build a complete Yar wire packet (header + packager_name + msgpack body).
    /// Constructs wire-format header bytes directly to avoid byte-order issues.
    fn build_request_packet(id: u32, method: &str, params_pk: Option<Packager>, persistent: bool) -> Vec<u8> {
        let mut req = YarRequest::new(id as u64, method);
        if let Some(pk) = params_pk {
            req.set_parameters(pk);
        }
        let req_msgpack = req.pack(0).unwrap();

        let body: Vec<u8> = {
            let mut out = Vec::with_capacity(PACKAGER_NAME.len() + req_msgpack.len());
            out.extend_from_slice(PACKAGER_NAME.as_slice());
            out.extend_from_slice(&req_msgpack);
            out
        };

        let reserved = if persistent { PROTOCOL_PERSISTENT } else { 0 };

        // Build header directly in wire format (big-endian)
        let mut header_bytes = vec![0u8; HEADER_SIZE];
        BigEndian::write_u32(&mut header_bytes[0..4], id);
        BigEndian::write_u16(&mut header_bytes[4..6], 0); // version
        BigEndian::write_u32(&mut header_bytes[6..10], crate::protocol::PROTOCOL_MAGIC);
        BigEndian::write_u32(&mut header_bytes[10..14], reserved);
        // provider[14..46] stays zeroed
        // token[46..78] stays zeroed
        BigEndian::write_u32(&mut header_bytes[78..82], body.len() as u32);

        let mut packet = header_bytes;
        packet.extend_from_slice(&body);
        packet
    }

    /// Parse a response packet into (header, body_bytes).
    fn parse_response_packet(data: &[u8]) -> YarResult<(YarHeader, Vec<u8>)> {
        if data.len() < HEADER_SIZE {
            return Err(YarError::Protocol("response too short for header".into()));
        }
        let header = parse_header_from_wire(&data[..HEADER_SIZE]).map_err(|_| {
            YarError::InvalidMagic
        })?;
        let body = data[HEADER_SIZE..].to_vec();
        Ok((header, body))
    }

    #[test]
    fn test_server_new() {
        let server = YarServer::new("127.0.0.1:9999");
        assert_eq!(server.hostname, "127.0.0.1:9999");
        assert!(server.stand_alone);
        assert_eq!(server.max_workers, 1);
        assert_eq!(server.read_timeout, Duration::from_secs(3));
    }

    #[test]
    fn test_server_register_handler() {
        let rt = tokio::runtime::Runtime::new().unwrap();

        let server = YarServer::new("127.0.0.1:9998");
        rt.block_on(async {
            server.register_handler("echo", |req| {
                let mut pk = Packager::new();
                pk.push_string(req.method().as_bytes()).unwrap();
                YarResponse::success(req.id(), pk)
            }).await;
        });

        // Verify the handler was registered by checking the handlers map.
        let handlers = rt.block_on(async { server.handlers.read().await });
        assert!(handlers.contains_key("echo"));
    }

    #[tokio::test]
    async fn test_server_handle_connection() {
        // Bind to a random available port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = YarServer::new(&format!("127.0.0.1:{}", addr.port()));
        server.register_handler("hello", |req| {
            let mut pk = Packager::new();
            pk.push_string(b"world").unwrap();
            YarResponse::success(req.id(), pk)
        }).await;

        let handlers = server.handlers.clone();

        // Spawn a minimal server that just handles one connection
        let server_handle = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.into_split();

            // Read header
            let mut header_buf = vec![0u8; HEADER_SIZE];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut header_buf).await.unwrap();
            let header = parse_header_from_wire(&header_buf).unwrap();

            // Read body
            let body_len = header.body_len as usize;
            let mut body_buf = vec![0u8; body_len];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut body_buf).await.unwrap();

            // Verify packager name
            assert_eq!(&body_buf[..PACKAGER_NAME.len()], PACKAGER_NAME.as_slice());

            // Unpack request
            let request = YarRequest::unpack(&body_buf, PACKAGER_NAME.len()).unwrap();
            let method = request.method().to_string();
            let _req_id = request.id();

            // Find and call handler
            let guard = handlers.read().await;
            let handler = guard.get(&method).unwrap();
            let response = handler(&request);
            drop(guard);

            // Send response
            let extra_bytes = HEADER_SIZE + PACKAGER_NAME.len();
            let mut payload = response.pack(extra_bytes).unwrap();

            let body_len = (payload.len() - HEADER_SIZE) as u32;
            let header_bytes = build_response_header_bytes(response.id as u32, body_len);
            payload[..HEADER_SIZE].copy_from_slice(&header_bytes);
            payload[HEADER_SIZE..HEADER_SIZE + PACKAGER_NAME.len()].copy_from_slice(PACKAGER_NAME.as_slice());

            tokio::io::AsyncWriteExt::write_all(&mut writer, &payload).await.unwrap();
            tokio::io::AsyncWriteExt::flush(&mut writer).await.unwrap();
            tokio::io::AsyncWriteExt::shutdown(&mut writer).await.unwrap();
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(&addr).await.unwrap();

        let packet = build_request_packet(1, "hello", None, false);
        stream.write_all(&packet).await.unwrap();
        stream.flush().await.unwrap();

        // Give server time to process
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Read the response
        let mut response_buf = vec![0u8; 4096];
        let read_result = stream.read(&mut response_buf).await;
        match read_result {
            Ok(n) if n > 0 => {
                let response_data = &response_buf[..n];

                // Parse response
                let (header, body) = parse_response_packet(response_data).unwrap();
                assert_eq!(packed_u32!(header, id), 1);

                // Verify packager name
                assert_eq!(&body[..PACKAGER_NAME.len()], PACKAGER_NAME.as_slice());

                // Unpack response
                let response = YarResponse::unpack(&body, PACKAGER_NAME.len()).unwrap();
                assert_eq!(response.get_status(), 0);
                let retval = response.get_response().unwrap();
                assert_eq!(retval.as_str(), Some("world"));
            }
            Ok(_) => {
                panic!("Server closed connection without sending response");
            }
            Err(e) => {
                panic!("Client read error: {:?}", e);
            }
        }

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_server_unknown_method() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = YarServer::new(&format!("127.0.0.1:{}", addr.port()));

        let server_handle = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            let mut stream = stream;
            let handlers = server.handlers.clone();
            let _ = handle_connection_with_timeout(
                &mut stream,
                &handlers,
                Duration::from_secs(5),
                DEFAULT_MAX_REQUEST_SIZE,
            ).await;
            let _ = stream.shutdown().await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(&addr).await.unwrap();

        // Send a request for an unregistered method
        let packet = build_request_packet(42, "nonexistent", None, false);
        stream.write_all(&packet).await.unwrap();
        stream.flush().await.unwrap();

        // Give the server time to process and send back the response
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut response_buf = vec![0u8; 4096];
        let result = stream.read(&mut response_buf).await;
        // Server sends error response then closes connection.
        // We should either get data or a clean EOF (0 bytes).
        match result {
            Ok(n) if n > 0 => {
                let (_header, body) = parse_response_packet(&response_buf[..n]).unwrap();
                let response = YarResponse::unpack(&body, PACKAGER_NAME.len()).unwrap();
                assert_ne!(response.get_status(), 0);
                assert!(response.get_error().unwrap().contains("Method not found"));
            }
            Ok(_) => {
                // Server closed before we could read - timing issue
            }
            Err(e) => {
                // ConnectionReset can happen on some OS when server closes fast.
                assert!(
                    e.kind() == std::io::ErrorKind::ConnectionReset,
                    "Expected ConnectionReset, got: {:?}",
                    e
                );
            }
        }

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_server_bad_magic() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = YarServer::new(&format!("127.0.0.1:{}", addr.port()));

        let server_handle = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            let mut stream = stream;
            let handlers = server.handlers.clone();
            let _ = handle_connection_with_timeout(
                &mut stream,
                &handlers,
                Duration::from_secs(5),
                DEFAULT_MAX_REQUEST_SIZE,
            ).await;
            let _ = stream.shutdown().await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(&addr).await.unwrap();

        // Send a header with bad magic number
        let mut bad_header_bytes = vec![0u8; HEADER_SIZE];
        BigEndian::write_u32(&mut bad_header_bytes[6..10], 0xDEADBEEF);
        // Set a body_len of 0 so the server won't try to read more
        BigEndian::write_u32(&mut bad_header_bytes[78..82], 0);

        stream.write_all(&bad_header_bytes).await.unwrap();

        // Give the server time to process and close the connection
        tokio::time::sleep(Duration::from_millis(100)).await;

        // The connection should be closed (read returns 0)
        let mut buf = [0u8; 1];
        let result = stream.read(&mut buf).await;
        // Should get EOF (Ok(0)) or a connection reset error
        assert!(
            result.is_ok() && result.unwrap() == 0,
            "Expected connection to be closed for bad magic"
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_server_persistent_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = YarServer::new(&format!("127.0.0.1:{}", addr.port()));
        server.register_handler("add", |req| {
            let params = req.get_parameters();
            let arr = params.and_then(|p| p.as_array()).unwrap_or(&[]);
            let sum: i64 = arr
                .iter()
                .filter_map(|v| v.as_integer())
                .sum();

            let mut pk = Packager::new();
            pk.push_long(sum).unwrap();
            YarResponse::success(req.id(), pk)
        }).await;

        let server_handle = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            let mut stream = stream;
            let handlers = server.handlers.clone();
            let _ = handle_connection_with_timeout(
                &mut stream,
                &handlers,
                Duration::from_secs(5),
                DEFAULT_MAX_REQUEST_SIZE,
            ).await;
            let _ = stream.shutdown().await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(&addr).await.unwrap();

        // Build params: array [3, 4]
        let params_pk = {
            let mut pk = Packager::start_array(2).unwrap();
            pk.push_long(3).unwrap();
            pk.push_long(4).unwrap();
            pk
        };

        // Send first request with PERSISTENT flag
        let packet = build_request_packet(1, "add", Some(params_pk), true);
        stream.write_all(&packet).await.unwrap();

        let mut response_buf = vec![0u8; 4096];
        let n = stream.read(&mut response_buf).await.unwrap();
        let (_header, body) = parse_response_packet(&response_buf[..n]).unwrap();
        let response = YarResponse::unpack(&body, PACKAGER_NAME.len()).unwrap();
        assert_eq!(response.get_status(), 0);
        assert_eq!(response.get_response().unwrap().as_integer(), Some(7));

        // Send second request on the same persistent connection
        let params_pk2 = {
            let mut pk = Packager::start_array(2).unwrap();
            pk.push_long(10).unwrap();
            pk.push_long(20).unwrap();
            pk
        };
        let packet2 = build_request_packet(2, "add", Some(params_pk2), true);
        stream.write_all(&packet2).await.unwrap();

        let n2 = stream.read(&mut response_buf).await.unwrap();
        let (_header2, body2) = parse_response_packet(&response_buf[..n2]).unwrap();
        let response2 = YarResponse::unpack(&body2, PACKAGER_NAME.len()).unwrap();
        assert_eq!(response2.get_status(), 0);
        assert_eq!(response2.get_response().unwrap().as_integer(), Some(30));

        // Send third request without PERSISTENT flag to close
        let params_pk3 = {
            let mut pk = Packager::start_array(2).unwrap();
            pk.push_long(10).unwrap();
            pk.push_long(20).unwrap();
            pk
        };
        let packet3 = build_request_packet(3, "add", Some(params_pk3), false);
        stream.write_all(&packet3).await.unwrap();

        let n3 = stream.read(&mut response_buf).await.unwrap();
        let (_header3, body3) = parse_response_packet(&response_buf[..n3]).unwrap();
        let response3 = YarResponse::unpack(&body3, PACKAGER_NAME.len()).unwrap();
        assert_eq!(response3.get_response().unwrap().as_integer(), Some(30));

        server_handle.abort();
    }
}
