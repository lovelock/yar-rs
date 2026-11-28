//! End-to-end test of the `yar-server-rust` echo entry point.
//!
//! Spawns the actual binary as a subprocess with an explicit CLI:
//! `--max-request-size 1m --read-timeout 5s` on an ephemeral port, then:
//!   1. echoes a persistent-client request with a 64 KiB body,
//!   2. sends a header whose declared body (1 MiB + 16) exceeds the limit
//!      and expects a non-zero status before any body is read,
//!   3. kills and reaps the child (RAII guard) so no process leaks —
//!      even when an assertion above fails.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use byteorder::{BigEndian, ByteOrder};

use yar_rs::client::YarClient;
use yar_rs::*;

/// RAII guard: kills and reaps the child on drop, including on panics,
/// so a failing assertion never leaves the server process behind.
struct ServerGuard {
    child: Child,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}

/// Helper: bind to port 0 to find an available port, then release it.
fn find_available_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Spawn the server binary and poll-connect until the port is reachable
/// (deadline ~5s). Panics with an explicit message if the child exits early.
fn start_server(port: u16) -> ServerGuard {
    let child = Command::new(env!("CARGO_BIN_EXE_yar-server-rust"))
        .args([
            "--host",
            &format!("127.0.0.1:{}", port),
            "--max-request-size",
            "1m",
            "--read-timeout",
            "5s",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn yar-server-rust");

    let mut guard = ServerGuard { child };

    let addr = format!("127.0.0.1:{}", port);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(Some(status)) = guard.child.try_wait() {
            panic!(
                "yar-server-rust exited early before accepting on {}: {}",
                addr, status
            );
        }
        match TcpStream::connect(&addr) {
            Ok(_) => return guard,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("server on {} never became reachable: {}", addr, e),
        }
    }
}

/// Build a raw Yar wire header (as the C client would) declaring the given
/// body length. The body itself is never sent for case (b).
fn build_header(id: u32, body_len: u32, persistent: bool) -> Vec<u8> {
    let mut header_bytes = vec![0u8; HEADER_SIZE];
    BigEndian::write_u32(&mut header_bytes[0..4], id); // id
    BigEndian::write_u16(&mut header_bytes[4..6], 0); // version
    BigEndian::write_u32(&mut header_bytes[6..10], PROTOCOL_MAGIC); // magic
    let reserved = if persistent { PROTOCOL_PERSISTENT } else { 0 };
    BigEndian::write_u32(&mut header_bytes[10..14], reserved); // reserved
                                                               // provider[14..46] and token[46..78] stay zeroed
    BigEndian::write_u32(&mut header_bytes[78..82], body_len); // body_len
    header_bytes
}

#[test]
fn test_binary_echo_and_oversized_rejection() {
    let port = find_available_port();
    let _server = start_server(port);
    let addr = format!("127.0.0.1:{}", port);

    // -- Case (a): persistent YarClient echo with a 64 KiB body ----------
    let mut body = Vec::with_capacity(65536);
    for i in 0..65536 {
        body.push((i % 256) as u8);
    }

    let mut client = YarClient::connect(&addr)
        .unwrap_or_else(|e| panic!("client failed to connect to {}: {}", addr, e));
    client.set_timeout(Duration::from_secs(5));
    client.set_persistent(true);

    let mut p = Packager::new();
    p.push_string(&body)
        .expect("packaging a 64 KiB string must not fail");
    let response = client
        .call("echo", vec![p])
        .unwrap_or_else(|e| panic!("echo call failed: {}", e));

    assert_eq!(
        response.get_status(),
        0,
        "echo call returned non-zero status: {:?}",
        response.get_error()
    );

    // The client wraps params in an array, so the echoed retval is an
    // array holding the body once: [body].
    let retval = response
        .get_response()
        .unwrap_or_else(|| panic!("echo response has no retval"));
    let arr = retval
        .as_array()
        .unwrap_or_else(|| panic!("echo retval is not an array"));
    assert_eq!(arr.len(), 1, "expected exactly one echoed parameter");

    let echoed = arr[0]
        .as_bytes()
        .unwrap_or_else(|| panic!("echoed parameter is not bytes"));
    assert_eq!(
        echoed.len(),
        65536,
        "echoed body length mismatch (expected 65536)"
    );

    // Compare only first and last 16 bytes of the echoed body.
    assert_eq!(
        &echoed[..16],
        &body[..16],
        "first 16 bytes of echoed body sent"
    );
    assert_eq!(
        &echoed[65520..],
        &body[65520..],
        "last 16 bytes of echoed body (bytes 65520..65536) sent"
    );

    drop(client);

    // -- Case (b): pre-body rejection of an oversized request ------------
    // Declare 1 MiB + 16 bytes while the server allows 1m = 1_048_576.
    let mut stream = TcpStream::connect(&addr).unwrap_or_else(|e| {
        panic!(
            "oversized-request test failed to reconnect to {}: {}",
            addr, e
        )
    });
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(&build_header(77, 1024 * 1024 + 16, false))
        .unwrap();
    stream.flush().unwrap();

    // The server must reject on the header alone: reply without ever
    // reading the (absent) body, then close the connection.
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break, // clean EOF after the error response
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) => panic!("read failed while awaiting oversized rejection: {:?}", e),
        }
    }

    assert!(
        raw.len() >= HEADER_SIZE,
        "expected at least a response header for the oversized request, got {} bytes",
        raw.len()
    );

    let response = YarResponse::unpack(&raw[HEADER_SIZE..], PACKAGER_NAME.len())
        .unwrap_or_else(|e| panic!("failed to unpack rejection response: {}", e));
    assert_ne!(
        response.get_status(),
        0,
        "oversized request must be rejected with a non-zero status"
    );
    assert!(
        response
            .get_error()
            .map(|m| m.contains("request too large"))
            .unwrap_or(false),
        "expected 'request too large' error, got: {:?}",
        response.get_error()
    );
}
