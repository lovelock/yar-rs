//! C-parity behavior: server error status, `tcp://`, and the pre-packed
//! client path added for the benchmark.

use std::time::Duration;
use yar_rs::*;

fn find_available_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn wait_for_port(port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    for _ in 0..40 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start on {port}");
}

fn start_echo_server(hostname: String) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut server = YarServer::new(&hostname);
            server.set_access_log(false);
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

#[test]
fn unknown_method_uses_c_error_status_8() {
    let port = find_available_port();
    let _server = start_echo_server(format!("127.0.0.1:{}", port));
    wait_for_port(port);

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();
    let resp = client.call("does-not-exist", vec![]).unwrap();
    assert_eq!(resp.get_status(), 8, "C uses YAR_ERROR=8");
    assert!(resp.get_error().unwrap().contains("Method not found"));
}

#[test]
fn client_and_server_accept_tcp_uri() {
    let port = find_available_port();
    let _server = start_echo_server(format!("tcp://127.0.0.1:{}", port));
    wait_for_port(port);

    let mut client = YarClient::connect(&format!("tcp://127.0.0.1:{}", port)).unwrap();
    let resp = client.call("echo", Vec::new()).unwrap();
    assert_eq!(resp.get_status(), 0);
}

#[test]
fn pre_packed_request_roundtrip() {
    let port = find_available_port();
    let _server = start_echo_server(format!("127.0.0.1:{}", port));
    wait_for_port(port);

    let mut param = Packager::new();
    param.push_string(b"packed").unwrap();
    let mut arr = Packager::start_array(1).unwrap();
    arr.push_packager(&param).unwrap();
    let packet = client::build_request_packet("echo", Some(arr), true).unwrap();

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();
    client.set_persistent(true);
    let raw = client.call_raw(&packet).unwrap();
    assert_eq!(
        &raw[..PACKAGER_NAME.len() - 1],
        &PACKAGER_NAME[..PACKAGER_NAME.len() - 1]
    );
    let resp = YarResponse::unpack(&raw, PACKAGER_NAME.len()).unwrap();
    assert_eq!(resp.get_status(), 0);
    assert_eq!(
        resp.get_response().unwrap().as_array().unwrap()[0].as_bytes(),
        Some(b"packed".as_ref())
    );
}
