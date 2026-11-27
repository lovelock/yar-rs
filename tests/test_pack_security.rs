//! Security regression tests for msgpack parsing of untrusted input.

use yar_rs::{Unpackager, YarData};

fn nested_arrays(depth: usize) -> Vec<u8> {
    let mut bytes = vec![0x91; depth];
    bytes.push(0xc0);
    bytes
}

#[test]
fn deeply_nested_array_is_rejected() {
    let bytes = nested_arrays(2_000);
    let err = Unpackager::new(&bytes)
        .err()
        .expect("deep nesting must be rejected");
    assert!(
        err.to_string().contains("depth"),
        "expected a depth-limit error, got: {err}"
    );
}

#[test]
fn deeply_nested_map_is_rejected() {
    // Each nesting level is a one-entry map: 0x81, then the key/value.
    let depth = 2_000;
    let mut bytes = Vec::with_capacity(depth * 2 + 1);
    for _ in 0..depth {
        bytes.push(0x81);
        bytes.push(0xa1);
        bytes.push(b'k');
    }
    bytes.push(0xc0);
    let err = Unpackager::new(&bytes)
        .err()
        .expect("deep map nesting must be rejected");
    assert!(
        err.to_string().contains("depth"),
        "expected a depth-limit error, got: {err}"
    );
}

#[test]
fn array_length_beyond_input_is_rejected_before_allocation() {
    // array32 with a declared length of 1000 but no elements following.
    let bytes = [0xdd, 0x00, 0x00, 0x03, 0xe8];
    let err = Unpackager::new(&bytes)
        .err()
        .expect("oversized array must be rejected");
    assert!(
        err.to_string().contains("exceeds remaining input"),
        "expected a length-limit error, got: {err}"
    );
}

#[test]
fn map_length_beyond_input_is_rejected_before_allocation() {
    // map32 with a declared length of 1000 but no entries following.
    let bytes = [0xdf, 0x00, 0x00, 0x03, 0xe8];
    let err = Unpackager::new(&bytes)
        .err()
        .expect("oversized map must be rejected");
    assert!(
        err.to_string().contains("exceeds remaining input"),
        "expected a length-limit error, got: {err}"
    );
}

#[test]
fn string_length_beyond_input_is_rejected_before_allocation() {
    // str32 with a declared length of 1000 but no body following.
    let bytes = [0xdb, 0x00, 0x00, 0x03, 0xe8];
    let err = Unpackager::new(&bytes)
        .err()
        .expect("oversized string must be rejected");
    assert!(
        err.to_string().contains("exceeds remaining input"),
        "expected a length-limit error, got: {err}"
    );
}

#[test]
fn excessive_node_count_is_rejected() {
    let items = 1_000_001u32;
    let mut bytes = Vec::with_capacity(5 + items as usize);
    bytes.push(0xdd); // array32
    bytes.extend_from_slice(&items.to_be_bytes());
    bytes.extend(std::iter::repeat_n(0xc0, items as usize));
    let err = Unpackager::new(&bytes)
        .err()
        .expect("node limit must be enforced");
    assert!(
        err.to_string().contains("node"),
        "expected a node-limit error, got: {err}"
    );
}

#[test]
fn trailing_bytes_are_rejected() {
    let err = Unpackager::new(&[0xc0, 0x00])
        .err()
        .expect("trailing bytes must be rejected");
    assert!(
        err.to_string().contains("trailing"),
        "expected a trailing-data error, got: {err}"
    );
}

#[test]
fn empty_container_current_does_not_panic() {
    let empty = PackagerForTest::array();
    let data = Unpackager::new(&empty).unwrap().into_data();
    let it = data.iter().unwrap();
    assert_eq!(it.current(), &YarData::Null);
}

struct PackagerForTest;

impl PackagerForTest {
    fn array() -> Vec<u8> {
        vec![0x90]
    }
}
