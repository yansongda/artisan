//! ParserPlugin 全链路集成测试（wiremock）
//!
//! 锁定 0.17.0 语义：响应解析由链尾挂载的 [`ParserPlugin`] 完成，
//! `IgniteCore` 仅执行 HTTP 请求。覆盖：忘挂负例、JSON/XML/Query 解包、
//! Response/NoRequest/Custom 方向、守卫负例。

use artisan_http::direction::{Destination, Direction, DirectionKind};
use artisan_http::event::{Event, EventListener};
use artisan_http::packer::Packer;
use artisan_http::packers::{QueryPacker, XmlPacker};
use artisan_http::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{Artful, ArtfulError, Plugin, Rocket, flow_ctrl::Next};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 设置 HTTP 方法和 URL 的插件
struct MethodUrlPlugin {
    method: reqwest::Method,
    url: String,
}

#[async_trait]
impl Plugin for MethodUrlPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.method = self.method.clone();
        rocket.config.url = self.url.clone();
        next.call(rocket).await
    }
}

/// 设置 method/url/direction 的插件
struct ConfigPlugin {
    method: reqwest::Method,
    url: String,
    direction: DirectionKind,
}

#[async_trait]
impl Plugin for ConfigPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.method = self.method.clone();
        rocket.config.url = self.url.clone();
        rocket.config.direction = self.direction.clone();
        next.call(rocket).await
    }
}

/// 将 direction 置为 NoRequest 的插件
struct SetNoRequestPlugin;

#[async_trait]
impl Plugin for SetNoRequestPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.direction = DirectionKind::NoRequest;
        next.call(rocket).await
    }
}

/// 链上替换 rocket.packer 的插件（packer 经构造参数注入）
struct ReplacePackerPlugin(Arc<dyn Packer>);

#[async_trait]
impl Plugin for ReplacePackerPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.packer = self.0.clone();
        next.call(rocket).await
    }
}

/// 后向阶段预置 Json destination 的插件（触发 ParserPlugin 守卫）
struct PresetDestinationPlugin;

#[async_trait]
impl Plugin for PresetDestinationPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        next.call(rocket).await?;

        // 此时 IgniteCore 已执行完毕（仅发请求，不再解析）：
        // 预置非 Response 的 destination 应被链尾 ParserPlugin 拒绝
        rocket.destination = Some(Destination::Json(json!({"preset": true})));

        Ok(())
    }
}

/// 记录 Custom direction 是否被调用
#[derive(Debug)]
struct CustomRecordingDirection {
    called: Arc<Mutex<bool>>,
}

#[async_trait]
impl Direction for CustomRecordingDirection {
    async fn parse(&self, rocket: &mut Rocket) -> artisan_http::Result<Destination> {
        *self.called.lock().unwrap() = true;

        let text = rocket
            .destination_origin
            .take()
            .map(|response| response.status().to_string())
            .unwrap_or_default();

        Ok(Destination::Json(json!({"status": text})))
    }
}

/// ArtfulEnd 时观测 rocket.destination 是否为 Some(Destination::None) 的监听器
struct NoneDestinationObserver {
    observed: Arc<Mutex<Option<bool>>>,
}

impl EventListener for NoneDestinationObserver {
    fn name(&self) -> &'static str {
        "NoneDestinationObserver"
    }

    fn on_event(&self, event: &mut Event<'_>) -> artisan_http::Result<()> {
        if let Event::ArtfulEnd { rocket } = event {
            *self.observed.lock().unwrap() =
                Some(matches!(rocket.destination, Some(Destination::None)));
        }

        Ok(())
    }
}

/// 从 Destination 中解出 JSON 结果，类型不符则 panic
fn expect_json(result: Destination) -> serde_json::Value {
    match result {
        Destination::Json(json) => json,
        other => panic!("Expected JSON destination, got {:?}", other),
    }
}

// ============ 场景 1：忘挂负例 ============

#[tokio::test]
async fn chain_without_parser_plugin_returns_none_but_sends_request() {
    // 忘挂 ParserPlugin：请求正常发出（IgniteCore 仅执行 HTTP）、
    // destination 保持 None，Artful::artful 归一返回 Destination::None 且不报错
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/no-parser"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code": 0})))
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/no-parser",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        // 注意：未挂 ParserPlugin
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    // 不报错，返回 Destination::None（rocket.destination.unwrap_or_default() 归一）
    assert!(matches!(result, Destination::None));

    // 请求确实发出（服务端命中一次）
    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
}

// ============ 场景 2：默认链挂 ParserPlugin + JSON 响应 ============

#[tokio::test]
async fn parser_plugin_parses_json_response() {
    // 对齐 PHP ArtfulTest::testDefaultDirection 语义：完整链（链尾 ParserPlugin）
    // + JSON 响应 → Destination::Json 内容正确
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"code": 0, "message": "parsed"})),
        )
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/json",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    let json = expect_json(result);
    assert_eq!(json["code"], 0);
    assert_eq!(json["message"], "parsed");
}

// ============ 场景 3：XmlPacker 全链路 ============

#[tokio::test]
async fn parser_plugin_unpacks_xml_via_replaced_packer() {
    // packer 经链上插件替换为 XmlPacker + wiremock 返回 XML 体 →
    // ParserPlugin 经 rocket.packer 解包为 XML Object（全链路验证 packer 可替换语义）
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("<xml><code>0</code><ok>true</ok></xml>", "application/xml"),
        )
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/xml",
        }),
        Arc::new(ReplacePackerPlugin(Arc::new(XmlPacker))),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    // XmlPacker 输出：根元素值即结果（不含根名），叶子文本为字符串
    let json = expect_json(result);
    assert_eq!(json["code"], "0");
    assert_eq!(json["ok"], "true");
}

// ============ 场景 4：QueryPacker 全链路（raw 模式证书无损） ============

/// 自造 query 串（含 `\r\n`、`+`、`/`，多段验证 `&` 切分），
/// 对齐 PHP QueryPackerTest::testUnpackRaw 的证书逐字符无损语义
const QUERY_RAW_RESPONSE: &str = "accessType=0&signPubKeyCert=-----BEGIN CERTIFICATE-----\r\nMIIE+abc/xyz+AB==\r\n-----END CERTIFICATE-----&signature=c++EAuub/Rk==";

#[tokio::test]
async fn query_packer_raw_mode_preserves_cert_characters() {
    // packer 替换为 QueryPacker：请求体按 RFC1738 打包（wiremock 断言）、
    // 响应为 query 串；payload 预置 `_unpack_raw: true` 走 raw 模式 →
    // 证书字段逐字符无损（`\r\n`、`+`、`/` 均不被解码破坏）
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/query"))
        .and(body_string_contains("biz=test"))
        .and(body_string_contains("_unpack_raw=1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(QUERY_RAW_RESPONSE, "text/plain"))
        .mount(&mock_server)
        .await;

    // StartPlugin 将 params 初始化到 payload：`_unpack_raw` 随 payload
    // 全量传给 QueryPacker::unpack（ParserPlugin 不过滤 `_` 前缀特殊参数）
    let params = HashMap::from([
        ("_unpack_raw".to_string(), json!(true)),
        ("biz".to_string(), json!("test")),
    ]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/query",
        }),
        Arc::new(ReplacePackerPlugin(Arc::new(QueryPacker))),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    let json = expect_json(result);

    // 证书字段逐字符无损（含 \r\n、+、/）
    assert_eq!(
        json["signPubKeyCert"],
        "-----BEGIN CERTIFICATE-----\r\nMIIE+abc/xyz+AB==\r\n-----END CERTIFICATE-----"
    );
    // signature 的 `+` 与 `/` 不被破坏
    assert_eq!(json["signature"], "c++EAuub/Rk==");
    assert_eq!(json["accessType"], "0");
}

// ============ 场景 5：OriginResponseDirection ============

#[tokio::test]
async fn response_direction_wraps_origin_response() {
    // config.direction = Response + ParserPlugin → destination 为 Destination::Response
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/response"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("raw body", "text/plain"))
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(ConfigPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/response",
            direction: DirectionKind::Response,
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    let response = match result {
        Destination::Response(response) => response,
        other => panic!("Expected Response destination, got {:?}", other),
    };
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "raw body");
}

// ============ 场景 6：NoRequest + ParserPlugin ============

#[tokio::test]
async fn no_request_with_parser_plugin_sets_none_destination() {
    // NoRequest + ParserPlugin：IgniteCore 短路不发起请求（wiremock expect(0) 零命中），
    // ParserPlugin 后向经 NoHttpRequestDirection 透传 → rocket.destination
    // 变为 Some(Destination::None)（0.16.0 中保持 None，0.17.0 语义变化点）
    let mock_server = MockServer::start().await;

    // 若 NoRequest 失效发起请求，expect(0) 在 MockServer drop 时验证失败
    Mock::given(method("GET"))
        .and(path("/never"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"never": true})))
        .expect(0)
        .mount(&mock_server)
        .await;

    let observed = Arc::new(Mutex::new(None));
    let artful = Artful::builder()
        .event_listener(Arc::new(NoneDestinationObserver {
            observed: observed.clone(),
        }))
        .build()
        .unwrap();

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(SetNoRequestPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/never",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    // ArtfulEnd 监听器直接观测 rocket.destination：新语义下为 Some(Destination::None)
    assert_eq!(*observed.lock().unwrap(), Some(true));

    // 入口返回值新旧语义相同（unwrap_or_default 归一），此断言仅锁定不报错
    assert!(matches!(result, Destination::None));
}

// ============ 场景 7：Custom direction ============

#[tokio::test]
async fn custom_direction_dispatched_through_parser_plugin() {
    // Custom direction + ParserPlugin → 自定义 parse 被调用（分发发生在插件后向阶段）
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/custom"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let called = Arc::new(Mutex::new(false));
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(ConfigPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/custom",
            direction: DirectionKind::Custom(Arc::new(CustomRecordingDirection {
                called: called.clone(),
            })),
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    // 自定义 parse 确实被调用
    assert!(*called.lock().unwrap());

    let json = expect_json(result);
    assert_eq!(json["status"], "200 OK");
}

// ============ 场景 8：守卫负例 ============

#[tokio::test]
async fn parser_plugin_rejects_preset_json_destination() {
    // 守卫（对齐 PHP InvalidParamsException 9208）：链上插件后向预置
    // Some(Destination::Json(_)) → ParserPlugin 报 InvalidParameter
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/guard"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/guard",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
        // 预置插件位于 ParserPlugin 内层（链尾）：后向阶段先于 Parser 执行，
        // 预置值才能被 Parser 的守卫拦截
        Arc::new(PresetDestinationPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(matches!(
        result.unwrap_err(),
        ArtfulError::InvalidParameter { param, .. } if param == "destination"
    ));
}
