//! Cross-language demo server for yar-rs.
//!
//! Registers three RPC methods:
//!   - "echo"  -> returns the string "echo:ok"
//!   - "add"   -> takes two integer params, returns their sum
//!   - "greet" -> takes a string param, returns "Hello, {name}!"
//!
//! Run:
//!   cargo run --example cross_server -- -S 127.0.0.1:9501

use yar_rs::{Packager, YarRequest, YarResponse, YarServer};

fn parse_server_addr(args: &[String]) -> String {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-S" && i + 1 < args.len() {
            return args[i + 1].clone();
        }
        i += 1;
    }
    "127.0.0.1:9501".to_string()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = parse_server_addr(&args);

    let server = YarServer::new(&addr);

    // "echo" — no params, returns "echo:ok"
    server
        .register_handler("echo", |req: &YarRequest| -> YarResponse {
            let mut pk = Packager::new();
            let _ = pk.push_string(b"echo:ok");
            YarResponse::success(req.id(), pk)
        })
        .await;

    // "add" — two integer params, returns their sum
    server
        .register_handler("add", |req: &YarRequest| -> YarResponse {
            let params = req.get_parameters();
            let arr = params.and_then(|p| p.as_array()).unwrap_or(&[]);

            let a = arr.first().and_then(|v| v.as_integer()).unwrap_or(0);
            let b = arr.get(1).and_then(|v| v.as_integer()).unwrap_or(0);
            let sum = a + b;

            let mut pk = Packager::new();
            let _ = pk.push_long(sum);
            YarResponse::success(req.id(), pk)
        })
        .await;

    // "greet" — one string param, returns "Hello, {name}!"
    server
        .register_handler("greet", |req: &YarRequest| -> YarResponse {
            let name = req
                .get_parameters()
                .and_then(|p| p.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .unwrap_or("World");

            let greeting = format!("Hello, {}!", name);
            let mut pk = Packager::new();
            let _ = pk.push_string(greeting.as_bytes());
            YarResponse::success(req.id(), pk)
        })
        .await;

    println!("Yar server listening on {}", addr);
    println!("Registered methods: echo, add, greet");

    if let Err(e) = server.run().await {
        eprintln!("Server error: {}", e);
        std::process::exit(1);
    }
}
