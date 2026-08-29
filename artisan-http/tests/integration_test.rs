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
use wiremock::matchers::{header, method, path};
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

    assert!(matches!(result, Destination::Json(_)));
}

#[tokio::test]
async fn test_pipeline_with_payload() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/test"))
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

    assert!(matches!(result, Destination::Json(_)));
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

    assert!(matches!(result, Destination::Json(_)));
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

    assert!(matches!(result, Destination::Json(_)));
}

/// 声明表单 Content-Type 的自定义 Packer
#[derive(Debug)]
struct FormPacker;

impl Packer for FormPacker {
    fn pack(&self, data: &HashMap<String, Value>) -> artisan_http::Result<String> {
        let pairs: Vec<String> = data.iter().map(|(k, v)| format!("{k}={v}")).collect();
        Ok(pairs.join("&"))
    }

    fn unpack(&self, data: &str) -> artisan_http::Result<Value> {
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

    assert!(matches!(result, Destination::Json(_)));
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

    assert!(matches!(result, Destination::Json(_)));
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

    assert!(matches!(result, Destination::Json(_)));
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
