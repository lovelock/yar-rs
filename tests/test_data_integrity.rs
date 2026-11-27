//! Request/response data-integrity regression tests.

use yar_rs::{Packager, Unpackager, YarData, YarRequest, YarResponse};

fn params() -> Packager {
    let mut arr = Packager::start_array(2).unwrap();
    arr.push_long(42).unwrap();
    arr.push_string(b"payload").unwrap();
    arr
}

#[test]
fn request_survives_unpack_then_pack() {
    let mut req = YarRequest::new(7, "method");
    req.set_parameters(params());

    let wire = req.pack(0).unwrap();
    let unpacked = YarRequest::unpack(&wire, 0).unwrap();
    let repacked = unpacked.pack(0).unwrap();
    let again = YarRequest::unpack(&repacked, 0).unwrap();

    let arr = again.get_parameters().unwrap().as_array().unwrap();
    assert_eq!(arr[0].as_ulong(), Some(42));
    assert_eq!(arr[1].as_str(), Some("payload"));
}

#[test]
fn response_survives_unpack_then_pack() {
    let mut retval = Packager::start_array(2).unwrap();
    retval.push_long(1).unwrap();
    retval.push_string(b"ok").unwrap();
    let resp = YarResponse::success(9, retval);

    let wire = resp.pack(0).unwrap();
    let unpacked = YarResponse::unpack(&wire, 0).unwrap();
    let repacked = unpacked.pack(0).unwrap();
    let again = YarResponse::unpack(&repacked, 0).unwrap();

    let arr = again.get_response().unwrap().as_array().unwrap();
    assert_eq!(arr[0].as_ulong(), Some(1));
    assert_eq!(arr[1].as_str(), Some("ok"));
}

#[test]
fn ulong_too_large_for_i64_does_not_wrap() {
    assert_eq!(YarData::Ulong(u64::MAX).as_integer(), None);
    assert_eq!(YarData::Ulong(1u64 << 63).as_integer(), None);
    assert_eq!(YarData::Ulong(i64::MAX as u64).as_integer(), Some(i64::MAX));
    assert_eq!(YarData::Long(-5).as_integer(), Some(-5));
}

#[test]
fn response_without_id_is_rejected() {
    // { "s": 0, "r": null, "e": null } — no "i"
    let mut pk = Packager::start_map(3).unwrap();
    pk.push_string(b"s").unwrap();
    pk.push_long(0).unwrap();
    pk.push_string(b"r").unwrap();
    pk.push_null().unwrap();
    pk.push_string(b"e").unwrap();
    pk.push_null().unwrap();

    let err = YarResponse::unpack(pk.as_bytes(), 0)
        .err()
        .expect("missing id must be rejected");
    assert!(err.to_string().contains("'i'"), "got: {err}");
}

#[test]
fn response_without_status_is_rejected() {
    // { "i": 1, "r": null, "e": null } — no "s"
    let mut pk = Packager::start_map(3).unwrap();
    pk.push_string(b"i").unwrap();
    pk.push_long(1).unwrap();
    pk.push_string(b"r").unwrap();
    pk.push_null().unwrap();
    pk.push_string(b"e").unwrap();
    pk.push_null().unwrap();

    let err = YarResponse::unpack(pk.as_bytes(), 0)
        .err()
        .expect("missing status must be rejected");
    assert!(err.to_string().contains("'s'"), "got: {err}");
}

#[test]
fn repacked_request_is_still_wire_valid() {
    // Guard against the fallback accidentally adding an extra map entry.
    let mut req = YarRequest::new(1, "a");
    req.set_parameters(params());
    let wire = req.pack(0).unwrap();
    let unpacked = YarRequest::unpack(&wire, 0).unwrap();
    let repacked = unpacked.pack(0).unwrap();

    let root = Unpackager::new(&repacked).unwrap();
    assert_eq!(root.data().as_map().unwrap().len(), 3);
}
