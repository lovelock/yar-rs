//! `yar-server-rust` — standalone Yar server offering an `echo` method.
//!
//! CLI (all options optional, defaults shown):
//!   --host <addr>     bind address                   (default 127.0.0.1:9501)
//!   --inflight <n>    max in-flight connections, 0 = unlimited (default 0)
//!   --max-request-size <size>  b/k/m suffix, bare = bytes   (default 8m)
//!   --read-timeout <dur>       s/ms suffix, bare = seconds  (default 5s)

use std::process::ExitCode;
use std::time::Duration;

use yar_rs::pack::Packager;
use yar_rs::profiler::{parse_duration_suffix, parse_size_suffix};
use yar_rs::response::YarResponse;
use yar_rs::server::YarServer;

const USAGE: &str = "\
Usage: yar-server-rust [options]

Options:
  --host <addr>              Bind address (default: 127.0.0.1:9501)
  --inflight <n>             Max in-flight connections, 0 = unlimited (default: 0)
  --max-request-size <size>  Max request body bytes: b/k/m suffix allowed (default: 8m)
  --read-timeout <dur>       Per-request read timeout: s/ms suffix allowed (default: 5s)";

struct Options {
    host: String,
    inflight: usize,
    max_request_size: usize,
    read_timeout: Duration,
}

fn parse_args(argv: &[String]) -> Result<Options, String> {
    let mut host: String = "127.0.0.1:9501".into();
    let mut inflight = 0;
    let mut max_request_size = parse_size_suffix("8m").expect("valid default");
    let mut read_timeout = parse_duration_suffix("5s").expect("valid default");
    let mut i = 0;
    while i < argv.len() {
        let (flag, value) = match argv[i].as_str() {
            f @ ("--host" | "--inflight" | "--max-request-size" | "--read-timeout") => (
                f,
                argv.get(i + 1).ok_or_else(|| format!("missing value for {f}"))?,
            ),
            other => return Err(format!("unknown argument: {other}")),
        };
        match (flag, value) {
            ("--host", v) => host = v.clone(),
            ("--inflight", v) => {
                inflight = v.parse().map_err(|_| format!("invalid --inflight value: {v}"))?;
            }
            ("--max-request-size", v) => {
                max_request_size =
                    parse_size_suffix(v).map_err(|e| format!("--max-request-size: {e}"))?;
            }
            ("--read-timeout", v) => {
                read_timeout =
                    parse_duration_suffix(v).map_err(|e| format!("--read-timeout: {e}"))?;
            }
            _ => unreachable!("flag was matched in the use-list above"),
        }
        i += 2;
    }
    Ok(Options {
        host,
        inflight,
        max_request_size,
        read_timeout,
    })
}

async fn serve(opts: Options) -> Result<(), String> {
    let mut server = YarServer::new(&opts.host);
    server.set_max_request_size(opts.max_request_size);
    server.set_max_inflight(opts.inflight);
    server.set_read_timeout(opts.read_timeout);

    server
        .register_handler("echo", |req| {
            // Echo semantics: return the whole parsed parameter tree, or
            // null when the request carried no parameters. Never unwrap:
            // a push failure becomes an error response.
            let mut pk = Packager::new();
            let pushed = match req.get_parameters() {
                Some(p) => pk.push_data(p),
                None => pk.push_null(),
            };
            match pushed {
                Ok(()) => YarResponse::success(req.id(), pk),
                Err(e) => YarResponse::error(req.id(), 1, &e.to_string()),
            }
        })
        .await;

    server.run().await.map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("yar-server-rust: {e}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let runtime = tokio::runtime::Runtime::new();
    let result = match runtime {
        Ok(rt) => rt.block_on(serve(opts)),
        Err(e) => Err(format!("failed to start runtime: {e}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS, // unreachable: the accept loop never returns
        Err(e) => {
            eprintln!("yar-server-rust: {e}");
            ExitCode::FAILURE
        }
    }
}
