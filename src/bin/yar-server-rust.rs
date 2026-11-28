//! `yar-server-rust` — standalone Yar server offering an `echo` method.
//!
//! CLI (all options optional, defaults shown):
//!   --host `<addr>`     bind address                   (default 127.0.0.1:9501)
//!   --inflight `<n>`    max in-flight connections, 0 = unlimited (default 1024)
//!   --max-request-size `<size>`  b/k/m suffix, bare = bytes   (default 10m)
//!   --read-timeout `<dur>`       s/ms suffix, bare = seconds  (default 3s, as C)
//!   --log-file `<path>`          log destination (default: stderr)
//!   --log-level `<level>`        okey|debug|notice|warning|error (default: notice)
//!   --no-access-log              disable the per-request OKEY access log

use std::process::ExitCode;
use std::time::Duration;

use yar_rs::cli::{parse_duration_suffix, parse_size_suffix};
use yar_rs::logger::LogLevel;
use yar_rs::pack::Packager;
use yar_rs::response::YarResponse;
use yar_rs::server::{
    YarServer, DEFAULT_MAX_INFLIGHT, DEFAULT_MAX_REQUEST_SIZE, SERVER_ERROR_STATUS,
};

const USAGE: &str = "\
Usage: yar-server-rust [options]

Options:
  --host <addr>              Bind address (default: 127.0.0.1:9501)
  --inflight <n>             Max in-flight connections, 0 = unlimited (default: 1024)
  --max-request-size <size>  Max request body bytes: b/k/m suffix allowed (default: 10m)
  --read-timeout <dur>       Per-request read timeout: s/ms suffix allowed (default: 3s)
  --log-file <path>          Log file path (default: stderr)
  --log-level <level>        okey|debug|notice|warning|error (default: notice)
  --no-access-log            Disable the per-request access log
  -h, --help                 Show this help";

struct Options {
    host: String,
    inflight: usize,
    max_request_size: usize,
    read_timeout: Duration,
    log_file: Option<String>,
    log_level: LogLevel,
    access_log: bool,
}

fn parse_log_level(s: &str) -> Result<LogLevel, String> {
    match s.to_ascii_lowercase().as_str() {
        "ok" | "okey" => Ok(LogLevel::Ok),
        "debug" => Ok(LogLevel::Debug),
        "notice" => Ok(LogLevel::Notice),
        "warn" | "warning" => Ok(LogLevel::Warning),
        "error" => Ok(LogLevel::Error),
        _ => Err(format!("invalid --log-level value: {s}")),
    }
}

fn parse_args(argv: &[String]) -> Result<Options, String> {
    let mut host: String = "127.0.0.1:9501".into();
    let mut inflight = DEFAULT_MAX_INFLIGHT;
    let mut max_request_size = DEFAULT_MAX_REQUEST_SIZE;
    let mut read_timeout = parse_duration_suffix("3s").expect("valid default");
    let mut log_file: Option<String> = None;
    let mut log_level = LogLevel::Notice;
    let mut access_log = true;

    let mut i = 0;
    while i < argv.len() {
        let flag = argv[i].as_str();
        if flag == "--no-access-log" {
            access_log = false;
            i += 1;
            continue;
        }
        if flag == "-h" || flag == "--help" {
            return Err(String::new());
        }
        let value = match flag {
            "--host" | "--inflight" | "--max-request-size" | "--read-timeout" | "--log-file"
            | "--log-level" => argv
                .get(i + 1)
                .ok_or_else(|| format!("missing value for {flag}"))?,
            other => return Err(format!("unknown argument: {other}")),
        };
        match flag {
            "--host" => host = value.clone(),
            "--inflight" => {
                inflight = value
                    .parse()
                    .map_err(|_| format!("invalid --inflight value: {value}"))?;
            }
            "--max-request-size" => {
                max_request_size =
                    parse_size_suffix(value).map_err(|e| format!("--max-request-size: {e}"))?;
            }
            "--read-timeout" => {
                read_timeout =
                    parse_duration_suffix(value).map_err(|e| format!("--read-timeout: {e}"))?;
            }
            "--log-file" => log_file = Some(value.clone()),
            "--log-level" => log_level = parse_log_level(value)?,
            _ => unreachable!("flag was matched in the use-list above"),
        }
        i += 2;
    }

    Ok(Options {
        host,
        inflight,
        max_request_size,
        read_timeout,
        log_file,
        log_level,
        access_log,
    })
}

async fn serve(opts: Options) -> Result<(), String> {
    let mut server = YarServer::new(&opts.host);
    server.set_max_request_size(opts.max_request_size);
    server.set_max_inflight(opts.inflight);
    server.set_read_timeout(opts.read_timeout);
    server.set_log_level(opts.log_level);
    server.set_access_log(opts.access_log);
    if let Some(path) = &opts.log_file {
        server.set_log_file(path);
    }

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
                Err(e) => YarResponse::error(req.id(), SERVER_ERROR_STATUS, &e.to_string()),
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
            if !e.is_empty() {
                eprintln!("yar-server-rust: {e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(if e.is_empty() { 0 } else { 2 });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_c() {
        let o = parse_args(&[]).unwrap();
        assert_eq!(o.max_request_size, 10 * 1024 * 1024);
        assert_eq!(o.read_timeout, Duration::from_secs(3));
        assert!(o.access_log);
        assert_eq!(o.log_level, LogLevel::Notice);
    }
}
