use artisan_http::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{Artful, ArtfulError, Plugin, Rocket, Shortcut, flow_ctrl::Next};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

/// 记录 get_plugins 收到的 params，供断言透传语义
struct RecordingShortcut {
    method: reqwest::Method,
    url: String,
    received_params: Arc<Mutex<Option<HashMap<String, Value>>>>,
}

impl Shortcut for RecordingShortcut {
    fn get_plugins(&self, params: &HashMap<String, Value>) -> Vec<Arc<dyn Plugin>> {
        *self.received_params.lock().unwrap() = Some(params.clone());

        vec![
            Arc::new(StartPlugin),
            Arc::new(MethodUrlPlugin {
                method: self.method.clone(),
                url: self.url.clone(),
            }),
            Arc::new(AddPayloadBodyPlugin),
            Arc::new(AddRadarPlugin),
            Arc::new(ParserPlugin),
        ]
    }
}

/// 包含注定失败插件的 Shortcut
struct FailingShortcut {
    method: reqwest::Method,
    url: String,
}

struct FailingPlugin;

#[async_trait]
impl Plugin for FailingPlugin {
    async fn assembly(&self, _rocket: &mut Rocket, _next: Next<'_>) -> artisan_http::Result<()> {
        Err(ArtfulError::Other("shortcut plugin failed".to_string()))
    }
}

impl Shortcut for FailingShortcut {
    fn get_plugins(&self, _params: &HashMap<String, Value>) -> Vec<Arc<dyn Plugin>> {
        vec![
            Arc::new(StartPlugin),
            Arc::new(MethodUrlPlugin {
                method: self.method.clone(),
                url: self.url.clone(),
            }),
            Arc::new(FailingPlugin),
            Arc::new(AddRadarPlugin),
            Arc::new(ParserPlugin),
        ]
    }
}

#[tokio::test]
async fn test_artful_shortcut_full_chain() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/shortcut/orders"))
        .and(header("content-type", "application/json"))
        .and(body_string_contains("order_id"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code": 0})))
        .mount(&mock_server)
        .await;

    let params = HashMap::from([
        ("order_id".to_string(), json!("123")),
        ("amount".to_string(), json!(100)),
    ]);

    let shortcut = RecordingShortcut {
        method: reqwest::Method::POST,
        url: mock_server.uri() + "/shortcut/orders",
        received_params: Arc::new(Mutex::new(None)),
    };

    let artful = Artful::new().unwrap();
    let result = artful.shortcut(shortcut, params).await.unwrap();

    if let artisan_http::Destination::Json(json) = result {
        assert_eq!(json["code"], 0);
    } else {
        panic!("Expected JSON destination");
    }
}

#[tokio::test]
async fn test_shortcut_receives_params() {
    // get_plugins 收到的 params 应与传入 Artful::shortcut 的完全一致
    let params = HashMap::from([("order_id".to_string(), json!("123"))]);

    let shortcut = RecordingShortcut {
        method: reqwest::Method::POST,
        url: "http://unused.local/never-called".to_string(),
        received_params: Arc::new(Mutex::new(None)),
    };
    let received = shortcut.received_params.clone();

    let artful = Artful::new().unwrap();
    // URL 不可达，请求会失败，但 params 在请求前已透传给 get_plugins
    let _ = artful.shortcut(shortcut, params.clone()).await;

    let received = received
        .lock()
        .unwrap()
        .clone()
        .expect("get_plugins should be called");
    assert_eq!(received, params);
}

#[tokio::test]
async fn test_shortcut_plugin_error_propagates() {
    let shortcut = FailingShortcut {
        method: reqwest::Method::GET,
        url: "http://unused.local/failing".to_string(),
    };

    let artful = Artful::new().unwrap();
    let result = artful.shortcut(shortcut, HashMap::new()).await;

    assert!(matches!(result.unwrap_err(), ArtfulError::Other(_)));
}
