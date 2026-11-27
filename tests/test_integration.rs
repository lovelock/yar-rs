//! Full client-server integration tests.
//!
//! Starts a Rust YarServer on a random port, connects with a Rust YarClient,
//! and verifies the complete RPC roundtrip.

use std::time::Duration;
use yar_rs::*;

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
    setup: impl FnOnce(&YarServer) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>
        + Send
        + 'static,
) -> std::thread::JoinHandle<()> {
    let addr = format!("127.0.0.1:{}", port);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = YarServer::new(&addr);
            setup(&server).await;
            let _ = server.run().await;
        });
    })
}

// ---------------------------------------------------------------------------
// Echo handler test
// ---------------------------------------------------------------------------

#[test]
fn test_integration_echo() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server
                .register_handler("echo", |req| {
                    // Echo back the method name as a string
                    let mut pk = Packager::new();
                    pk.push_string(b"echo:ok").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    // Wait for server to start
    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));

    let response = client.call("echo", vec![]).unwrap();
    assert_eq!(response.get_status(), 0);
    let retval = response.get_response().unwrap();
    assert_eq!(retval.as_str(), Some("echo:ok"));

    client.close();
}

// ---------------------------------------------------------------------------
// Add handler test
// ---------------------------------------------------------------------------

#[test]
fn test_integration_add() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server
                .register_handler("add", |req| {
                    let params = req.get_parameters();
                    let arr = params.and_then(|p| p.as_array()).unwrap_or(&[]);
                    let sum: i64 = arr.iter().filter_map(|v| v.as_integer()).sum();

                    let mut pk = Packager::new();
                    pk.push_long(sum).unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));

    // Build params: [3, 4]
    let mut p1 = Packager::new();
    p1.push_long(3).unwrap();
    let mut p2 = Packager::new();
    p2.push_long(4).unwrap();

    let response = client.call("add", vec![p1, p2]).unwrap();
    assert_eq!(response.get_status(), 0);
    let retval = response.get_response().unwrap();
    assert_eq!(retval.as_integer(), Some(7));

    client.close();
}

// ---------------------------------------------------------------------------
// Error handler test
// ---------------------------------------------------------------------------

#[test]
fn test_integration_error_handler() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server
                .register_handler("error", |req| {
                    YarResponse::error(req.id(), 42, "intentional error")
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));

    let response = client.call("error", vec![]).unwrap();
    assert_eq!(response.get_status(), 42);
    assert_eq!(response.get_error(), Some("intentional error"));
    assert!(response.get_response().is_none());

    client.close();
}

// ---------------------------------------------------------------------------
// Unknown method error handling
// ---------------------------------------------------------------------------

#[test]
fn test_integration_unknown_method() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |_server| {
        Box::pin(async move {
            // Register no handlers - any method call should fail
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));

    let response = client.call("nonexistent", vec![]).unwrap();
    assert_ne!(response.get_status(), 0);
    let err = response.get_error().unwrap();
    assert!(
        err.contains("Method not found"),
        "error should mention 'Method not found', got: {}",
        err
    );

    client.close();
}

// ---------------------------------------------------------------------------
// Persistent connections: multiple calls on one connection
// ---------------------------------------------------------------------------

#[test]
fn test_integration_persistent_connection() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server
                .register_handler("counter", |req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"counted").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));
    client.set_persistent(true);

    // First call
    let resp1 = client.call("counter", vec![]).unwrap();
    assert_eq!(resp1.get_status(), 0);
    assert_eq!(resp1.get_response().unwrap().as_str(), Some("counted"));

    // Second call on same persistent connection
    let resp2 = client.call("counter", vec![]).unwrap();
    assert_eq!(resp2.get_status(), 0);
    assert_eq!(resp2.get_response().unwrap().as_str(), Some("counted"));

    // Third call
    let resp3 = client.call("counter", vec![]).unwrap();
    assert_eq!(resp3.get_status(), 0);

    client.close();
}

// ---------------------------------------------------------------------------
// Multiple handlers registered
// ---------------------------------------------------------------------------

#[test]
fn test_integration_multiple_handlers() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server
                .register_handler("hello", |req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"world").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;

            server
                .register_handler("ping", |req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"pong").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;

            server
                .register_handler("double", |req| {
                    let params = req.get_parameters();
                    let arr = params.and_then(|p| p.as_array()).unwrap_or(&[]);
                    let val = arr.first().and_then(|v| v.as_integer()).unwrap_or(0);

                    let mut pk = Packager::new();
                    pk.push_long(val * 2).unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));
    client.set_persistent(true);

    // Call "hello"
    let resp = client.call("hello", vec![]).unwrap();
    assert_eq!(resp.get_response().unwrap().as_str(), Some("world"));

    // Call "ping"
    let resp = client.call("ping", vec![]).unwrap();
    assert_eq!(resp.get_response().unwrap().as_str(), Some("pong"));

    // Call "double" with param [21]
    let mut p = Packager::new();
    p.push_long(21).unwrap();
    let resp = client.call("double", vec![p]).unwrap();
    assert_eq!(resp.get_response().unwrap().as_integer(), Some(42));

    client.close();
}

// ---------------------------------------------------------------------------
// Complex return value (nested structures)
// ---------------------------------------------------------------------------

#[test]
fn test_integration_complex_retval() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server
                .register_handler("complex", |req| {
                    // Return: {"status": "ok", "items": [1, 2, 3]}
                    let mut inner_arr = Packager::start_array(3).unwrap();
                    inner_arr.push_long(1).unwrap();
                    inner_arr.push_long(2).unwrap();
                    inner_arr.push_long(3).unwrap();

                    let mut retval = Packager::start_map(2).unwrap();
                    retval.push_string(b"status").unwrap();
                    retval.push_string(b"ok").unwrap();
                    retval.push_string(b"items").unwrap();
                    retval.push_packager(&inner_arr).unwrap();

                    YarResponse::success(req.id(), retval)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));

    let resp = client.call("complex", vec![]).unwrap();
    assert_eq!(resp.get_status(), 0);

    let rv = resp.get_response().unwrap();
    let map = rv.as_map().unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map[0].0.as_str(), Some("status"));
    assert_eq!(map[0].1.as_str(), Some("ok"));
    assert_eq!(map[1].0.as_str(), Some("items"));

    let items = map[1].1.as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].as_ulong(), Some(1));
    assert_eq!(items[1].as_ulong(), Some(2));
    assert_eq!(items[2].as_ulong(), Some(3));

    client.close();
}

// ---------------------------------------------------------------------------
// Client connection error handling
// ---------------------------------------------------------------------------

#[test]
fn test_integration_connect_refused() {
    // Try to connect to a port with no server
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let result = YarClient::connect(&addr);
    assert!(result.is_err(), "should fail to connect to unused port");
}

#[test]
fn test_integration_http_url_rejected() {
    let result = YarClient::connect("http://localhost:8080");
    assert!(result.is_err());

    let result = YarClient::connect("https://localhost:443");
    assert!(result.is_err());
}

#[test]
fn test_integration_no_port_rejected() {
    let result = YarClient::connect("localhost");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Non-persistent mode: each call uses a new connection
// ---------------------------------------------------------------------------

#[test]
fn test_integration_non_persistent_multiple_connections() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            server
                .register_handler("test", |req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"ok").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    // First connection
    {
        let mut client = YarClient::connect(&addr).unwrap();
        client.set_timeout(Duration::from_secs(5));
        let resp = client.call("test", vec![]).unwrap();
        assert_eq!(resp.get_status(), 0);
        // Client drops here, closing the connection
    }

    // Second connection (new TCP connection)
    {
        let mut client = YarClient::connect(&addr).unwrap();
        client.set_timeout(Duration::from_secs(5));
        let resp = client.call("test", vec![]).unwrap();
        assert_eq!(resp.get_status(), 0);
    }
}

#[test]
fn test_server_echoes_the_request_id() {
    let port = find_available_port();
    let addr = format!("127.0.0.1:{}", port);

    let _server_thread = start_server(port, |server| {
        Box::pin(async move {
            // Deliberately return id 0: the server must overwrite it with the
            // request id, exactly as the C server does.
            server
                .register_handler("legacy", |_req| {
                    let mut pk = Packager::new();
                    pk.push_string(b"ok").unwrap();
                    YarResponse::success(0, pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&addr).unwrap();
    client.set_timeout(Duration::from_secs(5));

    let resp = client.call("legacy", vec![]).unwrap();
    assert_eq!(resp.get_status(), 0);
    assert_eq!(resp.id, 1000);
    assert_eq!(resp.get_response().unwrap().as_str(), Some("ok"));

    client.close();
}
