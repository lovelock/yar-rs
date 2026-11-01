//! Regression test: a socket timeout must surface as an error.
//!
//! `YarClient::set_timeout` configures `SO_RCVTIMEO`/`SO_SNDTIMEO`. A blocking
//! socket reports an expired timeout as `WouldBlock`, which used to be treated
//! as "retry": a call against a silent peer spun forever instead of failing.
//! That made the profiler unable to count timeouts as failures.

use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use yar_rs::YarClient;

#[test]
fn read_timeout_surfaces_as_error_instead_of_spinning() {
    // A peer that accepts the connection and never answers.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let _peer = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_secs(10));
    });

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut client = YarClient::connect(&addr.to_string()).unwrap();
        client.set_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let result = client.call("echo", vec![]);
        let _ = tx.send((result.is_err(), started.elapsed()));
    });

    let (is_err, elapsed) = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("client.call ignored the read timeout and never returned");
    assert!(is_err, "a timed-out call must fail, got Ok");
    assert!(
        elapsed >= Duration::from_millis(150),
        "returned before the timeout elapsed: {:?}",
        elapsed
    );
}
