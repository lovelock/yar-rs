//! Server hardening tests: pid file, Unix sockets, request deadline.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use yar_rs::*;

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_for_port(port: u16) {
    let addr = format!("127.0.0.1:{port}");
    for _ in 0..50 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn default_request_limit_matches_c() {
    assert_eq!(DEFAULT_MAX_REQUEST_SIZE, 10 * 1024 * 1024);
}

#[test]
fn pid_file_is_written_on_startup() {
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("yar-rs-pid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pid_path = dir.join("server.pid");
    let _ = std::fs::remove_file(&pid_path);

    let pid_arg = pid_path.to_string_lossy().to_string();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server = YarServer::new(&format!("127.0.0.1:{port}"));
            server.set_pid_file(&pid_arg);
            server
                .register_handler("echo", |req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"ok").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
            let _ = server.run().await;
        });
    });

    wait_for_port(port);
    let contents = std::fs::read_to_string(&pid_path).expect("pid file must exist");
    assert_eq!(contents.trim(), std::process::id().to_string());
    let _ = std::fs::remove_file(&pid_path);
}

#[cfg(unix)]
#[test]
fn unix_socket_server_roundtrip() {
    let dir = std::env::temp_dir().join(format!("yar-rs-sock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("yar.sock");
    let _ = std::fs::remove_file(&sock);
    let sock_arg = sock.to_string_lossy().to_string();

    let sock_for_server = sock_arg.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = YarServer::new(&sock_for_server);
            server
                .register_handler("echo", |req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"unix-ok").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
            let _ = server.run().await;
        });
    });

    for _ in 0..50 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let mut client = YarClient::connect(&sock_arg).expect("connect to unix socket");
    client.set_timeout(Duration::from_secs(2));
    let resp = client.call("echo", vec![]).expect("unix call");
    assert_eq!(resp.get_response().unwrap().as_str(), Some("unix-ok"));

    let _ = std::fs::remove_file(&sock);
}

#[test]
fn total_request_deadline_closes_a_trickling_client() {
    let port = free_port();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server = YarServer::new(&format!("127.0.0.1:{port}"));
            server.set_read_timeout(Duration::from_secs(2));
            server.set_request_timeout(Some(Duration::from_millis(300)));
            server.set_max_inflight(8);
            server.run().await.unwrap();
        });
    });
    wait_for_port(port);

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    // Announce a body larger than the single byte we send, then trickle.
    let mut header = YarHeader::new();
    header.render(1, None, None, 1024, 0);
    stream.write_all(&header.to_bytes()).unwrap();

    let started = Instant::now();
    let mut buf = [0u8; 1];
    loop {
        if started.elapsed() > Duration::from_secs(3) {
            panic!("server did not enforce the total request deadline");
        }
        if stream.write_all(&[0x00]).is_err() {
            break;
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "connection should close shortly after the 300ms deadline, took {:?}",
        started.elapsed()
    );
}
