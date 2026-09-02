use artisan_http::direction::Destination;
use artisan_http::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{
    Artful, ArtfulError, ClientOptions, Config, Packer, Plugin, Rocket, flow_ctrl::Next,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

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

/// 从 Destination 中解出 JSON 结果，类型不符则 panic
fn expect_json(result: Destination) -> serde_json::Value {
    match result {
        Destination::Json(json) => json,
        other => panic!("Expected JSON destination, got {:?}", other),
    }
}

#[tokio::test]
async fn test_full_pipeline() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/orders"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"code": 0, "data": "success"})),
        )
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/api/orders",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    let json = expect_json(result);
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"], "success");
}

#[tokio::test]
async fn test_pipeline_with_payload() {
    let mock_server = MockServer::start().await;

    // 请求体应包含 payload 中的业务参数
    Mock::given(method("POST"))
        .and(path("/api/test"))
        .and(body_string_contains("order_id"))
        .and(body_string_contains("amount"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
        .mount(&mock_server)
        .await;

    let params = HashMap::from([
        ("order_id".to_string(), json!("123")),
        ("amount".to_string(), json!(100)),
    ]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/api/test",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    let json = expect_json(result);
    assert_eq!(json["status"], "ok");
}

// ============ Content-Type 自动补头测试 ============

#[tokio::test]
async fn default_chain_sets_content_type() {
    let mock_server = MockServer::start().await;

    // 默认链（含 AddPayloadBodyPlugin）发出的 JSON body 应自动携带 Content-Type
    Mock::given(method("POST"))
        .and(path("/ct-default"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    // params 非空：空 payload 不打包、不补 CT
    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/ct-default",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

struct CustomContentTypePlugin;

#[async_trait]
impl Plugin for CustomContentTypePlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.add_header("Content-Type", "application/custom");
        next.call(rocket).await
    }
}

#[tokio::test]
async fn manual_content_type_not_overridden() {
    let mock_server = MockServer::start().await;

    // 用户显式设置的 Content-Type 不应被框架覆盖
    Mock::given(method("POST"))
        .and(path("/ct-custom"))
        .and(header("content-type", "application/custom"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(CustomContentTypePlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/ct-custom",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

/// 声明表单 Content-Type 的自定义 Packer
#[derive(Debug)]
struct FormPacker;

impl Packer for FormPacker {
    fn pack(
        &self,
        data: &HashMap<String, Value>,
        _params: &HashMap<String, Value>,
    ) -> artisan_http::Result<String> {
        let pairs: Vec<String> = data.iter().map(|(k, v)| format!("{k}={v}")).collect();
        Ok(pairs.join("&"))
    }

    fn unpack(&self, data: &str, _params: &HashMap<String, Value>) -> artisan_http::Result<Value> {
        serde_json::from_str(data).map_err(|e| ArtfulError::JsonDeserializeError {
            message: e.to_string(),
            source: Some(e),
        })
    }

    fn content_type(&self) -> Option<&'static str> {
        Some("application/x-www-form-urlencoded")
    }
}

struct ReplacePackerPlugin;

#[async_trait]
impl Plugin for ReplacePackerPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.packer = Arc::new(FormPacker);
        next.call(rocket).await
    }
}

#[tokio::test]
async fn custom_packer_content_type() {
    let mock_server = MockServer::start().await;

    // 自定义 Packer 声明的 MIME 应生效
    Mock::given(method("POST"))
        .and(path("/ct-form"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(ReplacePackerPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/ct-form",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

#[tokio::test]
async fn fallback_branch_sets_content_type() {
    let mock_server = MockServer::start().await;

    // 链中不含 AddPayloadBodyPlugin：AddRadarPlugin fallback 打包分支应补 Content-Type
    Mock::given(method("POST"))
        .and(path("/ct-fallback"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/ct-fallback",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

// ============ client 级配置生效测试 ============

#[tokio::test]
async fn client_timeout_takes_effect() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true}))
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&mock_server)
        .await;

    // client 级 timeout=1s < mock 延迟 2s，请求应超时失败
    let config = Config {
        http: ClientOptions {
            timeout: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let artful = Artful::with_config(config).unwrap();

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/slow",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(matches!(result.unwrap_err(), ArtfulError::RequestFailed(_)));
}

// ============ Content-Type 判重与 timeout 覆盖语义测试 ============

/// 断言请求恰好携带一个 Content-Type 头（按头名聚合计数，不区分大小写）
struct SingleContentTypeHeader;

impl Match for SingleContentTypeHeader {
    fn matches(&self, request: &Request) -> bool {
        request.headers.get_all("content-type").iter().count() == 1
    }
}

struct LowercaseContentTypePlugin;

#[async_trait]
impl Plugin for LowercaseContentTypePlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        // 小写键：判重若区分大小写，补头后会重复发送两个 Content-Type
        rocket.add_header("content-type", "application/custom");
        next.call(rocket).await
    }
}

#[tokio::test]
async fn lowercase_content_type_not_duplicated() {
    let mock_server = MockServer::start().await;

    // 恰好一个 CT 头，且值是用户显式设置的值
    Mock::given(method("POST"))
        .and(path("/ct-lowercase"))
        .and(SingleContentTypeHeader)
        .and(header("content-type", "application/custom"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(LowercaseContentTypePlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/ct-lowercase",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

#[tokio::test]
async fn request_timeout_overrides_client_timeout() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/slow-override"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true}))
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&mock_server)
        .await;

    // client 级 5s 足以容纳 2s 延迟；请求级 1s 覆盖后应超时
    let config = Config {
        http: ClientOptions {
            timeout: Some(5),
            ..Default::default()
        },
        ..Default::default()
    };
    let artful = Artful::with_config(config).unwrap();

    struct RequestTimeoutPlugin;

    #[async_trait]
    impl Plugin for RequestTimeoutPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.config.http.timeout = Some(1);
            next.call(rocket).await
        }
    }

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(RequestTimeoutPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/slow-override",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(HashMap::new(), plugins).await;

    // 请求级 timeout=1s 覆盖 client 级 5s：若覆盖语义失效，请求 2s 后成功、断言失败
    assert!(matches!(result.unwrap_err(), ArtfulError::RequestFailed(_)));
}

// ============ 错误路径：MissingRequest 与非法 JSON 响应 ============

#[tokio::test]
async fn missing_request_when_no_radar_plugin() {
    // 空链经 artful() 触发框架自动挂载的链尾核心动作：默认方向 Json，
    // radar 缺失（无 AddRadarPlugin）仍报 MissingRequest
    let plugins: Vec<Arc<dyn Plugin>> = vec![];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(matches!(result.unwrap_err(), ArtfulError::MissingRequest));
}

#[tokio::test]
async fn invalid_json_response_errors() {
    let mock_server = MockServer::start().await;

    // 响应体不是合法 JSON → JsonDirection 解析失败
    Mock::given(method("GET"))
        .and(path("/not-json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("this is not json", "text/plain"))
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/not-json",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(matches!(
        result.unwrap_err(),
        ArtfulError::JsonDeserializeError { .. }
    ));
}

// ============ 框架默认 User-Agent ============

#[tokio::test]
async fn default_user_agent_sent() {
    let mock_server = MockServer::start().await;

    // 默认 UA 应为 yansongda/artisan-http:{version}
    let expected_ua = concat!("yansongda/artisan-http:", env!("CARGO_PKG_VERSION"));
    Mock::given(method("GET"))
        .and(path("/default-ua"))
        .and(header("user-agent", expected_ua))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/default-ua",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

// ============ AddPayloadBodyPlugin 守卫与空 payload 行为 ============

#[tokio::test]
async fn preset_body_not_overridden() {
    let mock_server = MockServer::start().await;

    // config.body 已预设：AddPayloadBodyPlugin 不应打包 payload，CT 保留用户设置的值
    Mock::given(method("POST"))
        .and(path("/preset-body"))
        .and(header("content-type", "text/plain"))
        .and(body_string_contains("preset body"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    struct PresetBodyPlugin;

    #[async_trait]
    impl Plugin for PresetBodyPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.set_body("preset body");
            rocket.add_header("Content-Type", "text/plain");
            next.call(rocket).await
        }
    }

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(PresetBodyPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/preset-body",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

/// 匹配请求头中不存在指定头名的 matcher
struct AbsentHeader(&'static str);

impl Match for AbsentHeader {
    fn matches(&self, request: &Request) -> bool {
        request.headers.get(self.0).is_none()
    }
}

#[tokio::test]
async fn empty_payload_no_content_type() {
    let mock_server = MockServer::start().await;

    // 空 payload 时不打包、不补 Content-Type：请求应无 CT 头
    Mock::given(method("POST"))
        .and(path("/empty-payload"))
        .and(AbsentHeader("content-type"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/empty-payload",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

/// 匹配请求体不包含指定子串的 matcher
struct BodyNotContains(&'static str);

impl Match for BodyNotContains {
    fn matches(&self, request: &Request) -> bool {
        let body_str = std::str::from_utf8(&request.body).unwrap_or_default();
        !body_str.contains(self.0)
    }
}

#[tokio::test]
async fn start_plugin_keeps_existing_payload() {
    let mock_server = MockServer::start().await;

    // StartPlugin 在 payload 非空时不应再用 params 覆盖：body 只含 inner、不含 outer
    Mock::given(method("POST"))
        .and(path("/prefilled"))
        .and(body_string_contains("inner"))
        .and(BodyNotContains("outer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    struct PreFillPlugin;

    #[async_trait]
    impl Plugin for PreFillPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.payload.insert("inner".to_string(), json!("prefill"));
            next.call(rocket).await
        }
    }

    let params = HashMap::from([("outer".to_string(), json!("param"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(PreFillPlugin),
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/prefilled",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}

// ============ 自定义请求头全量透传 ============

#[tokio::test]
async fn custom_headers_forwarded() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/multi-headers"))
        .and(header("X-Request-Id", "req-123"))
        .and(header("X-Channel", "alipay"))
        .and(header("Authorization", "Bearer token-xyz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    struct MultiHeadersPlugin;

    #[async_trait]
    impl Plugin for MultiHeadersPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.add_header("X-Request-Id", "req-123");
            rocket.add_header("X-Channel", "alipay");
            rocket.add_header("Authorization", "Bearer token-xyz");
            next.call(rocket).await
        }
    }

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MultiHeadersPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/multi-headers",
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(params, plugins).await.unwrap();

    assert_eq!(expect_json(result)["ok"], true);
}
