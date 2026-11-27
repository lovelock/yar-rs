//! Tests for the profiler library: percentile primitives, CSV rendering,
//! validation of `run_profile` configs, and an end-to-end load run against
//! the real `yar-server-rust` binary.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use yar_rs::profiler::{
    percentile, run_profile, write_csv, ProfilerConfig, StageReport, DEFAULT_SIZES, DEFAULT_VUS,
    MAX_BODY,
};

/// Approximate f64 equality helper for interpolated sample values.
fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{}: expected {}, got {}",
        what,
        expected,
        actual
    );
}

/// Small helper: 1..=1000 as f64, ascending (percentile expects sorted input).
fn ramp_1_to_1000() -> Vec<f64> {
    (1..=1000).map(|x| x as f64).collect()
}

/// Empty input yields 0.0 for every percentile level.
#[test]
fn test_percentile_empty_is_zero() {
    let empty: Vec<f64> = Vec::new();
    for pct in [0.0, 50.0, 95.0, 99.0, 99.9, 100.0] {
        assert_eq!(
            percentile(&empty, pct),
            0.0,
            "empty slice with pct {} must be 0.0",
            pct
        );
    }
}

/// With a single sample every percentile equals that sample.
#[test]
fn test_percentile_single_element() {
    let v = [1.0];
    for pct in [0.0, 50.0, 95.0, 99.0, 99.9, 100.0] {
        assert_eq!(
            percentile(&v, pct),
            1.0,
            "single-element slice with pct {} must be 1.0",
            pct
        );
    }
}

/// k6-style linear interpolation on a 1..=1000 ramp:
/// rank = pct/100 * (len-1), interpolated between floor/ceil neighbours.
#[test]
fn test_percentile_ramp_1_to_1000() {
    let v = ramp_1_to_1000();
    // p50:  rank 499.5  -> 500 + 0.5
    assert_close(percentile(&v, 50.0), 500.5, "p50");
    // p95:  rank 949.05 -> 950 + 0.05  (inside (949, 951))
    assert_close(percentile(&v, 95.0), 950.05, "p95");
    // p99:  rank 989.01 -> 990.01      (inside (990, 991])
    let p99 = percentile(&v, 99.0);
    assert!(
        p99 > 990.0 && p99 <= 991.0,
        "p99 must be in (990, 991], got {}",
        p99
    );
    // p99.9: rank 998.001 -> 999.001
    assert_close(percentile(&v, 99.9), 999.001, "p99.9");    // exact endpoints
    assert_eq!(percentile(&v, 0.0), 1.0);
    assert_eq!(percentile(&v, 100.0), 1000.0);
}

/// Interpolation between two elements of a two-element slice.
#[test]
fn test_percentile_two_element_interpolation() {
    let v = [1.0, 3.0];
    // rank 0.5 -> 1 + 0.5*(3-1) = 2
    assert_eq!(percentile(&v, 50.0), 2.0);
}

/// pct is clamped to [0, 100], so out-of-range values behave like the
/// nearest valid endpoint.
#[test]
fn test_percentile_clamps_pct() {
    let v = ramp_1_to_1000();
    assert_eq!(percentile(&v, 150.0), percentile(&v, 100.0));
    assert_eq!(percentile(&v, -10.0), percentile(&v, 0.0));
    assert_eq!(percentile(&v, 150.0), 1000.0);
    assert_eq!(percentile(&v, -10.0), 1.0);
}

// -- run_profile validation (no server needed) --------------------------------

fn valid_cfg() -> ProfilerConfig {
    ProfilerConfig {
        host: "127.0.0.1:1".to_string(),
        body_sizes: vec![8],
        vus: vec![2],
        step_time: Duration::from_millis(100),
    }
}

/// MAX_BODY is the enforced limit: samples of it must validate, anything
/// above must fail up front before any I/O happens.
#[test]
fn test_run_profile_body_size_validation() {
    // A body size above MAX_BODY is rejected.
    let cfg = ProfilerConfig {
        body_sizes: vec![MAX_BODY, MAX_BODY + 1],
        ..valid_cfg()
    };
    assert!(run_profile(&cfg).is_err(), "body > MAX_BODY must be Err");

    // MAX_BODY itself is accepted (validation-wise; a smaller check below
    // forces the Err path so no server is needed for this test).
    let cfg = ProfilerConfig {
        body_sizes: vec![MAX_BODY],
        vus: vec![],
        ..valid_cfg()
    };
    assert!(run_profile(&cfg).is_err(), "empty vus is Err");
}

#[test]
fn test_run_profile_rejects_empty_body_sizes() {
    let cfg = ProfilerConfig {
        body_sizes: vec![],
        ..valid_cfg()
    };
    assert!(run_profile(&cfg).is_err());
}

#[test]
fn test_run_profile_rejects_empty_vus() {
    let cfg = ProfilerConfig {
        vus: vec![],
        ..valid_cfg()
    };
    assert!(run_profile(&cfg).is_err());
}

#[test]
fn test_run_profile_rejects_zero_vus() {
    let cfg = ProfilerConfig {
        vus: vec![10, 0],
        ..valid_cfg()
    };
    assert!(run_profile(&cfg).is_err());
}

/// The default ladders are exposed and within the enforced body limit.
#[test]
fn test_default_ladders() {
    assert_eq!(MAX_BODY, 4 * 1024 * 1024);
    assert_eq!(DEFAULT_SIZES.len(), 10);
    assert_eq!(DEFAULT_VUS.len(), 7);
    for s in DEFAULT_SIZES {
        assert!(s <= MAX_BODY, "default size {} exceeds MAX_BODY", s);
    }
    for v in DEFAULT_VUS {
        assert!(v > 0, "default VU count {} must be positive", v);
    }
}

// -- write_csv ----------------------------------------------------------------

fn stage(body_size: usize, vus: usize, first_error: &str) -> StageReport {
    StageReport {
        body_size,
        vus,
        duration_s: 10.0,
        qps: 1234.567,
        ok: 12_300,
        fail: 2,
        p50: 1.5,
        p95: 3.25,
        p99: 4.0,
        p999: 5.0,
        first_error: first_error.to_string(),
        start_ts: 1700000000,
        end_ts: 1700000010,
    }
}

/// The CSV header is exactly the fixed 13-column contract.
#[test]
fn test_write_csv_header_is_fixed_contract() {
    let mut buf: Vec<u8> = Vec::new();
    write_csv(&[], &mut buf).expect("writing only a header must succeed");
    let text = String::from_utf8(buf).unwrap();
    let header = "size_bytes,vus,duration_s,qps,ok,fail,p50_ms,p95_ms,p99_ms,p999_ms,first_error,start_ts,end_ts";
    assert_eq!(text.lines().next(), Some(header));
    assert!(text.ends_with('\n'));
}

/// Numeric and timestamp fields are formatted with the pinned precisions,
/// and an empty first_error stays an unquoted empty field.
#[test]
fn test_write_csv_record_formatting() {
    let mut buf: Vec<u8> = Vec::new();
    write_csv(&[stage(1024, 10, "")], &mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();
    let mut lines = text.lines();
    lines.next().unwrap(); // skip header
    assert_eq!(
        lines.next(),
        Some("1024,10,10.000,1234.57,12300,2,1.500,3.250,4.000,5.000,,1700000000,1700000010")
    );
    assert_eq!(lines.next(), None);
}

/// first_error containing comma, double-quote, CR or LF is wrapped in
/// double quotes with internal quotes doubled.
#[test]
fn test_write_csv_escapes_first_error() {
    // Comma only.
    let mut buf: Vec<u8> = Vec::new();
    write_csv(&[stage(1, 1, "conn reset, retry")], &mut buf).unwrap();
    assert!(String::from_utf8(buf).unwrap().contains(",\"conn reset, retry\","));

    // Double quote -> doubled inside a quoted field.
    let mut buf: Vec<u8> = Vec::new();
    write_csv(&[stage(1, 1, "say \"hi\"")], &mut buf).unwrap();
    assert!(String::from_utf8(buf).unwrap().contains(",\"say \"\"hi\"\"\","));

    // Newline -> quoted; the raw LF stays as quoted data, so the buffer
    // contains header-\n + the embedded data-\n + the record-\n.
    let mut buf: Vec<u8> = Vec::new();
    write_csv(&[stage(1, 1, "a\nb")], &mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();
    assert_eq!(
        text.matches('\n').count(),
        3,
        "quoted data LF must be preserved, not escaped away: {:?}",
        text
    );
    assert!(text.contains(",\"a\nb\","));

    // CR alone also triggers quoting.
    let mut buf: Vec<u8> = Vec::new();
    write_csv(&[stage(1, 1, "a\rb")], &mut buf).unwrap();
    assert!(String::from_utf8(buf).unwrap().contains(",\"a\rb\","));
}

// -- end-to-end against the real echo server ----------------------------------

/// RAII guard: kills and reaps the child on drop, including on panics,
/// so a failing assertion never leaves the server process behind.
struct ServerGuard2 {
    child: Child,
}

impl Drop for ServerGuard2 {
    fn drop(&mut self) {
        if self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}

/// Spawn the server binary and poll-connect until the port is reachable
/// (deadline ~5s). Panics with an explicit message if the child exits early.
fn start_echo_server(port: u16) -> ServerGuard2 {
    let child = Command::new(env!("CARGO_BIN_EXE_yar-server-rust"))
        .args([
            "--host",
            &format!("127.0.0.1:{}", port),
            "--max-request-size",
            "8m",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn yar-server-rust");

    let mut guard = ServerGuard2 { child };

    let addr = format!("127.0.0.1:{}", port);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(Some(status)) = guard.child.try_wait() {
            panic!(
                "yar-server-rust exited early before accepting on {}: {}",
                addr, status
            );
        }
        match std::net::TcpStream::connect(&addr) {
            Ok(_) => return guard,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("server on {} never became reachable: {}", addr, e),
        }
    }
}

/// Full load run: two stages against the live echo server.
#[test]
fn test_end_to_end_profile_two_stages() {
    // Bind to port 0 just to discover a free port, then release it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let _server = start_echo_server(port);

    let cfg = ProfilerConfig {
        host: format!("127.0.0.1:{}", port),
        body_sizes: vec![8, 65_536],
        vus: vec![2],
        step_time: Duration::from_millis(500),
    };

    let reports = run_profile(&cfg).expect("profile run against live server must succeed");

    assert_eq!(reports.len(), 2, "expected two stage reports");

    for (idx, r) in reports.iter().enumerate() {
        assert!(
            r.ok > 0,
            "stage {} (size {}) completed no requests",
            idx,
            r.body_size
        );
        assert_eq!(
            r.fail, 0,
            "stage {} (size {}) had failures: {}",
            idx,
            r.body_size,
            r.first_error
        );
    }

    let (small, large) = (&reports[0], &reports[1]);
    assert_eq!(small.body_size, 8);
    assert_eq!(large.body_size, 65_536);
    assert!(
        large.qps < small.qps,
        "large-body qps ({}) must be lower than small-body qps ({})",
        large.qps,
        small.qps
    );

    assert!(small.start_ts > 0, "start_ts must be an epoch second");
    assert!(large.end_ts >= large.start_ts, "stage window must be ordered");
    assert!(small.end_ts >= small.start_ts);

    // CSV output starts with the fixed header.
    let mut buf: Vec<u8> = Vec::new();
    write_csv(&reports, &mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();
    let header = "size_bytes,vus,duration_s,qps,ok,fail,p50_ms,p95_ms,p99_ms,p999_ms,first_error,start_ts,end_ts";
    assert_eq!(text.lines().next(), Some(header));
}

// ---------------------------------------------------------------------------
// Failure accounting and connection recovery
// ---------------------------------------------------------------------------

/// A connection error is a failed sample (spec: failure judgment includes
/// connection errors), never a silently empty stage.
#[test]
fn test_connect_failure_is_counted_as_failure() {
    // Reserve a port and release it, so nothing is listening there.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let cfg = ProfilerConfig {
        host: format!("127.0.0.1:{}", port),
        body_sizes: vec![64],
        vus: vec![2],
        step_time: Duration::from_millis(200),
    };

    let reports = run_profile(&cfg).expect("a refused connection is not a run error");
    assert_eq!(reports.len(), 1);
    let r = &reports[0];
    assert_eq!(r.ok, 0, "no call can succeed without a server");
    assert!(
        r.fail >= 2,
        "each VU's failed connect must count as a failure, got {}",
        r.fail
    );
    assert!(
        r.first_error.contains("connect failed"),
        "first_error should name the failure, got {:?}",
        r.first_error
    );
}

/// The server answers the first connection and hangs up, then keeps every
/// later connection alive. A VU must dial a fresh connection after the hangup
/// instead of hammering the dead socket for the rest of the stage (which used
/// to inflate `fail`/`qps` while burning CPU).
#[test]
fn test_vu_reconnects_when_the_server_closes_the_connection() {
    use std::io::{Read as _, Write as _};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use yar_rs::pack::Packager;
    use yar_rs::protocol::{YarHeader, HEADER_SIZE};
    use yar_rs::response::YarResponse;
    use yar_rs::PACKAGER_NAME;

    const BODY: usize = 64;
    let body: Vec<u8> = (0..BODY).map(|i| (i % 251) as u8).collect();

    fn echo_response(id: u32, body: &[u8]) -> Vec<u8> {
        let mut one = Packager::new();
        one.push_string(body).unwrap();
        let mut arr = Packager::start_array(1).unwrap();
        arr.push_packager(&one).unwrap();
        let resp = YarResponse::success(id as u64, arr);

        let extra = HEADER_SIZE + PACKAGER_NAME.len();
        let mut payload = resp.pack(extra).unwrap();
        let resp_len = (payload.len() - HEADER_SIZE) as u32;
        let mut header = YarHeader::new();
        header.render(id, None, None, resp_len, 0);
        let header_bytes = header.to_bytes();
        payload[..HEADER_SIZE].copy_from_slice(&header_bytes);
        payload[HEADER_SIZE..extra].copy_from_slice(PACKAGER_NAME);
        payload
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connections = Arc::new(AtomicUsize::new(0));

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let body = body.clone();
            let first = connections.fetch_add(1, Ordering::SeqCst) == 0;
            std::thread::spawn(move || loop {
                let mut header = [0u8; HEADER_SIZE];
                if stream.read_exact(&mut header).is_err() {
                    return;
                }
                let (id, body_len) = match YarHeader::from_bytes(&header) {
                    Ok(h) => (h.id, h.body_len as usize),
                    Err(_) => return,
                };
                let mut req_body = vec![0u8; body_len];
                if stream.read_exact(&mut req_body).is_err() {
                    return;
                }
                if stream.write_all(&echo_response(id, &body)).is_err() {
                    return;
                }
                let _ = stream.flush();
                if first {
                    return; // hang up after the first response on this connection only
                }
            });
        }
    });

    let cfg = ProfilerConfig {
        host: format!("127.0.0.1:{}", port),
        body_sizes: vec![BODY],
        vus: vec![2],
        step_time: Duration::from_millis(600),
    };
    let reports = run_profile(&cfg).expect("profile run failed");
    let r = &reports[0];
    assert!(
        r.ok > 100,
        "expected the VU to keep working after a reconnect, got ok={} fail={} err={:?}",
        r.ok,
        r.fail,
        r.first_error
    );
    assert!(
        r.fail <= 2,
        "a closed connection must be re-dialed, not retried: fail={} err={:?}",
        r.fail,
        r.first_error
    );
}
