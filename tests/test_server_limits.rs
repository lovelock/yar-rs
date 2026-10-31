//! Server-side request limits tests.
//!
//! Verifies the server's memory firewall: a request whose declared body
//! length exceeds `set_max_request_size` is rejected with an error response
//! as soon as the header is parsed — before the server reads or allocates
//! for the body.

use std::io::{Read, Write};
use std::time::Duration;

use yar_rs::*;

/// Helper: safely read a u32 from a packed YarHeader field.
macro_rules! packed_u32 {
    ($h:expr, $field:ident) => {{
        let h = &$h;
        unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(h.$field)) }
    }};
}

/// Helper: bind to port 0 to find an available port, then release it.
fn find_available_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Helper: wait for a port to become available, retrying up to 2 seconds.
fn wait_for_port(port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    for _ in 0..20 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Helper: start a YarServer in a background thread with a tokio runtime.
/// Returns the JoinHandle for cleanup.
fn start_server(
    port: u16,
    setup: impl FnOnce(&mut YarServer) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> + Send + 'static,
) -> std::thread::JoinHandle<()> {
    let addr = format!("127.0.0.1:{}", port);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server = YarServer::new(&addr);
            setup(&mut server).await;
            let _ = server.run().await;
        });
    })
}

// ---------------------------------------------------------------------------
// Concurrent server startup in one process must be benign
// ---------------------------------------------------------------------------

#[test]
fn test_concurrent_server_start_same_process() {
    // Two servers whose run() calls race the global-logger check-then-init
    // in run_standalone. The loser (whose init_global hits "already set")
    // must still bind and serve — an early exit here is the regression.
    let barrier = std::sync::Barrier::new(2);
    let barrier = std::sync::Arc::new(barrier);
    let (early_exit_tx, early_exit_rx) = std::sync::mpsc::channel();

    let mut addresses = Vec::new();
    let mut ports = Vec::new();
    for _ in 0..2 {
        let port = find_available_port();
        let addr = format!("127.0.0.1:{}", port);
        ports.push(port);
        addresses.push(addr.clone());

        let barrier = barrier.clone();
        let early_exit_tx = early_exit_tx.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                let mut server = YarServer::new(&addr);
                server.set_max_request_size(4096);
                server.register_handler("echo", |req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"echo:ok").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
                // Align both run() calls so the concurrent init race is hit.
                barrier.wait();
                server.run().await
            });
            // Only run() errors ever reach this send (success never returns
            // from the accept loop); relayed so the test can assert on it.
            let _ = early_exit_tx.send(result);
        });
    }
    drop(early_exit_tx);

    for addr in &addresses {
        wait_for_port(ports.remove(0));

        let mut client = YarClient::connect(addr).unwrap_or_else(|e| {
            panic!(
                "server on {} failed to start: {} — concurrent logger init must be benign",
                addr, e
            )
        });
        client.set_timeout(Duration::from_secs(5));

        let response = client.call("echo", vec![]).unwrap();
        assert_eq!(response.get_status(), 0);
        let retval = response.get_response().unwrap();
        assert_eq!(retval.as_str(), Some("echo:ok"));

        client.close();
    }

    // No server may have exited early during the racing startup.
    match early_exit_rx.recv_timeout(Duration::from_millis(500)) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {} // all still serving
        other => panic!("a server exited early during startup: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Memory firewall: oversized requests rejected from the header alone
// ---------------------------------------------------------------------------

#[test]
fn test_oversized_request_rejected_before_reading_body() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server.set_max_request_size(1024);
            server.register_handler("echo", |req| {
                let mut pk = Packager::new();
                pk.push_string(b"echo:ok").unwrap();
                YarResponse::success(req.id(), pk)
            }).await;
        })
    });

    wait_for_port(port);

    // Hand-craft an 82-byte big-endian header declaring a 2048-byte body
    // (larger than the configured 1024-byte limit), but never send a body.
    // Constructed as the C client would produce it (see test_cross_language).
    let mut header_bytes = vec![0u8; HEADER_SIZE];
    use byteorder::{BigEndian, ByteOrder};
    BigEndian::write_u32(&mut header_bytes[0..4], 42);              // id
    BigEndian::write_u16(&mut header_bytes[4..6], 0);               // version
    BigEndian::write_u32(&mut header_bytes[6..10], PROTOCOL_MAGIC); // magic
    BigEndian::write_u32(&mut header_bytes[10..14], 0);             // reserved
    // provider[14..46] stays zero
    // token[46..78] stays zero
    BigEndian::write_u32(&mut header_bytes[78..82], 2048);          // body_len

    let mut stream = std::net::TcpStream::connect(&addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    stream.write_all(&header_bytes).unwrap();
    stream.flush().unwrap();

    // The server must reject the request on the header alone and reply
    // without ever attempting to read the (absent) body. Read until EOF.
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break, // clean EOF after the error response
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) => panic!("read failed before EOF: {:?}", e),
        }
    }

    assert!(raw.len() >= HEADER_SIZE, "expected at least a response header, got {} bytes", raw.len());

    // Parse the response header and body (via unaligned reads — YarHeader is packed)
    let header = YarHeader::from_bytes(&raw[..HEADER_SIZE]).unwrap();
    let body = &raw[HEADER_SIZE..];
    assert_eq!(packed_u32!(header, id), 42);

    // Skip the PACKAGER_NAME prefix and unpack the response payload
    let response = YarResponse::unpack(body, PACKAGER_NAME.len()).unwrap();
    assert_ne!(response.get_status(), 0);
    assert!(
        response.get_error().unwrap().contains("request too large"),
        "unexpected error message: {:?}",
        response.get_error()
    );
}

// ---------------------------------------------------------------------------
// Requests within the limit are still served normally
// ---------------------------------------------------------------------------

#[test]
fn test_request_within_limit_still_served() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server.set_max_request_size(1024);
            server.register_handler("echo", |req| {
                let mut pk = Packager::new();
                pk.push_string(b"echo:ok").unwrap();
                YarResponse::success(req.id(), pk)
            }).await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));

    let response = client.call("echo", vec![]).unwrap();
    assert_eq!(response.get_status(), 0);
    let retval = response.get_response().unwrap();
    assert_eq!(retval.as_str(), Some("echo:ok"));

    client.close();
}
