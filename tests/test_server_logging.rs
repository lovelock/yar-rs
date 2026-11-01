//! Server log-level behavior for peer disconnects.
//!
//! A client that connects and goes away without sending a complete request
//! (health check, port probe, aborted client, a load generator closing its
//! persistent connections) is routine. Logging each one at NOTICE floods the
//! log; only a truncated request is worth a notice.

use std::io::Write;
use std::time::{Duration, Instant};

use yar_rs::logger::{self, LogLevel};
use yar_rs::*;

fn read_log(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

#[test]
fn clean_disconnect_is_a_debug_line_not_a_read_error() {
    let dir = std::env::temp_dir().join(format!("yar-log-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log_path = dir.join("server.log");
    let _ = std::fs::remove_file(&log_path);

    // This test binary owns the process-global logger, so the server started
    // below writes into our file at DEBUG level.
    logger::init_global(log_path.to_str(), LogLevel::Debug).expect("logger init");

    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let addr = format!("127.0.0.1:{}", port);
    let server_addr = addr.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut server = YarServer::new(&server_addr);
            server.set_read_timeout(Duration::from_secs(5));
            server
                .register_handler("echo", |req| {
                    let mut pk = Packager::new();
                    pk.push_null().unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
            let _ = server.run().await;
        });
    });

    assert!(
        wait_until(Duration::from_secs(5), || read_log(&log_path).contains("Listening on")),
        "server never logged readiness:\n{}",
        read_log(&log_path)
    );

    // 1) health-check style: connect, send nothing, close.
    std::net::TcpStream::connect(&addr).unwrap();

    // 2) aborted client: two header bytes, then close (a truncated request).
    let mut partial = std::net::TcpStream::connect(&addr).unwrap();
    partial.write_all(&[0x80, 0xdf]).unwrap();
    partial.flush().unwrap();
    drop(partial);

    // 3) a complete header announcing a body, then close without the body:
    //    still a truncated request even though zero body bytes arrived.
    let mut header = yar_rs::protocol::YarHeader::new();
    header.render(1, None, None, 1024, 0);
    let header_bytes: [u8; yar_rs::protocol::HEADER_SIZE] = unsafe {
        std::ptr::read_unaligned(
            &header as *const yar_rs::protocol::YarHeader
                as *const [u8; yar_rs::protocol::HEADER_SIZE],
        )
    };
    let mut no_body = std::net::TcpStream::connect(&addr).unwrap();
    no_body.write_all(&header_bytes).unwrap();
    no_body.flush().unwrap();
    drop(no_body);

    assert!(
        wait_until(Duration::from_secs(5), || {
            let text = read_log(&log_path);
            text.contains("Connection closed by peer") && text.contains("Read error")
        }),
        "expected both a debug close line and a notice for the truncation:\n{}",
        read_log(&log_path)
    );

    let text = read_log(&log_path);
    assert!(
        text.lines().any(|l| l.contains("DEBUG") && l.contains("Connection closed by peer")),
        "a clean disconnect must be logged at DEBUG:\n{}",
        text
    );
    let notices: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("NOTICE") && l.contains("Read error"))
        .collect();
    assert_eq!(
        notices.len(),
        2,
        "the two truncated requests (not the clean close) should produce NOTICEs, got {}:\n{}",
        notices.len(),
        text
    );
}
