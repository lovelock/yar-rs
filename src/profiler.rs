//! Helpers shared by the profiler tooling (CLI arguments, load generation).
//!
//! This module holds parsing utilities used by the profiler binaries so the
//! bins stay thin. The load generator itself (Task 4) will build on this
//! module as well.

use std::time::{Duration, SystemTime};

use crate::client::YarClient;
use crate::error::{YarError, YarResult};
use crate::pack::Packager;
use crate::response::YarResponse;

/// Split `s` at the start of its trailing ASCII-alphabetic suffix.
///
/// Returns `(digits, suffix)` where `suffix` may be empty (bare number).
/// Returns `None` when `s` is empty or contains no non-alphabetic character
/// at all (e.g. `"ms"` alone).
fn split_numeric_tail(s: &str) -> Option<(&str, &str)> {
    let last_non_alpha = s.rfind(|c: char| !c.is_ascii_alphabetic())?;
    let digits_end = last_non_alpha + 1;
    Some((&s[..digits_end], &s[digits_end..]))
}

/// Parse a size string such as `"256"`, `"256b"`, `"1k"` or `"8m"`.
///
/// A bare number is bytes. Suffixes are case-insensitive:
/// `b` = 1, `k` = 1024, `m` = 1024 * 1024.
///
/// Errors on empty input, unknown suffixes, non-numeric components and
/// overflow (checked at parse and at multiplication).
pub fn parse_size_suffix(s: &str) -> Result<usize, String> {
    let (digits, suffix) =
        split_numeric_tail(s).ok_or_else(|| format!("invalid size '{}': empty", s))?;

    let multiplier: usize = match suffix {
        "" => 1,
        "b" | "B" => 1,
        "k" | "K" => 1024,
        "m" | "M" => 1024 * 1024,
        _ => return Err(format!("invalid size '{}': unknown suffix '{}'", s, suffix)),
    };

    if digits.is_empty() {
        return Err(format!("invalid size '{}': missing number", s));
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid size '{}': not a number", s));
    }

    let n: usize = digits
        .parse()
        .map_err(|_| format!("invalid size '{}': too large", s))?;
    n.checked_mul(multiplier)
        .ok_or_else(|| format!("invalid size '{}': too large", s))
}

/// Parse a duration string such as `"5"`, `"5s"`, `"250ms"` or `"2MS"`.
///
/// A bare number is seconds. Supported suffixes (case-insensitive): `s`
/// (seconds) and `ms` (milliseconds).
///
/// Errors on empty input, unknown suffixes, non-numeric components and
/// overflow.
pub fn parse_duration_suffix(s: &str) -> Result<Duration, String> {
    let (digits, suffix) =
        split_numeric_tail(s).ok_or_else(|| format!("invalid duration '{}': empty", s))?;

    if digits.is_empty() {
        return Err(format!("invalid duration '{}': missing number", s));
    }
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("invalid duration '{}': too large", s))?;

    match suffix {
        "" | "s" | "S" => Ok(Duration::from_secs(n)),
        "ms" | "MS" | "Ms" | "mS" => Ok(Duration::from_millis(n)),
        _ => Err(format!(
            "invalid duration '{}': unknown suffix '{}'",
            s, suffix
        )),
    }
}

/// Return the `pct`-percentile of an ascending slice with linear
/// interpolation (k6-style), i.e. value at continuous rank
/// `(pct / 100) * (len - 1)` between its neighbouring elements.
///
/// `sorted` must be in ascending order (this is not checked). `pct` is
/// clamped to [0, 100]. An empty slice yields `0.0`; a single-element
/// slice yields that element.
pub fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pct = pct.clamp(0.0, 100.0);
    let rank = (pct / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if upper == lower {
        return sorted[lower];
    }
    let frac = rank - lower as f64;
    sorted[lower] * (1.0 - frac) + sorted[upper] * frac
}

// -- Load generation ----------------------------------------------------------

/// Upper bound for a single request body: 4 MiB. A configured stage whose
/// `body_size` exceeds this value makes `run_profile` fail up front.
pub const MAX_BODY: usize = 4 * 1024 * 1024;

/// Default body-size ladder in bytes (256 B .. 4 M, ~4x steps).
pub const DEFAULT_SIZES: [usize; 10] = [
    256, 1_024, 4_096, 16_384, 65_536, 262_144, 524_288, 1_048_576, 2_097_152, 4_194_304,
];

/// Default VU (virtual user / concurrent connection) ladder.
pub const DEFAULT_VUS: [usize; 7] = [10, 20, 40, 80, 160, 320, 640];

/// Per-call socket timeout so a wedged server cannot hang a VU forever.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Pause between consecutive stages, letting the previous stage's
/// connections fully settle and the server drain. Not applied after the
/// final stage.
const QUIET_PERIOD: Duration = Duration::from_secs(1);

/// configuration of one profiling run.
pub struct ProfilerConfig {
    pub host: String,
    pub body_sizes: Vec<usize>,
    pub vus: Vec<usize>,
    pub step_time: Duration,
}

/// Result of one stage: one (body_size, vus) cell of the matrix.
pub struct StageReport {
    pub body_size: usize,
    pub vus: usize,
    /// Stage length in seconds (the configured step_time).
    pub duration_s: f64,
    /// Completed requests (ok + fail) per second.
    pub qps: f64,
    pub ok: u64,
    pub fail: u64,
    /// Latency percentiles of the OK samples, in milliseconds.
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub p999: f64,
    /// First failure text observed in the stage ("" when none).
    pub first_error: String,
    /// Stage wall-clock window in epoch seconds (SystemTime).
    pub start_ts: u64,
    pub end_ts: u64,
}

/// Run the full profiling matrix: every body size x every VU level, one
/// stage apiece, blocking until all of them are done.
///
/// # Errors
///
/// Returns an error before touching the network when configured body
/// sizes exceed [`MAX_BODY`], or when `body_sizes`/`vus` is empty or
/// contains a zero VU level.
pub fn run_profile(cfg: &ProfilerConfig) -> YarResult<Vec<StageReport>> {
    if cfg.body_sizes.is_empty() {
        return Err(YarError::Connection(
            "profiler: body_sizes must not be empty".into(),
        ));
    }
    if cfg.vus.is_empty() {
        return Err(YarError::Connection("profiler: vus must not be empty".into()));
    }
    if let Some(&v) = cfg.vus.iter().find(|&&v| v == 0) {
        return Err(YarError::Connection(format!("profiler: VU level {} is zero", v)));
    }
    if let Some(&s) = cfg.body_sizes.iter().find(|&&s| s > MAX_BODY) {
        return Err(YarError::Connection(format!(
            "profiler: body size {} exceeds MAX_BODY {}",
            s, MAX_BODY
        )));
    }

    let mut reports = Vec::new();
    for &body_size in &cfg.body_sizes {
        for &vus in &cfg.vus {
            // Quiet period before every stage but the first.
            if !reports.is_empty() {
                std::thread::sleep(QUIET_PERIOD);
            }
            reports.push(run_stage(cfg, body_size, vus));
        }
    }
    Ok(reports)
}

/// One stage: `vus` worker threads each holding their own persistent
/// `YarClient` and hammering `echo` until the stage deadline.
fn run_stage(cfg: &ProfilerConfig, body_size: usize, vus: usize) -> StageReport {
    let body = std::sync::Arc::new(make_body(body_size));
    let deadline = std::time::Instant::now() + cfg.step_time;
    let start_ts = epoch_secs();

    let (tx, rx) = std::sync::mpsc::channel::<ThreadOutcome>();
    let mut handles = Vec::with_capacity(vus);
    for _ in 0..vus {
        let tx = tx.clone();
        let body = std::sync::Arc::clone(&body);
        let host = cfg.host.clone();
        handles.push(std::thread::spawn(move || {
            let mut client = match YarClient::connect(&host) {
                Ok(c) => c,
                Err(e) => {
                    // A connection error is a failed sample (spec: failures
                    // include connection errors), not an empty stage.
                    let _ = tx.send(ThreadOutcome {
                        ok: 0,
                        fail: 1,
                        latencies_ms: Vec::new(),
                        first_error: Some(format!("connect failed: {}", e)),
                    });
                    return;
                }
            };
            client.set_timeout(CALL_TIMEOUT);
            client.set_persistent(true);

            let mut outcome = ThreadOutcome {
                ok: 0,
                fail: 0,
                latencies_ms: Vec::new(),
                first_error: None,
            };
            while std::time::Instant::now() < deadline {
                // One echo parameter: the shared deterministic body.
                let mut p = Packager::new();
                if let Err(e) = p.push_string(&body) {
                    outcome.note_failure(format!("pack failed: {}", e));
                    continue;
                }
                let t0 = std::time::Instant::now();
                let result = client.call("echo", vec![p]);
                let latency_ms = t0.elapsed().as_secs_f64() * 1000.0;
                match validate_echo(&result, body_size, &body) {
                    Ok(()) => {
                        outcome.ok += 1;
                        outcome.latencies_ms.push(latency_ms);
                    }
                    Err(msg) => {
                        outcome.note_failure(msg);
                        // A transport failure leaves the persistent socket
                        // dead or out of sync. Reusing it would turn the rest
                        // of the stage into a microsecond-fast failure loop
                        // that inflates fail/qps and burns CPU; dial again,
                        // as a k6 VU would. A content/status failure is a
                        // server-side answer, so the connection stays.
                        if result.is_err() {
                            match YarClient::connect(&host) {
                                Ok(mut fresh) => {
                                    fresh.set_timeout(CALL_TIMEOUT);
                                    fresh.set_persistent(true);
                                    client = fresh;
                                }
                                Err(e) => {
                                    outcome
                                        .note_failure(format!("reconnect failed: {}", e));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            let _ = tx.send(outcome);
        }));
    }
    drop(tx);

    // Aggregate worker outcomes as they arrive.
    let mut agg = ThreadOutcome {
        ok: 0,
        fail: 0,
        latencies_ms: Vec::new(),
        first_error: None,
    };
    for _ in 0..vus {
        if let Ok(o) = rx.recv() {
            agg.ok += o.ok;
            agg.fail += o.fail;
            agg.latencies_ms.extend(o.latencies_ms);
            if agg.first_error.is_none() {
                agg.first_error = o.first_error;
            }
        }
    }
    for h in handles {
        let _ = h.join();
    }
    let end_ts = epoch_secs();

    agg.into_report(body_size, vus, cfg.step_time, start_ts, end_ts)
}

/// What one VU thread observed over its stage.
struct ThreadOutcome {
    ok: u64,
    fail: u64,
    latencies_ms: Vec<f64>,
    first_error: Option<String>,
}

impl ThreadOutcome {
    /// Record a failed sample, keeping only the first error text.
    fn note_failure(&mut self, msg: String) {
        self.fail += 1;
        if self.first_error.is_none() {
            self.first_error = Some(msg);
        }
    }

    /// Turn raw counters into a [`StageReport`].
    fn into_report(
        self,
        body_size: usize,
        vus: usize,
        step_time: Duration,
        start_ts: u64,
        end_ts: u64,
    ) -> StageReport {
        let mut latencies = self.latencies_ms;
        latencies.sort_by(f64::total_cmp);
        let duration_s = step_time.as_secs_f64();
        StageReport {
            body_size,
            vus,
            duration_s,
            qps: if duration_s > 0.0 {
                (self.ok + self.fail) as f64 / duration_s
            } else {
                0.0
            },
            ok: self.ok,
            fail: self.fail,
            p50: percentile(&latencies, 50.0),
            p95: percentile(&latencies, 95.0),
            p99: percentile(&latencies, 99.0),
            p999: percentile(&latencies, 99.9),
            first_error: self.first_error.unwrap_or_default(),
            start_ts,
            end_ts,
        }
    }
}

/// Validate one `echo` response: zero status, retval = one-element array
/// of bytes with the same length and matching head/tail against what was
/// sent (`body_size` < 32 compares the whole body).
fn validate_echo(
    result: &YarResult<YarResponse>,
    body_size: usize,
    body: &[u8],
) -> Result<(), String> {
    let response = match result {
        Ok(r) => r,
        Err(e) => return Err(format!("call failed: {}", e)),
    };
    if response.get_status() != 0 {
        return Err(format!(
            "non-zero status {} (error {:?})",
            response.get_status(),
            response.get_error()
        ));
    }
    let arr = response
        .get_response()
        .and_then(|d| d.as_array())
        .ok_or_else(|| "retval is not an array".to_string())?;
    if arr.len() != 1 {
        return Err(format!(
            "retval array has {} elements, expected 1",
            arr.len()
        ));
    }
    let echoed = arr[0]
        .as_bytes()
        .ok_or_else(|| "retval element is not bytes".to_string())?;
    verify_echo(body_size, body, echoed)
}

/// Echo-fidelity check for one sample: length must equal `body_size`;
/// bodies below 32 bytes are compared whole, larger ones only by their
/// first and last 16 bytes (the hot path must not scan the middle of a
/// 4 MiB body).
fn verify_echo(body_size: usize, sent: &[u8], echoed: &[u8]) -> Result<(), String> {
    if echoed.len() != body_size {
        return Err(format!(
            "echoed body is {} bytes, expected {}",
            echoed.len(),
            body_size
        ));
    }
    if body_size < 32 {
        if echoed == sent {
            return Ok(());
        }
        return Err("echoed body differs from what was sent".into());
    }
    if echoed[..16] != sent[..16] {
        return Err("echoed body: first 16 bytes differ".into());
    }
    if echoed[body_size - 16..] != sent[body_size - 16..] {
        return Err("echoed body: last 16 bytes differ".into());
    }
    Ok(())
}

/// Deterministic request body: byte `i` is `(i % 251) as u8`.
fn make_body(body_size: usize) -> Vec<u8> {
    (0..body_size).map(|i| (i % 251) as u8).collect()
}

/// Current time in epoch seconds (0 if the clock is before the epoch).
fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// -- CSV rendering ------------------------------------------------------------

/// CSV header: fixed 13-column contract shared with Task 8 run.sh.
pub const CSV_HEADER: &str = "size_bytes,vus,duration_s,qps,ok,fail,p50_ms,p95_ms,p99_ms,p999_ms,first_error,start_ts,end_ts";

/// Quote a CSV field when it contains comma, double-quote, CR or LF,
/// doubling any embedded double quotes.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Write stage reports as CSV (`CSV_HEADER` then one line per stage,
/// each terminated by `\n`). The body size column is named `size_bytes`.
pub fn write_csv(reports: &[StageReport], out: &mut dyn std::io::Write) -> std::io::Result<()> {
    writeln!(out, "{}", CSV_HEADER)?;
    for r in reports {
        writeln!(
            out,
            "{},{},{:.3},{:.2},{},{},{:.3},{:.3},{:.3},{:.3},{},{},{}",
            r.body_size,
            r.vus,
            r.duration_s,
            r.qps,
            r.ok,
            r.fail,
            r.p50,
            r.p95,
            r.p99,
            r.p999,
            csv_field(&r.first_error),
            r.start_ts,
            r.end_ts,
        )?;
    }
    Ok(())
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- percentile -------------------------------------------------------------

    #[test]
    fn percentile_ramp_matches_linear_interpolation() {
        let v: Vec<f64> = (1..=1000).map(|x| x as f64).collect();
        assert!((percentile(&v, 50.0) - 500.5).abs() < 1e-9);
        assert!((percentile(&v, 95.0) - 950.05).abs() < 1e-9);
        assert!((percentile(&v, 99.0) - 990.01).abs() < 1e-9);
        assert!((percentile(&v, 99.9) - 999.001).abs() < 1e-9);
    }

    #[test]
    fn percentile_clamps_out_of_range() {
        let v = [1.0, 2.0, 3.0];
        assert_eq!(percentile(&v, 150.0), 3.0);
        assert_eq!(percentile(&v, -1.0), 1.0);
    }

    #[test]
    fn percentile_empty_and_single() {
        assert_eq!(percentile(&[], 99.0), 0.0);
        assert_eq!(percentile(&[7.5], 99.0), 7.5);
    }

    // -- echo fidelity ---------------------------------------------------------

    #[test]
    fn verify_echo_large_body_ignores_middle_bytes() {
        let body = make_body(65_536);
        let mut echoed = body.clone();
        // Middle byte differs, ends are identical: still accepted.
        echoed[32_000] ^= 0xff;
        assert_eq!(verify_echo(65_536, &body, &echoed), Ok(()));
    }

    #[test]
    fn verify_echo_large_body_tail_mismatch_is_rejected() {
        let body = make_body(65_536);
        let mut echoed = body.clone();
        echoed[65_535] ^= 0xff;
        let err = verify_echo(65_536, &body, &echoed).unwrap_err();
        assert!(err.contains("last 16 bytes"), "got: {err}");
    }

    #[test]
    fn verify_echo_small_body_compares_whole() {
        let body = make_body(16);
        assert_eq!(verify_echo(16, &body, &body), Ok(()));
        // One middle byte differs: whole-body comparison must reject.
        let mut echoed = body.clone();
        echoed[7] ^= 0xff;
        let err = verify_echo(16, &body, &echoed).unwrap_err();
        assert!(err.contains("differs"), "got: {err}");
    }

    #[test]
    fn verify_echo_length_mismatch_is_rejected() {
        let body = make_body(64);
        let (short, long) = (&body[..63], &body[..64]);
        let err1 = verify_echo(64, &body, short).unwrap_err();
        assert!(err1.contains("63 bytes, expected 64"), "got: {err1}");
        let err2 = verify_echo(64, &body, [long, long].concat().as_slice()).unwrap_err();
        assert!(err2.contains("128 bytes, expected 64"), "got: {err2}");
    }

    // -- sizes ----------------------------------------------------------------

    #[test]
    fn parse_size_bare_number_is_bytes() {
        assert_eq!(parse_size_suffix("256").unwrap(), 256);
        assert_eq!(parse_size_suffix("0").unwrap(), 0);
    }

    #[test]
    fn parse_size_b_suffix() {
        assert_eq!(parse_size_suffix("256b").unwrap(), 256);
        assert_eq!(parse_size_suffix("256B").unwrap(), 256);
    }

    #[test]
    fn parse_size_k_suffix() {
        assert_eq!(parse_size_suffix("1k").unwrap(), 1024);
        assert_eq!(parse_size_suffix("1K").unwrap(), 1024);
    }

    #[test]
    fn parse_size_m_suffix() {
        assert_eq!(parse_size_suffix("8m").unwrap(), 8 * 1024 * 1024);
        assert_eq!(parse_size_suffix("8M").unwrap(), 8 * 1024 * 1024);
    }

    #[test]
    fn parse_size_rejects_empty() {
        assert!(parse_size_suffix("").is_err());
    }

    #[test]
    fn parse_size_rejects_unknown_suffix() {
        assert!(parse_size_suffix("512x").is_err());
        assert!(parse_size_suffix("512g").is_err());
    }

    #[test]
    fn parse_size_rejects_suffix_without_number() {
        assert!(parse_size_suffix("m").is_err());
        assert!(parse_size_suffix("kb").is_err());
    }

    #[test]
    fn parse_size_rejects_non_numeric() {
        assert!(parse_size_suffix("12x3k").is_err());
        assert!(parse_size_suffix("abc").is_err());
    }

    #[test]
    fn parse_size_rejects_overflow() {
        // Parse overflow: exceeds usize range.
        assert!(parse_size_suffix("99999999999999999999").is_err());
        // Multiplication overflow: parses, but exceeds usize when scaled.
        assert!(parse_size_suffix("17863391866455421m").is_err());
    }

    // -- durations ------------------------------------------------------------

    #[test]
    fn parse_duration_bare_number_is_seconds() {
        assert_eq!(parse_duration_suffix("5").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn parse_duration_seconds_suffix() {
        assert_eq!(parse_duration_suffix("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_duration_suffix("5S").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn parse_duration_milliseconds_suffix() {
        assert_eq!(
            parse_duration_suffix("250ms").unwrap(),
            Duration::from_millis(250)
        );
        assert_eq!(
            parse_duration_suffix("250MS").unwrap(),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn parse_duration_rejects_empty() {
        assert!(parse_duration_suffix("").is_err());
    }

    #[test]
    fn parse_duration_rejects_unknown_suffix() {
        assert!(parse_duration_suffix("5x").is_err());
        assert!(parse_duration_suffix("5m").is_err());
        assert!(parse_duration_suffix("5us").is_err());
    }

    #[test]
    fn parse_duration_rejects_suffix_without_number() {
        assert!(parse_duration_suffix("s").is_err());
        assert!(parse_duration_suffix("ms").is_err());
    }

    #[test]
    fn parse_duration_rejects_non_numeric() {
        assert!(parse_duration_suffix("5x7s").is_err());
        assert!(parse_duration_suffix("abc").is_err());
    }
}
