//! Cross-language demo client for yar-rs.
//!
//! Connects to a Yar server and calls:
//!   - "echo"  (no params)   -> expects "echo:ok"
//!   - "add"   (10, 20)      -> expects 30
//!   - "greet" ("Yar")       -> expects "Hello, Yar!"
//!
//! Run:
//!   cargo run --example cross_client -- -h 127.0.0.1:9501 -n 1 -p

use yar_rs::{Packager, YarClient};

struct Args {
    host: String,
    rounds: usize,
    persistent: bool,
}

fn parse_args(argv: &[String]) -> Args {
    let mut host = "127.0.0.1:9501".to_string();
    let mut rounds: usize = 1;
    let mut persistent = true;
    let mut i = 1; // skip argv[0]
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" if i + 1 < argv.len() => {
                host = argv[i + 1].clone();
                i += 2;
            }
            "-n" if i + 1 < argv.len() => {
                rounds = argv[i + 1].parse().unwrap_or(1);
                i += 2;
            }
            "-p" => {
                persistent = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    Args {
        host,
        rounds,
        persistent,
    }
}

fn run_round(client: &mut YarClient, round: usize, total: usize) {
    let prefix = if total > 1 {
        format!("[round {}/{}] ", round, total)
    } else {
        String::new()
    };

    // 1. echo
    match client.call("echo", vec![]) {
        Ok(resp) => {
            if resp.get_status() == 0 {
                let val = resp
                    .get_response()
                    .and_then(|d| d.as_str())
                    .unwrap_or("<null>");
                println!("{}[CALL echo] => {}", prefix, val);
            } else {
                println!(
                    "{}[CALL echo] ERROR status={} err={:?}",
                    prefix,
                    resp.get_status(),
                    resp.get_error()
                );
            }
        }
        Err(e) => println!("{}[CALL echo] FAILED: {}", prefix, e),
    }

    // 2. add(10, 20)
    let mut pk_a = Packager::new();
    let _ = pk_a.push_long(10);
    let mut pk_b = Packager::new();
    let _ = pk_b.push_long(20);

    match client.call("add", vec![pk_a, pk_b]) {
        Ok(resp) => {
            if resp.get_status() == 0 {
                let val = resp
                    .get_response()
                    .and_then(|d| d.as_integer())
                    .unwrap_or(0);
                println!("{}[CALL add 10+20] => {}", prefix, val);
            } else {
                println!(
                    "{}[CALL add] ERROR status={} err={:?}",
                    prefix,
                    resp.get_status(),
                    resp.get_error()
                );
            }
        }
        Err(e) => println!("{}[CALL add] FAILED: {}", prefix, e),
    }

    // 3. greet("Yar")
    let mut pk_name = Packager::new();
    let _ = pk_name.push_string(b"Yar");

    match client.call("greet", vec![pk_name]) {
        Ok(resp) => {
            if resp.get_status() == 0 {
                let val = resp
                    .get_response()
                    .and_then(|d| d.as_str())
                    .unwrap_or("<null>");
                println!("{}[CALL greet \"Yar\"] => {}", prefix, val);
            } else {
                println!(
                    "{}[CALL greet] ERROR status={} err={:?}",
                    prefix,
                    resp.get_status(),
                    resp.get_error()
                );
            }
        }
        Err(e) => println!("{}[CALL greet] FAILED: {}", prefix, e),
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let args = parse_args(&argv);

    println!(
        "Connecting to {} (persistent={}, rounds={})",
        args.host, args.persistent, args.rounds
    );

    for round in 1..=args.rounds {
        let mut client = match YarClient::connect(&args.host) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to connect to {}: {}", args.host, e);
                std::process::exit(1);
            }
        };
        client.set_persistent(args.persistent);

        run_round(&mut client, round, args.rounds);

        client.close();
    }
}
