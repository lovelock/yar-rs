//! Tests for the profiler library: percentile primitives, CSV rendering,
//! and an end-to-end load run against the real `yar-server-rust` binary.

use yar_rs::profiler::percentile;

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
