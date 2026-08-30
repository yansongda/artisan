use artisan_http::FlowCtrl;
use artisan_http::Rocket;
use artisan_http::direction::{Destination, DirectionKind};
use artisan_http::plugins::{AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{Artful, ArtfulError, ClientOptions, Config, Plugin, flow_ctrl::Next};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{header, method, path};
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

/// 从 Destination 中解出 JSON 结果，类型不符则 panic
fn expect_json(result: Destination) -> serde_json::Value {
    match result {
        Destination::Json(json) => json,
        other => panic!("Expected JSON destination, got {:?}", other),
    }
}

#[tokio::test]
async fn test_artisan_basic() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code": 0, "message": "hello artisan", "success": true})),
        )
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/test",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    let json = expect_json(result);
    assert_eq!(json["code"], 0);
    assert_eq!(json["message"], "hello artisan");
    assert_eq!(json["success"], true);
}

#[tokio::test]
async fn test_artisan_with_response_direction() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/raw"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("raw response", "text/plain"))
        .mount(&mock_server)
        .await;

    let mut rocket = Rocket::new(HashMap::new());
    rocket.config.method = reqwest::Method::GET;
    rocket.config.url = mock_server.uri() + "/raw";
    rocket.config.direction = DirectionKind::Response;

    let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(AddRadarPlugin), Arc::new(ParserPlugin)];

    let mut ctrl = FlowCtrl::new(plugins);
    ctrl.call_next(&mut rocket).await.unwrap();

    let response = match rocket.destination.expect("destination should be set") {
        Destination::Response(response) => response,
        other => panic!("Expected Response destination, got {:?}", other),
    };
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "raw response");
}

// ============ Artful 实例配置链路测试 ============

#[tokio::test]
async fn test_artful_with_client_takes_effect() {
    let mock_server = MockServer::start().await;

    // with_client 注入的自定义 client 应贯穿 Artful 链路：
    // 自定义 User-Agent（ClientOptions 之外的 client 级能力）应透传到发出的请求
    Mock::given(method("GET"))
        .and(path("/with-client"))
        .and(header("user-agent", "custom-agent/7.7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let config = Config {
        http: ClientOptions {
            timeout: Some(30),
            ..Default::default()
        },
        ..Default::default()
    };
    let custom_client = reqwest::Client::builder()
        .user_agent("custom-agent/7.7")
        .build()
        .unwrap();
    let artful = Artful::with_client(config, custom_client);

    // config() 读回构造时传入的配置（注意：config.http 不作用于注入的 client）
    assert_eq!(artful.config().http.timeout, Some(30));

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/with-client",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    let json = expect_json(result);
    assert_eq!(json["ok"], true);
}

// ============ Artful raw 方法测试 ============

#[tokio::test]
async fn test_artful_raw_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/raw-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": "ok"})))
        .mount(&mock_server)
        .await;

    let artful = Artful::new().unwrap();
    let request = artful
        .client()
        .get(mock_server.uri() + "/raw-test")
        .build()
        .unwrap();

    let response = artful.raw(request).await.unwrap();
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["data"], "ok");
}

#[tokio::test]
async fn test_artful_raw_with_headers() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/headers-test"))
        .and(header("X-Custom", "test-value"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;

    let artful = Artful::new().unwrap();
    let request = artful
        .client()
        .post(mock_server.uri() + "/headers-test")
        .header("X-Custom", "test-value")
        .build()
        .unwrap();

    let response = artful.raw(request).await.unwrap();
    assert_eq!(response.status(), 200);
}

// ============ 插件执行失败错误传播测试 ============

#[tokio::test]
async fn test_plugin_error_propagation() {
    struct ErrorPlugin {
        message: String,
    }

    #[async_trait]
    impl Plugin for ErrorPlugin {
        async fn assembly(
            &self,
            _rocket: &mut Rocket,
            _next: Next<'_>,
        ) -> artisan_http::Result<()> {
            Err(ArtfulError::PluginExecutionError {
                plugin_name: "ErrorPlugin".to_string(),
                message: self.message.clone(),
                source: None,
            })
        }
    }

    struct SuccessPlugin;

    #[async_trait]
    impl Plugin for SuccessPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.payload.insert("success".to_string(), json!(true));
            next.call(rocket).await
        }
    }

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(SuccessPlugin),
        Arc::new(ErrorPlugin {
            message: "intentional failure".to_string(),
        }),
        Arc::new(SuccessPlugin), // 这个插件不会执行
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(matches!(error, ArtfulError::PluginExecutionError { .. }));
}

#[tokio::test]
async fn test_plugin_chain_stops_on_error() {
    struct FirstPlugin;
    struct FailingPlugin;
    struct NeverRunPlugin;

    #[async_trait]
    impl Plugin for FirstPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.payload.insert("first".to_string(), json!(1));
            next.call(rocket).await
        }
    }

    #[async_trait]
    impl Plugin for FailingPlugin {
        async fn assembly(
            &self,
            _rocket: &mut Rocket,
            _next: Next<'_>,
        ) -> artisan_http::Result<()> {
            Err(ArtfulError::Other("plugin failed".to_string()))
        }
    }

    #[async_trait]
    impl Plugin for NeverRunPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.payload.insert("never_run".to_string(), json!(true));
            next.call(rocket).await
        }
    }

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(FirstPlugin),
        Arc::new(FailingPlugin),
        Arc::new(NeverRunPlugin),
    ];

    let mut rocket = Rocket::new(HashMap::new());
    let mut ctrl = FlowCtrl::new(plugins);

    let result = ctrl.call_next(&mut rocket).await;

    assert!(result.is_err());
    assert!(rocket.payload.contains_key("first"));
    assert!(!rocket.payload.contains_key("never_run"));
}

// ============ HTTP 请求失败测试 ============

#[tokio::test]
async fn test_http_404_response() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/not-found"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"error": "Not Found", "status": "not_found"})),
        )
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/not-found",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    // 404 不会返回错误，而是正常解析响应
    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    let json = expect_json(result);
    assert_eq!(json["error"], "Not Found");
    assert_eq!(json["status"], "not_found");
}

#[tokio::test]
async fn test_http_500_response() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/server-error"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({"error": "Internal Server Error", "status": "server_error"})),
        )
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: mock_server.uri() + "/server-error",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    // 500 不会返回错误，而是正常解析响应
    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    let json = expect_json(result);
    assert_eq!(json["error"], "Internal Server Error");
    assert_eq!(json["status"], "server_error");
}

#[tokio::test]
async fn test_http_timeout_response() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200).set_delay(Duration::from_secs(5)), // 5秒延迟
        )
        .mount(&mock_server)
        .await;

    struct TimeoutUrlPlugin {
        url: String,
        timeout: u64,
    }

    #[async_trait]
    impl Plugin for TimeoutUrlPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.config.method = reqwest::Method::GET;
            rocket.config.url = self.url.clone();
            rocket.config.http.timeout = Some(self.timeout);
            next.call(rocket).await
        }
    }

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(TimeoutUrlPlugin {
            url: mock_server.uri() + "/slow",
            timeout: 1, // 1秒超时
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await;

    // 请求应该超时失败
    assert!(result.is_err());
    let error = result.unwrap_err();
    // reqwest 超时错误会被包装成 RequestFailed
    assert!(matches!(error, ArtfulError::RequestFailed(_)));
}

#[tokio::test]
async fn test_http_invalid_url() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "not-a-valid-url".to_string(),
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_http_nonexistent_host() {
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "http://nonexistent-host-12345.local/test".to_string(),
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(result.is_err());
}

// ============ Artful::with_builder 测试 ============

#[tokio::test]
async fn with_builder_applies_config_http() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/slow-builder"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": true}))
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&mock_server)
        .await;

    // 空回调：with_builder 等价于 with_config，config.http.timeout 生效
    let config = Config {
        http: ClientOptions {
            timeout: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let artful = Artful::with_builder(config, |builder| builder).unwrap();

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/slow-builder",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(HashMap::new(), plugins).await;

    assert!(matches!(result.unwrap_err(), ArtfulError::RequestFailed(_)));
}

#[tokio::test]
async fn with_builder_customization_overrides() {
    let mock_server = MockServer::start().await;

    // 回调叠加生效：自定义 UA 覆盖框架默认 UA
    Mock::given(method("GET"))
        .and(path("/customized"))
        .and(header("user-agent", "custom-agent/7.7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let config = Config {
        http: ClientOptions {
            timeout: Some(30),
            ..Default::default()
        },
        ..Default::default()
    };
    let artful =
        Artful::with_builder(config, |builder| builder.user_agent("custom-agent/7.7")).unwrap();

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/customized",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    assert!(matches!(result, artisan_http::Destination::Json(_)));
}
