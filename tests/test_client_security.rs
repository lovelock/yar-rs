//! Client-side hardening tests: untrusted or malformed server responses.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use byteorder::{BigEndian, ByteOrder};
use yar_rs::{Packager, YarClient, YarResponse, HEADER_SIZE, PACKAGER_NAME, PROTOCOL_MAGIC};

/// Spawn a one-shot fake server that answers any request with `body`.
/// `body_len_override` lets a test lie about the body length.
fn fake_server(body: Vec<u8>, body_len_override: Option<u32>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Drain the request so the client's write returns.
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);

        let body_len = body_len_override.unwrap_or(body.len() as u32);

        let mut header = vec![0u8; HEADER_SIZE];
        BigEndian::write_u32(&mut header[0..4], 1000); // matching request id
        BigEndian::write_u16(&mut header[4..6], 0);
        BigEndian::write_u32(&mut header[6..10], PROTOCOL_MAGIC);
        BigEndian::write_u32(&mut header[10..14], 0);
        BigEndian::write_u32(&mut header[78..82], body_len);

        stream.write_all(&header).unwrap();
        if !body.is_empty() {
            stream.write_all(&body).unwrap();
        }
        let _ = stream.flush();
    });

    (addr, handle)
}

fn response_body(id: u64, packager: &[u8]) -> Vec<u8> {
    let mut pk = Packager::new();
    pk.push_string(b"ok").unwrap();
    let msgpack = YarResponse::success(id, pk).pack(0).unwrap();
    let mut body = packager.to_vec();
    body.extend_from_slice(&msgpack);
    body
}

#[test]
fn oversized_response_body_is_rejected_before_allocation() {
    let (addr, server) = fake_server(Vec::new(), Some(20 * 1024 * 1024));
    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(std::time::Duration::from_secs(2));

    let err = client.call("echo", vec![]).err().expect("must reject");
    assert!(
        err.to_string().contains("response body") && err.to_string().contains("exceeds"),
        "expected a response size error, got: {err}"
    );

    server.join().unwrap();
}

#[test]
fn wrong_packager_name_is_rejected() {
    let body = response_body(1000, b"WRONGPK\0");
    let (addr, server) = fake_server(body, None);
    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(std::time::Duration::from_secs(2));

    let err = client.call("echo", vec![]).err().expect("must reject");
    assert!(
        err.to_string().contains("packager"),
        "expected a packager error, got: {err}"
    );

    server.join().unwrap();
}

#[test]
fn mismatched_response_id_is_rejected() {
    let body = response_body(999, PACKAGER_NAME);
    let (addr, server) = fake_server(body, None);
    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(std::time::Duration::from_secs(2));

    let err = client.call("echo", vec![]).err().expect("must reject");
    assert!(
        err.to_string().contains("id"),
        "expected a response id error, got: {err}"
    );

    server.join().unwrap();
}

#[test]
fn a_failed_call_poisons_the_connection() {
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);

        // Answer the first request after the client's read timeout expires.
        thread::sleep(Duration::from_millis(300));
        let body = response_body(1000, PACKAGER_NAME);
        let mut header = vec![0u8; HEADER_SIZE];
        BigEndian::write_u32(&mut header[0..4], 1000);
        BigEndian::write_u32(&mut header[6..10], PROTOCOL_MAGIC);
        BigEndian::write_u32(&mut header[78..82], body.len() as u32);
        let _ = stream.write_all(&header);
        let _ = stream.write_all(&body);
        let _ = stream.flush();
    });

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_millis(100));

    let first = client.call("echo", vec![]).err().expect("must time out");
    assert!(
        matches!(first, yar_rs::YarError::Timeout),
        "expected Timeout, got: {first}"
    );

    // The socket still holds the late response. A second call must not be
    // allowed to read it as its own result, even with a generous timeout.
    client.set_timeout(Duration::from_secs(2));
    let second = client.call("echo", vec![]).err().expect("must reject");
    assert!(
        second.to_string().contains("unusable") || second.to_string().contains("failed call"),
        "expected a poisoned-connection error, got: {second}"
    );

    server.join().unwrap();
}
