//! `profiler` — k6-style VU/body-matrix load generator CLI.
//!
//! Runs every (body size, VU) stage against a single Yar server for a
//! fixed step time and writes one CSV line per stage (14 columns).
//!
//! Container/target orchestration (spinning up the server, resource
//! limits, cgroup sampling) lives in `profile/run.sh` (Task 8): this CLI
//! takes only `--host` and profiles whatever is listening there.
//!
//! CLI (all options optional, defaults shown):
//!   --host `<addr>`     target Yar server                     (default 127.0.0.1:9501)
//!   --vus `<csv>`       VU ladder, bare numbers, comma-list    (default 10,20,40,80,160,320,640)
//!   --sizes `<csv>`     body ladder, b/k/m suffix allowed      (default 0,256b,...,4m)
//!   --step-time `<dur>` per-stage measured duration, s/ms      (default 10s)
//!   --warmup `<dur>`    per-stage warmup before measuring      (default 2s)
//!   --out `<path>`      CSV destination                        (default: stdout)

use std::fs::File;
use std::process::ExitCode;
use std::time::Duration;

use yar_rs::profiler::{
    parse_duration_suffix, parse_size_suffix, run_profile, write_csv, ProfilerConfig,
    DEFAULT_SIZES, DEFAULT_VUS,
};

const USAGE: &str = "\
Usage: profiler [options]

Options:
  --host <addr>      Target Yar server as host:port (default: 127.0.0.1:9501)
  --vus <csv>        Comma-separated VU ladder (default: 10,20,40,80,160,320,640)
  --sizes <csv>      Comma-separated body ladder: b/k/m suffix (default: 0,256b,1k,...,4m)
  --step-time <dur>  Per-stage measured duration: s/ms suffix (default: 10s)
  --warmup <dur>     Per-stage warmup before measuring: s/ms suffix (default: 2s)
  --out <path>       Write CSV to this path instead of stdout";

#[derive(Debug)]
struct Args {
    host: String,
    vus: Vec<usize>,
    sizes: Vec<usize>,
    step_time: Duration,
    warmup: Duration,
    out: Option<String>,
}

/// Parse `--vus` as a comma-separated list of bare VU counts.
fn parse_vus_csv(s: &str) -> Result<Vec<usize>, String> {
    let mut vus = Vec::new();
    for token in s.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(format!("invalid --vus '{}': empty item", s));
        }
        vus.push(
            token
                .parse()
                .map_err(|_| format!("invalid --vus '{}': '{}' is not a number", s, token))?,
        );
    }
    Ok(vus)
}

/// Parse `--sizes` as a comma-separated list of sizes with b/k/m suffixes.
fn parse_sizes_csv(s: &str) -> Result<Vec<usize>, String> {
    let mut sizes = Vec::new();
    for token in s.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(format!("invalid --sizes '{}': empty item", s));
        }
        sizes.push(parse_size_suffix(token).map_err(|e| format!("--sizes: {}", e))?);
    }
    Ok(sizes)
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args {
        host: "127.0.0.1:9501".into(),
        vus: DEFAULT_VUS.to_vec(),
        sizes: DEFAULT_SIZES.to_vec(),
        step_time: parse_duration_suffix("10s").expect("valid default"),
        warmup: parse_duration_suffix("2s").expect("valid default"),
        out: None,
    };
    let mut i = 0;
    while i < argv.len() {
        let (flag, value) = match argv[i].as_str() {
            f @ ("--host" | "--vus" | "--sizes" | "--step-time" | "--warmup" | "--out") => (
                f,
                argv.get(i + 1)
                    .ok_or_else(|| format!("missing value for {f}"))?,
            ),
            other => return Err(format!("unknown argument: {other}")),
        };
        match (flag, value) {
            ("--host", v) => args.host = v.clone(),
            ("--vus", v) => args.vus = parse_vus_csv(v)?,
            ("--sizes", v) => args.sizes = parse_sizes_csv(v)?,
            ("--step-time", v) => {
                args.step_time =
                    parse_duration_suffix(v).map_err(|e| format!("--step-time: {e}"))?;
            }
            ("--warmup", v) => {
                args.warmup = parse_duration_suffix(v).map_err(|e| format!("--warmup: {e}"))?;
            }
            ("--out", v) => args.out = Some(v.clone()),
            _ => unreachable!("flag was matched in the use-list above"),
        }
        i += 2;
    }
    Ok(args)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("profiler: {e}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    let cfg = ProfilerConfig {
        host: args.host,
        body_sizes: args.sizes,
        vus: args.vus,
        step_time: args.step_time,
        warmup: args.warmup,
    };

    let reports = match run_profile(&cfg) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("profiler: {e}");
            return ExitCode::FAILURE;
        }
    };

    let result = match &args.out {
        Some(path) => File::create(path)
            .and_then(|mut f| write_csv(&reports, &mut f))
            .map_err(|e| e.to_string()),
        None => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            write_csv(&reports, &mut lock).map_err(|e| e.to_string())
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("profiler: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_match_the_documented_ladders() {
        let a = parse_args(&[]).unwrap();
        assert_eq!(a.host, "127.0.0.1:9501");
        assert_eq!(a.vus, DEFAULT_VUS.to_vec());
        assert_eq!(a.sizes, DEFAULT_SIZES.to_vec());
        assert_eq!(a.step_time, Duration::from_secs(10));
        assert_eq!(a.out, None);
    }

    #[test]
    fn explicit_overrides_win() {
        let a = parse_args(&argv(&[
            "--host",
            "10.0.0.5:9502",
            "--vus",
            "4,8",
            "--sizes",
            "1k,64k,1m",
            "--step-time",
            "250ms",
            "--out",
            "/tmp/report.csv",
        ]))
        .unwrap();
        assert_eq!(a.host, "10.0.0.5:9502");
        assert_eq!(a.vus, vec![4, 8]);
        assert_eq!(a.sizes, vec![1024, 65_536, 1_048_576]);
        assert_eq!(a.step_time, Duration::from_millis(250));
        assert_eq!(a.out, Some("/tmp/report.csv".to_string()));
    }

    #[test]
    fn unknown_arg_is_rejected() {
        let err = parse_args(&argv(&["--target", "rust"])).unwrap_err();
        assert!(err.contains("unknown argument"), "got: {err}");
    }

    #[test]
    fn missing_value_is_rejected() {
        let err = parse_args(&argv(&["--host"])).unwrap_err();
        assert!(err.contains("missing value"), "got: {err}");
    }

    #[test]
    fn bad_vus_are_rejected() {
        assert!(parse_args(&argv(&["--vus", "10,abc"])).is_err());
        assert!(parse_args(&argv(&["--vus", ""])).is_err());
        assert!(parse_args(&argv(&["--vus", "10,,"])).is_err());
    }

    #[test]
    fn bad_sizes_are_rejected() {
        // Unknown size suffix.
        assert!(parse_args(&argv(&["--sizes", "1k,2zz"])).is_err());
        assert!(parse_args(&argv(&["--sizes", ""])).is_err());
        assert!(parse_args(&argv(&["--sizes", "1g"])).is_err());
    }

    #[test]
    fn bad_step_time_is_rejected() {
        let err = parse_args(&argv(&["--step-time", "5x"])).unwrap_err();
        assert!(err.contains("--step-time"), "got: {err}");
    }

    #[test]
    fn sizes_accept_suffixes_and_bare_numbers() {
        let a = parse_args(&argv(&["--sizes", "256b,2K,8m,4096"])).unwrap();
        assert_eq!(a.sizes, vec![256, 2048, 8 * 1024 * 1024, 4096]);
    }
}
