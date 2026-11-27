//! Response integration tests - C binary compatibility verification.
//!
//! Uses known msgpack byte sequences from C test output as test vectors
//! to verify the Rust response pack/unpack is wire-compatible.

use yar_rs::*;

// ---------------------------------------------------------------------------
// C test vectors: exact msgpack bytes produced by C yar_response_pack
// ---------------------------------------------------------------------------

#[test]
fn test_response_c_vector_success() {
    // C: response body {i:1000, s:0, r:"result", e:null}
    // Expected msgpack bytes:
    //   0x84                                     fixmap(4)
    //   0xa1 0x69                                fixstr(1) "i"
    //   0xcd 0x03 0xe8                           uint16(1000)
    //   0xa1 0x73                                fixstr(1) "s"
    //   0x00                                     fixint(0)
    //   0xa1 0x72                                fixstr(1) "r"
    //   0xa6 0x72 0x65 0x73 0x75 0x6c 0x74     fixstr(6) "result"
    //   0xa1 0x65                                fixstr(1) "e"
    //   0xc0                                     nil
    let expected: Vec<u8> = vec![
        0x84, 0xa1, 0x69, 0xcd, 0x03, 0xe8, 0xa1, 0x73, 0x00, 0xa1, 0x72, 0xa6, 0x72, 0x65, 0x73,
        0x75, 0x6c, 0x74, 0xa1, 0x65, 0xc0,
    ];

    // Pack with Rust and verify bytes match C output
    let mut retval = Packager::new();
    retval.push_string(b"result").unwrap();

    let resp = YarResponse::success(1000, retval);
    let payload = resp.pack(0).unwrap();

    assert_eq!(
        payload, expected,
        "Rust pack must produce identical bytes to C:\n  got:      {:02x?}\n  expected: {:02x?}",
        payload, expected
    );
}

#[test]
fn test_response_c_vector_error() {
    // C: response body {i:2000, s:8, r:null, e:"something went wrong"}
    // Expected msgpack bytes:
    //   0x84                                             fixmap(4)
    //   0xa1 0x69                                        fixstr(1) "i"
    //   0xcd 0x07 0xd0                                   uint16(2000)
    //   0xa1 0x73                                        fixstr(1) "s"
    //   0x08                                             fixint(8)
    //   0xa1 0x72                                        fixstr(1) "r"
    //   0xc0                                             nil
    //   0xa1 0x65                                        fixstr(1) "e"
    //   0xb8                                             fixstr(22)
    //   "something went wrong" (22 bytes)
    let error_msg = b"something went wrong"; // 20 bytes
    let mut expected: Vec<u8> = vec![
        0x84, 0xa1, 0x69, 0xcd, 0x07, 0xd0, 0xa1, 0x73, 0x08, 0xa1, 0x72, 0xc0, 0xa1, 0x65,
    ];
    // fixstr(20) = 0xa0 | 20 = 0xb4
    expected.push(0xa0 | 20);
    expected.extend_from_slice(error_msg);

    // Pack with Rust and verify bytes match
    let resp = YarResponse::error(2000, 8, "something went wrong");
    let payload = resp.pack(0).unwrap();

    assert_eq!(
        payload, expected,
        "Rust pack must produce identical bytes to C:\n  got:      {:02x?}\n  expected: {:02x?}",
        payload, expected
    );
}

// ---------------------------------------------------------------------------
// Unpack C-produced bytes with Rust
// ---------------------------------------------------------------------------

#[test]
fn test_unpack_c_response_success() {
    // Bytes produced by C yar_response_pack for success response
    let c_bytes: Vec<u8> = vec![
        0x84, 0xa1, 0x69, 0xcd, 0x03, 0xe8, 0xa1, 0x73, 0x00, 0xa1, 0x72, 0xa6, 0x72, 0x65, 0x73,
        0x75, 0x6c, 0x74, 0xa1, 0x65, 0xc0,
    ];

    let resp = YarResponse::unpack(&c_bytes, 0).unwrap();
    assert_eq!(resp.id, 1000);
    assert_eq!(resp.get_status(), 0);
    assert!(resp.get_error().is_none());

    let retval = resp.get_response().unwrap();
    assert_eq!(retval.as_str(), Some("result"));
}

#[test]
fn test_unpack_c_response_error() {
    let error_msg = "something went wrong";
    let mut c_bytes: Vec<u8> = vec![
        0x84, 0xa1, 0x69, 0xcd, 0x07, 0xd0, 0xa1, 0x73, 0x08, 0xa1, 0x72, 0xc0, 0xa1, 0x65,
    ];
    c_bytes.push(0xa0 | error_msg.len() as u8);
    c_bytes.extend_from_slice(error_msg.as_bytes());

    let resp = YarResponse::unpack(&c_bytes, 0).unwrap();
    assert_eq!(resp.id, 2000);
    assert_eq!(resp.get_status(), 8);
    assert_eq!(resp.get_error(), Some("something went wrong"));
    assert!(resp.get_response().is_none());
}

// ---------------------------------------------------------------------------
// Pack/unpack roundtrip
// ---------------------------------------------------------------------------

#[test]
fn test_response_success_roundtrip() {
    let mut retval = Packager::new();
    retval.push_string(b"hello world").unwrap();

    let resp = YarResponse::success(42, retval);
    let payload = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&payload, 0).unwrap();

    assert_eq!(unpacked.id, 42);
    assert_eq!(unpacked.get_status(), 0);
    assert!(unpacked.get_error().is_none());

    let rv = unpacked.get_response().unwrap();
    assert_eq!(rv.as_str(), Some("hello world"));
}

#[test]
fn test_response_error_roundtrip() {
    let resp = YarResponse::error(99, 5, "timeout exceeded");
    let payload = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&payload, 0).unwrap();

    assert_eq!(unpacked.id, 99);
    assert_eq!(unpacked.get_status(), 5);
    assert_eq!(unpacked.get_error(), Some("timeout exceeded"));
    assert!(unpacked.get_response().is_none());
}

#[test]
fn test_response_empty_success_roundtrip() {
    // Success with no return value (null retval)
    let resp = YarResponse::new(100);
    let payload = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&payload, 0).unwrap();

    assert_eq!(unpacked.id, 100);
    assert_eq!(unpacked.get_status(), 0);
    assert!(unpacked.get_error().is_none());
    assert!(unpacked.get_response().is_none());
}

// ---------------------------------------------------------------------------
// Binary compatibility: minimal response
// ---------------------------------------------------------------------------

#[test]
fn test_response_binary_compat_minimal() {
    // Minimal response: {i:1, s:0, r:null, e:null}
    let resp = YarResponse::new(1);
    let payload = resp.pack(0).unwrap();

    let expected: Vec<u8> = vec![
        0x84, // fixmap(4)
        0xa1, b'i', 0x01, // "i" -> 1
        0xa1, b's', 0x00, // "s" -> 0
        0xa1, b'r', 0xc0, // "r" -> nil
        0xa1, b'e', 0xc0, // "e" -> nil
    ];

    assert_eq!(
        payload, expected,
        "minimal response bytes must match:\n  got:      {:02x?}\n  expected: {:02x?}",
        payload, expected
    );
}

// ---------------------------------------------------------------------------
// Extra bytes (header + packager name prefix)
// ---------------------------------------------------------------------------

#[test]
fn test_response_extra_bytes_prefix() {
    let resp = YarResponse::new(100);
    let extra = HEADER_SIZE + PACKAGER_NAME.len(); // 90
    let payload = resp.pack(extra).unwrap();

    // First `extra` bytes should be zeros
    assert!(payload.len() > extra);
    assert!(
        payload[..extra].iter().all(|&b| b == 0),
        "first {} bytes should be zero",
        extra
    );

    // Unpack with same offset
    let unpacked = YarResponse::unpack(&payload, extra).unwrap();
    assert_eq!(unpacked.id, 100);
    assert_eq!(unpacked.get_status(), 0);
}

// ---------------------------------------------------------------------------
// Error truncation at 1024 bytes
// ---------------------------------------------------------------------------

#[test]
fn test_response_error_truncation() {
    let mut resp = YarResponse::new(1);

    // Message under 1024 bytes: stored as-is
    resp.set_error(1, "short error");
    assert_eq!(resp.get_error(), Some("short error"));

    // Message over 1024 bytes: truncated with "..." appended
    let long_msg = "x".repeat(2000);
    resp.set_error(1, &long_msg);
    let err = resp.get_error().unwrap();
    assert_eq!(err.len(), 1027); // 1024 bytes + "..."
    assert!(err.ends_with("..."));
}

// ---------------------------------------------------------------------------
// Return value types
// ---------------------------------------------------------------------------

#[test]
fn test_response_with_integer_retval() {
    let mut retval = Packager::new();
    retval.push_ulong(99).unwrap();

    let resp = YarResponse::success(42, retval);
    let payload = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&payload, 0).unwrap();

    let rv = unpacked.get_response().unwrap();
    assert_eq!(rv.as_ulong(), Some(99));
}

#[test]
fn test_response_with_array_retval() {
    let mut retval = Packager::start_array(3).unwrap();
    retval.push_long(1).unwrap();
    retval.push_long(2).unwrap();
    retval.push_long(3).unwrap();

    let resp = YarResponse::success(1, retval);
    let payload = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&payload, 0).unwrap();

    let rv = unpacked.get_response().unwrap();
    let arr = rv.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_ulong(), Some(1));
    assert_eq!(arr[1].as_ulong(), Some(2));
    assert_eq!(arr[2].as_ulong(), Some(3));
}

#[test]
fn test_response_with_null_retval() {
    let mut retval = Packager::new();
    retval.push_null().unwrap();

    let resp = YarResponse::success(1, retval);
    let payload = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&payload, 0).unwrap();

    // Null retval: the response stores null as YarData::Null, but
    // get_response() checks for non-Null type, so it returns None.
    // This matches C behavior where null retval means "no return value".
    assert!(unpacked.get_response().is_none());
}

// ---------------------------------------------------------------------------
// set_retval method
// ---------------------------------------------------------------------------

#[test]
fn test_response_set_retval() {
    let mut resp = YarResponse::new(42);
    let mut pk = Packager::new();
    pk.push_string(b"via_set_retval").unwrap();
    resp.set_retval(pk);

    let payload = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&payload, 0).unwrap();

    assert_eq!(unpacked.id, 42);
    assert_eq!(unpacked.get_status(), 0);
    let rv = unpacked.get_response().unwrap();
    assert_eq!(rv.as_str(), Some("via_set_retval"));
}

// ---------------------------------------------------------------------------
// Status code handling
// ---------------------------------------------------------------------------

#[test]
fn test_response_status_codes() {
    // Status 0: success
    let resp = YarResponse::new(1);
    assert_eq!(resp.get_status(), 0);

    // Non-zero status: error
    let resp = YarResponse::error(1, 1, "error");
    assert_eq!(resp.get_status(), 1);

    // Large status code
    let resp = YarResponse::error(1, 255, "big error");
    assert_eq!(resp.get_status(), 255);
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[test]
fn test_response_unpack_truncated_fails() {
    assert!(YarResponse::unpack(&[0x84, 0xa1], 0).is_err());
}

#[test]
fn test_response_unpack_empty_fails() {
    assert!(YarResponse::unpack(&[], 0).is_err());
}

#[test]
fn test_response_unpack_extra_bytes_exceeds_data() {
    let data = vec![0x84, 0xa1, 0x69];
    assert!(YarResponse::unpack(&data, 100).is_err());
}
