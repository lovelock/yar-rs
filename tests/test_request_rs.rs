//! Request integration tests - C binary compatibility verification.
//!
//! Uses known msgpack byte sequences from C test output as test vectors
//! to verify the Rust request pack/unpack is wire-compatible.

use yar_rs::*;

// ---------------------------------------------------------------------------
// C test vectors: exact msgpack bytes produced by C yar_request_pack
// ---------------------------------------------------------------------------

#[test]
fn test_request_c_vector_null_params() {
    // C: request body {i:1000, m:"hello", p:null}
    // Expected msgpack bytes:
    //   0x83                                     fixmap(3)
    //   0xa1 0x69                                fixstr(1) "i"
    //   0xcd 0x03 0xe8                           uint16(1000)
    //   0xa1 0x6d                                fixstr(1) "m"
    //   0xa5 0x68 0x65 0x6c 0x6c 0x6f           fixstr(5) "hello"
    //   0xa1 0x70                                fixstr(1) "p"
    //   0xc0                                     nil
    let expected: Vec<u8> = vec![
        0x83,
        0xa1, 0x69,
        0xcd, 0x03, 0xe8,
        0xa1, 0x6d,
        0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f,
        0xa1, 0x70,
        0xc0,
    ];

    // Pack with Rust and verify bytes match C output
    let req = YarRequest::new(1000, "hello");
    let payload = req.pack(0).unwrap();

    assert_eq!(
        payload, expected,
        "Rust pack must produce identical bytes to C:\n  got:      {:02x?}\n  expected: {:02x?}",
        payload, expected
    );
}

#[test]
fn test_request_c_vector_with_params() {
    // C: request body {i:2000, m:"greet", p:[42, "world"]}
    // Expected msgpack bytes:
    //   0x83                                     fixmap(3)
    //   0xa1 0x69                                fixstr(1) "i"
    //   0xcd 0x07 0xd0                           uint16(2000)
    //   0xa1 0x6d                                fixstr(1) "m"
    //   0xa5 0x67 0x72 0x65 0x65 0x74           fixstr(5) "greet"
    //   0xa1 0x70                                fixstr(1) "p"
    //   0x92                                     fixarray(2)
    //   0x2a                                     fixint(42)
    //   0xa5 0x77 0x6f 0x72 0x6c 0x64           fixstr(5) "world"
    let expected: Vec<u8> = vec![
        0x83,
        0xa1, 0x69,
        0xcd, 0x07, 0xd0,
        0xa1, 0x6d,
        0xa5, 0x67, 0x72, 0x65, 0x65, 0x74,
        0xa1, 0x70,
        0x92,
        0x2a,
        0xa5, 0x77, 0x6f, 0x72, 0x6c, 0x64,
    ];

    // Build params: array [42, "world"]
    let mut params = Packager::start_array(2).unwrap();
    params.push_long(42).unwrap();
    params.push_string(b"world").unwrap();

    let mut req = YarRequest::new(2000, "greet");
    req.set_parameters(params);
    let payload = req.pack(0).unwrap();

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
fn test_unpack_c_request_null_params() {
    // Bytes produced by C yar_request_pack for {i:1000, m:"hello", p:null}
    let c_bytes: Vec<u8> = vec![
        0x83,
        0xa1, 0x69,
        0xcd, 0x03, 0xe8,
        0xa1, 0x6d,
        0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f,
        0xa1, 0x70,
        0xc0,
    ];

    let req = YarRequest::unpack(&c_bytes, 0).unwrap();
    assert_eq!(req.id(), 1000);
    assert_eq!(req.method(), "hello");

    let params = req.get_parameters().expect("params key should be present");
    assert_eq!(params, &YarData::Null);
}

#[test]
fn test_unpack_c_request_with_params() {
    // Bytes produced by C for {i:2000, m:"greet", p:[42, "world"]}
    let c_bytes: Vec<u8> = vec![
        0x83,
        0xa1, 0x69,
        0xcd, 0x07, 0xd0,
        0xa1, 0x6d,
        0xa5, 0x67, 0x72, 0x65, 0x65, 0x74,
        0xa1, 0x70,
        0x92,
        0x2a,
        0xa5, 0x77, 0x6f, 0x72, 0x6c, 0x64,
    ];

    let req = YarRequest::unpack(&c_bytes, 0).unwrap();
    assert_eq!(req.id(), 2000);
    assert_eq!(req.method(), "greet");

    let params = req.get_parameters().unwrap();
    let arr = params.as_array().expect("params should be an array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0].as_ulong(), Some(42));
    assert_eq!(arr[1].as_str(), Some("world"));
}

// ---------------------------------------------------------------------------
// Pack/unpack roundtrip
// ---------------------------------------------------------------------------

#[test]
fn test_request_pack_unpack_roundtrip() {
    let mut params = Packager::start_array(2).unwrap();
    params.push_long(42).unwrap();
    params.push_string(b"world").unwrap();

    let mut req = YarRequest::new(1000, "hello");
    req.set_parameters(params);

    let payload = req.pack(0).unwrap();
    let unpacked = YarRequest::unpack(&payload, 0).unwrap();

    assert_eq!(unpacked.id(), 1000);
    assert_eq!(unpacked.method(), "hello");

    let p = unpacked.get_parameters().unwrap();
    let arr = p.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0].as_ulong(), Some(42));
    assert_eq!(arr[1].as_str(), Some("world"));
}

#[test]
fn test_request_no_params_roundtrip() {
    let req = YarRequest::new(42, "ping");
    let payload = req.pack(0).unwrap();
    let unpacked = YarRequest::unpack(&payload, 0).unwrap();

    assert_eq!(unpacked.id(), 42);
    assert_eq!(unpacked.method(), "ping");

    let params = unpacked.get_parameters().unwrap();
    assert_eq!(params, &YarData::Null);
}

// ---------------------------------------------------------------------------
// Extra bytes (header + packager name prefix)
// ---------------------------------------------------------------------------

#[test]
fn test_request_extra_bytes_prefix() {
    let mut req = YarRequest::new(999, "test");
    let mut params = Packager::start_array(1).unwrap();
    params.push_long(1).unwrap();
    req.set_parameters(params);

    let extra = 90usize; // header(82) + packager_name(8) = 90
    let payload = req.pack(extra).unwrap();

    // First `extra` bytes must all be zero
    assert!(payload.len() > extra);
    assert!(
        payload[..extra].iter().all(|&b| b == 0),
        "first {} bytes should be zero",
        extra
    );

    // Unpack with the same offset should recover the request
    let unpacked = YarRequest::unpack(&payload, extra).unwrap();
    assert_eq!(unpacked.id(), 999);
    assert_eq!(unpacked.method(), "test");
}

#[test]
fn test_request_extra_bytes_matches_wire_format() {
    // The extra_bytes offset matches the wire format:
    //   [header: 82 bytes] [packager name: 8 bytes] [msgpack body]
    let extra = HEADER_SIZE + PACKAGER_NAME.len();
    assert_eq!(extra, 90);

    let req = YarRequest::new(1, "a");
    let payload = req.pack(extra).unwrap();

    // The msgpack body starts at offset 90
    let msgpack_body = &payload[extra..];

    // Verify the msgpack body is valid
    let unpacked = YarRequest::unpack(&payload, extra).unwrap();
    assert_eq!(unpacked.id(), 1);
    assert_eq!(unpacked.method(), "a");

    // Also verify we can parse the msgpack body directly
    let unpk = Unpackager::new(msgpack_body).unwrap();
    let map = unpk.data().as_map().unwrap();
    assert_eq!(map.len(), 3);
}

// ---------------------------------------------------------------------------
// Binary compatibility: minimal request
// ---------------------------------------------------------------------------

#[test]
fn test_request_binary_compat_minimal() {
    // Minimal request: {i:1, m:"a", p:null}
    let req = YarRequest::new(1, "a");
    let payload = req.pack(0).unwrap();

    let expected: Vec<u8> = vec![
        0x83,             // fixmap(3)
        0xa1, 0x69,       // fixstr(1) "i"
        0x01,             // fixint 1
        0xa1, 0x6d,       // fixstr(1) "m"
        0xa1, 0x61,       // fixstr(1) "a"
        0xa1, 0x70,       // fixstr(1) "p"
        0xc0,             // nil
    ];

    assert_eq!(payload, expected);
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[test]
fn test_request_unpack_truncated_fails() {
    // Too short to be a valid msgpack map
    assert!(YarRequest::unpack(&[0x83, 0xa1], 0).is_err());
}

#[test]
fn test_request_unpack_empty_fails() {
    assert!(YarRequest::unpack(&[], 0).is_err());
}

#[test]
fn test_request_unpack_extra_bytes_exceeds_data() {
    let data = vec![0x83, 0xa1, 0x69];
    assert!(YarRequest::unpack(&data, 100).is_err());
}

// ---------------------------------------------------------------------------
// Request getters
// ---------------------------------------------------------------------------

#[test]
fn test_request_getters() {
    let req = YarRequest::new(42, "my_method");
    assert_eq!(req.id(), 42);
    assert_eq!(req.method(), "my_method");
    assert!(req.get_parameters().is_none());
}

// ---------------------------------------------------------------------------
// Complex parameter types
// ---------------------------------------------------------------------------

#[test]
fn test_request_with_map_params() {
    // Build params: map {"key": "value", "count": 5}
    let mut params = Packager::start_map(2).unwrap();
    params.push_string(b"key").unwrap();
    params.push_string(b"value").unwrap();
    params.push_string(b"count").unwrap();
    params.push_long(5).unwrap();

    let mut req = YarRequest::new(100, "lookup");
    req.set_parameters(params);

    let payload = req.pack(0).unwrap();
    let unpacked = YarRequest::unpack(&payload, 0).unwrap();

    assert_eq!(unpacked.id(), 100);
    assert_eq!(unpacked.method(), "lookup");

    let p = unpacked.get_parameters().unwrap();
    let map = p.as_map().unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map[0].0.as_str(), Some("key"));
    assert_eq!(map[0].1.as_str(), Some("value"));
    assert_eq!(map[1].0.as_str(), Some("count"));
    assert_eq!(map[1].1.as_ulong(), Some(5));
}

#[test]
fn test_request_with_nested_params() {
    // Build params: array [null, true, [1, 2, 3]]
    let mut inner_arr = Packager::start_array(3).unwrap();
    inner_arr.push_long(1).unwrap();
    inner_arr.push_long(2).unwrap();
    inner_arr.push_long(3).unwrap();

    let mut params = Packager::start_array(3).unwrap();
    params.push_null().unwrap();
    params.push_bool(true).unwrap();
    params.push_packager(&inner_arr).unwrap();

    let mut req = YarRequest::new(200, "nested");
    req.set_parameters(params);

    let payload = req.pack(0).unwrap();
    let unpacked = YarRequest::unpack(&payload, 0).unwrap();

    let p = unpacked.get_parameters().unwrap();
    let arr = p.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0], YarData::Null);
    assert_eq!(arr[1].as_bool(), Some(true));

    let inner = arr[2].as_array().unwrap();
    assert_eq!(inner.len(), 3);
    assert_eq!(inner[0].as_ulong(), Some(1));
    assert_eq!(inner[1].as_ulong(), Some(2));
    assert_eq!(inner[2].as_ulong(), Some(3));
}
