//! Server read-timeout semantics.
//!
//! The timeout is an *idle* timeout (per read), matching the C server's
//! `YAR_READ_TIMEOUT`. A large body that keeps arriving must still be served
//! when the whole request takes longer than the timeout under load, while a
//! peer that stops sending must be disconnected.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use yar_rs::client::CLIENT_NAME;
use yar_rs::protocol::{YarHeader, HEADER_SIZE, PROTOCOL_MAGIC, PROTOCOL_PERSISTENT};
use yar_rs::request::YarRequest;
use yar_rs::*;

fn find_available_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn wait_for_port(port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    for _ in 0..50 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("server on port {} never became reachable", port);
}

/// Start a server whose `echo` handler returns the parameters verbatim.
fn start_echo_server(port: u16, read_timeout: Duration) -> std::thread::JoinHandle<()> {
    let addr = format!("127.0.0.1:{}", port);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server = YarServer::new(&addr);
            server.set_read_timeout(read_timeout);
            server.set_max_request_size(8 * 1024 * 1024);
            server
                .register_handler("echo", |req| {
                    let mut pk = Packager::new();
                    match req.get_parameters() {
                        Some(p) => pk.push_data(p).unwrap(),
                        None => pk.push_null().unwrap(),
                    }
                    YarResponse::success(req.id(), pk)
                })
                .await;
            let _ = server.run().await;
        });
    })
}

/// Build a wire-ready persistent `echo` request carrying one string parameter.
fn build_request(body: &[u8]) -> Vec<u8> {
    let mut one = Packager::new();
    one.push_string(body).unwrap();
    let mut arr = Packager::start_array(1).unwrap();
    arr.push_packager(&one).unwrap();

    let mut request = YarRequest::new(7, "echo");
    request.set_parameters(arr);

    let extra = HEADER_SIZE + PACKAGER_NAME.len();
    let mut payload = request.pack(extra).unwrap();
    let body_len = (payload.len() - HEADER_SIZE) as u32;

    let mut header = YarHeader::new();
    header.render(7, Some(CLIENT_NAME), None, body_len, PROTOCOL_PERSISTENT);
    let header_bytes = header.to_bytes();
    payload[..HEADER_SIZE].copy_from_slice(&header_bytes);
    payload[HEADER_SIZE..extra].copy_from_slice(PACKAGER_NAME);
    payload
}

fn read_response(stream: &mut TcpStream) -> Option<usize> {
    let mut header = [0u8; HEADER_SIZE];
    stream.read_exact(&mut header).ok()?;
    let parsed = YarHeader::from_bytes(&header).ok()?;
    if parsed.magic_num != PROTOCOL_MAGIC {
        return None;
    }
    let mut body = vec![0u8; parsed.body_len as usize];
    stream.read_exact(&mut body).ok()?;
    Some(body.len())
}

#[test]
fn trickling_body_is_not_cut_off_by_the_idle_timeout() {
    let port = find_available_port();
    let timeout = Duration::from_millis(300);
    let server = start_echo_server(port, timeout);
    wait_for_port(port);

    let payload = build_request(&[b'x'; 4096]);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    // Header first, then the body in 8 chunks with 100 ms gaps: every gap is
    // well under the 300 ms idle timeout, but the request as a whole takes
    // ~800 ms, i.e. more than twice the timeout.
    stream.write_all(&payload[..HEADER_SIZE]).unwrap();
    stream.flush().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    for chunk in payload[HEADER_SIZE..].chunks(512) {
        stream.write_all(chunk).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(100));
    }

    let body_len = read_response(&mut stream)
        .expect("server closed the connection although the body kept arriving");
    // The echoed 4 KiB string makes the response far larger than any error.
    assert!(
        body_len > 4096,
        "response too small to be the echo: {} bytes",
        body_len
    );

    drop(server);
}

#[test]
fn silent_peer_is_disconnected_after_the_idle_timeout() {
    let port = find_available_port();
    let timeout = Duration::from_millis(200);
    let server = start_echo_server(port, timeout);
    wait_for_port(port);

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    // Announce a 1 KiB body but never send it.
    let mut header = YarHeader::new();
    header.render(1, None, None, 1024, 0);
    let header_bytes = header.to_bytes();
    stream.write_all(&header_bytes).unwrap();

    let started = Instant::now();
    let mut buf = [0u8; 1];
    // EOF (0 bytes) means the server closed the connection; a read error is
    // also acceptable under some kernels, but it must not take long.
    let _ = stream.read(&mut buf);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "idle peer was not disconnected: {:?}",
        started.elapsed()
    );

    drop(server);
}
