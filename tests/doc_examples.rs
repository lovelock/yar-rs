//! 文档示例代码校验。
//!
//! `docs/` 用户指南中的示例代码在这里逐段落地并被真实执行，
//! 保证文档 snippet 始终与当前 API 一致、可编译、可运行。
//! 每个 `#[test]` 对应文档中的一段示例；修改文档时同步修改这里，反之亦然。

use std::time::Duration;
use yar_rs::*;

// ---------------------------------------------------------------------------
// 工具函数
// ---------------------------------------------------------------------------

/// 绑定端口 0 找一个空闲端口，然后释放。
fn find_available_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// 轮询等待端口可连接（最多约 2 秒）。
fn wait_for_port(port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    for _ in 0..40 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start on port {}", port);
}

/// 在后台线程里启动一个 Tokio runtime 上的 YarServer。
fn spawn_server<F>(port: u16, register: F) -> std::thread::JoinHandle<()>
where
    F: FnOnce(&YarServer) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>
        + Send
        + 'static,
{
    let addr = format!("127.0.0.1:{}", port);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let server = YarServer::new(&addr);
            register(&server).await;
            server.run().await.unwrap();
        });
    })
}

// ---------------------------------------------------------------------------
// docs/getting-started.md —— 最小完整示例
// ---------------------------------------------------------------------------

#[test]
fn doc_getting_started_minimal() {
    let port = find_available_port();

    let _server = spawn_server(port, |server| {
        Box::pin(async {
            server
                .register_handler("echo", |req| {
                    // 构造返回值："echo:ok"
                    let mut pk = Packager::new();
                    pk.push_string(b"echo:ok").unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();
    let resp = client.call("echo", vec![]).unwrap();

    assert_eq!(resp.get_status(), 0);
    let retval = resp.get_response().unwrap();
    assert_eq!(retval.as_str(), Some("echo:ok"));

    client.close();
}

// ---------------------------------------------------------------------------
// docs/client.md —— 带参数调用（多个 Packager 打包为参数数组）
// ---------------------------------------------------------------------------

#[test]
fn doc_client_call_with_params() {
    let port = find_available_port();

    let _server = spawn_server(port, |server| {
        Box::pin(async {
            // "add"：读取参数数组中的两个整数，返回和
            server
                .register_handler("add", |req| {
                    let arr = req
                        .get_parameters()
                        .and_then(|p| p.as_array())
                        .unwrap_or(&[]);

                    let a = arr.first().and_then(|v| v.as_integer()).unwrap_or(0);
                    let b = arr.get(1).and_then(|v| v.as_integer()).unwrap_or(0);

                    let mut pk = Packager::new();
                    pk.push_long(a + b).unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();
    client.set_timeout(Duration::from_secs(5));

    // 每个参数是一个独立 Packager；call() 会把它们包进一个 msgpack 数组
    let mut a = Packager::new();
    a.push_long(10).unwrap();
    let mut b = Packager::new();
    b.push_long(20).unwrap();

    let resp = client.call("add", vec![a, b]).unwrap();

    assert_eq!(resp.get_status(), 0);
    assert_eq!(resp.get_response().unwrap().as_integer(), Some(30));

    client.close();
}

#[test]
fn doc_client_call_no_params_is_null() {
    // 无参数时，服务端拿到的 "p" 字段是 Null 而不是数组
    let port = find_available_port();

    let _server = spawn_server(port, |server| {
        Box::pin(async {
            server
                .register_handler("noparams", |req| {
                    // p 为 YarData::Null，as_array() 返回 None
                    assert!(req
                        .get_parameters()
                        .map(|p| p.data_type() == YarDataType::Null)
                        .unwrap_or(true));

                    let mut pk = Packager::new();
                    pk.push_bool(true).unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();
    let resp = client.call("noparams", vec![]).unwrap();
    assert_eq!(resp.get_status(), 0);
    assert_eq!(resp.get_response().unwrap().as_bool(), Some(true));
    client.close();
}

// ---------------------------------------------------------------------------
// docs/server.md —— 处理 map / struct 型参数与返回嵌套结构
// ---------------------------------------------------------------------------

#[test]
fn doc_server_map_params_and_nested_retval() {
    let port = find_available_port();

    let _server = spawn_server(port, |server| {
        Box::pin(async {
            // 参数形如 [{ "user": "alice", "age": 30 }]
            server
                .register_handler("register", |req| {
                    let mut name = "anonymous";
                    let mut age: Option<i64> = None;

                    if let Some(arr) = req.get_parameters().and_then(|p| p.as_array()) {
                        if let Some(first) = arr.first() {
                            if let Some(map) = first.as_map() {
                                for (k, v) in map {
                                    match k.as_str() {
                                        Some("user") => name = v.as_str().unwrap_or("anonymous"),
                                        Some("age") => age = v.as_integer(),
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }

                    // 一维顺序打包：{"user": name, "adult": bool, "alias": [a1, a2]}
                    let mut pk = Packager::start_map(3).unwrap();
                    pk.push_string(b"user").unwrap();
                    pk.push_string(format!("{}@yar", name).as_bytes()).unwrap();
                    pk.push_string(b"adult").unwrap();
                    pk.push_bool(age.map(|a| a >= 18).unwrap_or(false)).unwrap();
                    pk.push_string(b"alias").unwrap();
                    pk.push_array(2).unwrap();
                    pk.push_string(b"al1").unwrap();
                    pk.push_string(b"al2").unwrap();

                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();

    let mut user = Packager::start_map(2).unwrap();
    user.push_string(b"user").unwrap();
    user.push_string(b"alice").unwrap();
    user.push_string(b"age").unwrap();
    user.push_long(30).unwrap();

    let resp = client.call("register", vec![user]).unwrap();
    assert_eq!(resp.get_status(), 0);

    let retval = resp.get_response().unwrap();
    let map = retval.as_map().unwrap();

    let get = |key: &str| -> Option<&YarData> {
        map.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    };

    assert_eq!(get("user").unwrap().as_str(), Some("alice@yar"));
    assert_eq!(get("adult").unwrap().as_bool(), Some(true));
    let alias = get("alias").unwrap().as_array().unwrap();
    assert_eq!(alias.len(), 2);
    assert_eq!(alias[0].as_str(), Some("al1"));

    client.close();
}

// ---------------------------------------------------------------------------
// docs/errors-logging.md —— handler 返回错误，客户端读取 status/error
// ---------------------------------------------------------------------------

#[test]
fn doc_handler_error_response() {
    let port = find_available_port();

    let _server = spawn_server(port, |server| {
        Box::pin(async {
            server
                .register_handler("divide", |req| {
                    let arr = req
                        .get_parameters()
                        .and_then(|p| p.as_array())
                        .unwrap_or(&[]);

                    let b = arr.get(1).and_then(|v| v.as_integer()).unwrap_or(0);
                    if b == 0 {
                        // 出错：设置非 0 status 与错误信息（超过 1024 字节会截断）
                        let mut resp = YarResponse::new(req.id());
                        resp.set_error(10000, "division by zero");
                        return resp;
                    }

                    let a = arr.first().and_then(|v| v.as_integer()).unwrap_or(0);
                    let mut pk = Packager::new();
                    pk.push_long(a / b).unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();
    // 两次 call 要复用同一连接，开启持久模式
    client.set_persistent(true);

    // 正常路径
    let mut a = Packager::new();
    a.push_long(9).unwrap();
    let mut b = Packager::new();
    b.push_long(3).unwrap();
    let resp = client.call("divide", vec![a, b]).unwrap();
    assert_eq!(resp.get_status(), 0);
    assert_eq!(resp.get_response().unwrap().as_integer(), Some(3));

    // 错误路径：除零
    let mut a = Packager::new();
    a.push_long(9).unwrap();
    let mut b = Packager::new();
    b.push_long(0).unwrap();
    let resp = client.call("divide", vec![a, b]).unwrap();
    assert_eq!(resp.get_status(), 10000);
    assert_eq!(resp.get_error(), Some("division by zero"));
    assert!(resp.get_response().is_none());

    client.close();
}

// ---------------------------------------------------------------------------
// docs/server.md —— 持久连接：同一连接上多次 call
// ---------------------------------------------------------------------------

#[test]
fn doc_persistent_connection() {
    let port = find_available_port();

    let _server = spawn_server(port, |server| {
        Box::pin(async {
            server
                .register_handler("add", |req| {
                    let arr = req
                        .get_parameters()
                        .and_then(|p| p.as_array())
                        .unwrap_or(&[]);
                    let sum: i64 = arr.iter().filter_map(|v| v.as_integer()).sum();
                    let mut pk = Packager::new();
                    pk.push_long(sum).unwrap();
                    YarResponse::success(req.id(), pk)
                })
                .await;
        })
    });

    wait_for_port(port);

    let mut client = YarClient::connect(&format!("127.0.0.1:{}", port)).unwrap();
    client.set_persistent(true);

    for (x, y) in [(3, 4), (10, 20)] {
        let mut a = Packager::new();
        a.push_long(x).unwrap();
        let mut b = Packager::new();
        b.push_long(y).unwrap();
        let resp = client.call("add", vec![a, b]).unwrap();
        assert_eq!(resp.get_response().unwrap().as_integer(), Some(x + y));
    }

    client.close();
}

// ---------------------------------------------------------------------------
// docs/data.md —— Packager 嵌套打包（与 C 手册同一示例）
// ---------------------------------------------------------------------------

#[test]
fn doc_pack_nested_structure() {
    // 目标结构：{ a => [b, c], d => e }
    let mut pk = Packager::start_map(2).unwrap();

    pk.push_string(b"a").unwrap(); // 第一个 key: a
    pk.push_array(2).unwrap(); // 第一个 key 对应的 2 元素数组
    pk.push_string(b"b").unwrap(); // 第一个元素
    pk.push_string(b"c").unwrap(); // 第二个元素（数组填充完毕）
    pk.push_string(b"d").unwrap(); // 第二个 key: d
    pk.push_string(b"e").unwrap(); // 第二个 key 对应的值 e

    // 解回来验证
    let data = Unpackager::new(pk.as_bytes()).unwrap().into_data();
    let map = data.as_map().unwrap();
    assert_eq!(map.len(), 2);

    let (k0, v0) = &map[0];
    assert_eq!(k0.as_str(), Some("a"));
    assert_eq!(v0.as_array().unwrap()[1].as_str(), Some("c"));

    let (k1, v1) = &map[1];
    assert_eq!(k1.as_str(), Some("d"));
    assert_eq!(v1.as_str(), Some("e"));
}

// ---------------------------------------------------------------------------
// docs/data.md —— YarData 类型读取与数字类型说明
// ---------------------------------------------------------------------------

#[test]
fn doc_yar_data_accessors() {
    // push_long(小正数) 在 msgpack 中是 "无符号" 编码，解包为 Ulong
    let mut pk = Packager::start_array(4).unwrap();
    pk.push_long(42).unwrap();
    pk.push_long(-42).unwrap();
    pk.push_string(b"hello").unwrap();
    pk.push_array(0).unwrap();

    let data: YarData = Unpackager::new(pk.as_bytes()).unwrap().into_data();
    let arr = data.as_array().unwrap();
    assert_eq!(arr.len(), 4);

    // 小正数解包成 Ulong —— 用 as_integer()（或 as_ulong()）读取
    assert_eq!(arr[0].as_ulong(), Some(42));
    assert_eq!(arr[0].as_integer(), Some(42));
    assert_eq!(arr[0].as_long(), None); // 注意：不是 Long！

    // 负数解包成 Long
    assert_eq!(arr[1].as_integer(), Some(-42));

    // 字符串与空数组
    assert_eq!(arr[2].as_str(), Some("hello"));
    assert_eq!(arr[3].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// docs/data.md —— UnpackIterator（数组迭代 / map 的 kv 交错迭代）
// ---------------------------------------------------------------------------

#[test]
fn doc_unpack_iterator() {
    let mut pk = Packager::start_array(2).unwrap();
    pk.push_long(1).unwrap();
    pk.push_long(2).unwrap();

    let data: YarData = Unpackager::new(pk.as_bytes()).unwrap().into_data();
    let mut it = data.iter().expect("不是容器类型时 iter() 返回 None");

    // map 类型时 iterator 会按 key0, value0, key1, value1 ... 交错产出
    let mut sum = 0;
    loop {
        sum += it.current().as_integer().unwrap_or(0);
        if it.next() == 0 {
            break; // next() 返回 0 表示迭代结束（与 C 一致）
        }
    }
    assert_eq!(sum, 3);
    it.reset(); // 回到开头
    assert_eq!(it.position(), 0);
}

// ---------------------------------------------------------------------------
// docs/protocol.md —— 底层 request / response 的打包解包
// ---------------------------------------------------------------------------

#[test]
fn doc_lowlevel_request_response_roundtrip() {
    // 请求体是 msgpack map：{"i": id, "m": 方法名, "p": 参数}
    let mut params = Packager::start_array(2).unwrap();
    params.push_long(42).unwrap();
    params.push_string(b"world").unwrap();

    let mut req = YarRequest::new(1000, "echo");
    req.set_parameters(params);

    let req_bytes = req.pack(0).unwrap();
    let parsed_req = YarRequest::unpack(&req_bytes, 0).unwrap();
    assert_eq!(parsed_req.id(), 1000);
    assert_eq!(parsed_req.method(), "echo");
    assert_eq!(
        parsed_req
            .get_parameters()
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // 响应体是 msgpack map：{"i": id, "s": 状态, "r": 返回值, "e": 错误}
    let mut retval = Packager::new();
    retval.push_string(b"echo:ok").unwrap();
    let resp = YarResponse::success(1000, retval);

    let resp_bytes = resp.pack(0).unwrap();
    let parsed_resp = YarResponse::unpack(&resp_bytes, 0).unwrap();
    assert_eq!(parsed_resp.get_status(), 0);
    assert_eq!(parsed_resp.get_response().unwrap().as_str(), Some("echo:ok"));
}

// ---------------------------------------------------------------------------
// docs/protocol.md —— 协议头 YarHeader
// ---------------------------------------------------------------------------

#[test]
fn doc_yar_header_wire() {
    use yar_rs::YarHeader;

    let mut header = YarHeader::new();
    // 参数：id, provider(最多写入 16 字节), token(最多写入 16 字节), body_len, reserved(标志位)
    header.render(1000, Some("Yar(Rust)-0.1.0"), None, 128, PROTOCOL_PERSISTENT);
    // render() 之后的字段是网络字节序，parse() 校验魔数并转回主机序，供 to_bytes() 序列化
    header.parse().unwrap();

    // 线上字节（大端）：可以直接 send 给 server / client
    let wire = header.to_bytes();
    assert_eq!(wire.len(), 82); // HEADER_SIZE

    // 对端解析（校验魔数 0x80DFEC60，不匹配返回 Err）
    let parsed = YarHeader::from_bytes(&wire).unwrap();
    let _ = parsed; // YarHeader 是 packed 结构，如需读取数值字段请从 wire 字节读取

    // 从线上字节读取字段（big-endian）
    let id = u32::from_be_bytes(wire[0..4].try_into().unwrap());
    let body_len = u32::from_be_bytes(wire[78..82].try_into().unwrap());
    assert_eq!(id, 1000);
    assert_eq!(body_len, 128);
}

// ---------------------------------------------------------------------------
// docs/errors-logging.md —— 日志系统
// ---------------------------------------------------------------------------

#[test]
fn doc_logger_usage() {
    use yar_rs::logger::{LogLevel, init_global, log_global, set_hostname};

    // None -> stderr；其他支持文件路径追加，或 "|cmd" 管道命令
    init_global(None, LogLevel::Notice).unwrap_or(()); // 已初始化时返回 Err，首次调用成功
    set_hostname("my-yar-service");
    log_global(LogLevel::Notice, "service started");

    // 本地使用也可以独立实例化（不挂全局）
    let local = yar_rs::logger::YarLogger::init(None, LogLevel::Debug).unwrap();
    local.log(LogLevel::Error, "local only");

    assert!(local.is_enabled());
    assert_eq!(local.min_level(), LogLevel::Debug);
}

// ---------------------------------------------------------------------------
// docs/faq.md —— 错误枚举匹配
// ---------------------------------------------------------------------------

#[test]
fn doc_yar_error_matching() {
    let err = YarClient::connect("http://example.com:80").unwrap_err();
    match err {
        YarError::Connection(msg) => assert!(msg.contains("HTTP")),
        other => panic!("expected Connection error, got {:?}", other),
    }

    // 无端口号的 host 会被拒绝
    let err2 = YarClient::connect("localhost").unwrap_err();
    assert!(matches!(err2, YarError::Connection(_)));
}
