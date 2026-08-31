//! 事件系统集成测试
//!
//! 锁定事件时序契约（设计文档 §3.2 触发矩阵）：
//! 成功序列 / HTTP 失败序列 / NoRequest / HttpStart 修改生效 / ArtfulEnd 改写生效 /
//! 监听器错误传播 / HttpError 分发失败保留原始错误。

use artisan_http::direction::Destination;
use artisan_http::event::{Event, EventListener};
use artisan_http::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, StartPlugin};
use artisan_http::{Artful, ArtfulError, Plugin, Rocket, flow_ctrl::Next};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// 设置 HTTP 方法和 URL 的插件（写法对齐 lib.rs 样例）
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

/// 将 direction 置为 NoRequest 的插件
struct SetNoRequestPlugin;

#[async_trait]
impl Plugin for SetNoRequestPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.direction = artisan_http::DirectionKind::NoRequest;
        next.call(rocket).await
    }
}

/// 不调用 next、直接返回 Ok 的插件（链视为成功，链尾核心动作不执行）
struct ShortCircuitOkPlugin;

#[async_trait]
impl Plugin for ShortCircuitOkPlugin {
    async fn assembly(&self, _rocket: &mut Rocket, _next: Next<'_>) -> artisan_http::Result<()> {
        Ok(())
    }
}

// ============ 测试监听器 ============

/// 记录事件变体名的旁路监听器（恒返回 Ok）
struct RecorderListener {
    records: Arc<Mutex<Vec<&'static str>>>,
}

impl RecorderListener {
    fn new(records: Arc<Mutex<Vec<&'static str>>>) -> Arc<Self> {
        Arc::new(Self { records })
    }
}

impl EventListener for RecorderListener {
    fn name(&self) -> &'static str {
        "Recorder"
    }

    fn on_event(&self, event: &mut Event<'_>) -> artisan_http::Result<()> {
        let name = match event {
            Event::ArtfulStart { .. } => "ArtfulStart",
            Event::HttpStart { .. } => "HttpStart",
            Event::HttpEnd { .. } => "HttpEnd",
            Event::HttpError { .. } => "HttpError",
            Event::ArtfulEnd { .. } => "ArtfulEnd",
        };
        self.records.lock().unwrap().push(name);

        Ok(())
    }
}

/// HttpStart 时向 radar 注入 `x-event-test` 请求头的监听器
struct HeaderInserterListener;

impl EventListener for HeaderInserterListener {
    fn name(&self) -> &'static str {
        "HeaderInserter"
    }

    fn on_event(&self, event: &mut Event<'_>) -> artisan_http::Result<()> {
        if let Event::HttpStart { rocket } = event {
            // radar 已构建：经 reqwest Request 的 *_mut 访问器修改请求
            if let Some(radar) = rocket.radar.as_mut() {
                radar.headers_mut().insert(
                    reqwest::header::HeaderName::from_static("x-event-test"),
                    reqwest::header::HeaderValue::from_static("true"),
                );
            }
        }

        Ok(())
    }
}

/// ArtfulEnd 时改写 destination 的监听器
struct DestinationRewriterListener;

impl EventListener for DestinationRewriterListener {
    fn name(&self) -> &'static str {
        "DestinationRewriter"
    }

    fn on_event(&self, event: &mut Event<'_>) -> artisan_http::Result<()> {
        if let Event::ArtfulEnd { rocket } = event {
            rocket.destination = Some(Destination::Json(json!({"rewritten": true})));
        }

        Ok(())
    }
}

/// HttpStart 时返回 Err 的监听器（模拟监听器故障）
struct FailingListener;

impl EventListener for FailingListener {
    fn name(&self) -> &'static str {
        "Failing"
    }

    fn on_event(&self, _event: &mut Event<'_>) -> artisan_http::Result<()> {
        Err(ArtfulError::Other("listener boom".to_string()))
    }
}

/// 仅在 HttpError 分发时返回 Err 的监听器（模拟监控型监听器故障）
struct HttpErrorFailingListener;

impl EventListener for HttpErrorFailingListener {
    fn name(&self) -> &'static str {
        "HttpErrorFailing"
    }

    fn on_event(&self, event: &mut Event<'_>) -> artisan_http::Result<()> {
        if matches!(event, Event::HttpError { .. }) {
            return Err(ArtfulError::Other("listener boom".to_string()));
        }

        Ok(())
    }
}

// ============ 构造辅助 ============

/// 最小插件链：StartPlugin → MethodUrlPlugin → AddPayloadBodyPlugin → AddRadarPlugin
/// （HTTP 执行由框架自动挂载的链尾核心动作完成）
fn plugin_chain(method: reqwest::Method, url: String) -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin { method, url }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
    ]
}

/// 构建注册了指定监听器的 Artful 实例
fn artful_with_listeners(listeners: Vec<Arc<dyn EventListener>>) -> Artful {
    let mut builder = Artful::builder();
    for listener in listeners {
        builder = builder.event_listener(listener);
    }
    builder.build().unwrap()
}

fn recorder_records() -> (Arc<Mutex<Vec<&'static str>>>, Arc<dyn EventListener>) {
    let records = Arc::new(Mutex::new(Vec::new()));
    (records.clone(), RecorderListener::new(records))
}

// ============ 触发矩阵 7 场景 ============

#[tokio::test]
async fn success_path_event_sequence() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let (records, recorder) = recorder_records();
    let artful = artful_with_listeners(vec![recorder]);

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);
    let result = artful
        .artful(
            params,
            plugin_chain(reqwest::Method::POST, mock_server.uri() + "/events"),
        )
        .await
        .unwrap();

    match result {
        Destination::Json(json) => assert_eq!(json["ok"], true),
        other => panic!("expected Json destination, got {other:?}"),
    }
    assert_eq!(
        *records.lock().unwrap(),
        vec!["ArtfulStart", "HttpStart", "HttpEnd", "ArtfulEnd"]
    );
}

#[tokio::test]
async fn http_error_fires_http_error() {
    // 绑定一个临时端口后立即释放：对该端口的连接必然被拒绝
    //（不用 MockServer::drop 制造失败——其关闭是异步的，存在连接仍可达的竞态）
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = format!("http://127.0.0.1:{port}/boom");

    let (records, recorder) = recorder_records();
    let artful = artful_with_listeners(vec![recorder]);

    let result = artful
        .artful(HashMap::new(), plugin_chain(reqwest::Method::POST, url))
        .await;

    assert!(matches!(result.unwrap_err(), ArtfulError::RequestFailed(_)));
    assert_eq!(
        *records.lock().unwrap(),
        vec!["ArtfulStart", "HttpStart", "HttpError"]
    );
}

#[tokio::test]
async fn no_request_direction_no_http_events() {
    // NoRequest 方向：不发起请求，仅 Artful 生命周期事件
    let (records, recorder) = recorder_records();
    let artful = artful_with_listeners(vec![recorder]);

    let mut plugins = plugin_chain(
        reqwest::Method::GET,
        "http://nonexistent-host-12345.local/no-request".to_string(),
    );
    // SetNoRequestPlugin 置于链尾核心动作之前即可（此处放在 StartPlugin 之后）
    plugins.insert(1, Arc::new(SetNoRequestPlugin));

    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    assert!(matches!(result, Destination::None));
    assert_eq!(*records.lock().unwrap(), vec!["ArtfulStart", "ArtfulEnd"]);
}

#[tokio::test]
async fn http_start_mutation_via_radar_reaches_server() {
    let mock_server = MockServer::start().await;

    // HttpStart 中经 radar 添加的请求头应真实到达服务端
    Mock::given(method("POST"))
        .and(path("/mutate"))
        .and(header("x-event-test", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let artful = artful_with_listeners(vec![Arc::new(HeaderInserterListener)]);

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);
    let result = artful
        .artful(
            params,
            plugin_chain(reqwest::Method::POST, mock_server.uri() + "/mutate"),
        )
        .await
        .unwrap();

    match result {
        Destination::Json(json) => assert_eq!(json["ok"], true),
        other => panic!("expected Json destination, got {other:?}"),
    }
}

#[tokio::test]
async fn artful_end_can_rewrite_destination() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/rewrite"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let artful = artful_with_listeners(vec![Arc::new(DestinationRewriterListener)]);

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);
    let result = artful
        .artful(
            params,
            plugin_chain(reqwest::Method::POST, mock_server.uri() + "/rewrite"),
        )
        .await
        .unwrap();

    // 解析结果应为 Json({"ok": true})，ArtfulEnd 改写后返回 {"rewritten": true}
    match result {
        Destination::Json(json) => assert_eq!(json["rewritten"], true),
        other => panic!("expected Json destination, got {other:?}"),
    }
}

#[tokio::test]
async fn listener_error_aborts_and_propagates() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/never-reached"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let artful = artful_with_listeners(vec![Arc::new(FailingListener)]);

    let params = HashMap::from([("order_id".to_string(), json!("123"))]);
    let result = artful
        .artful(
            params,
            plugin_chain(reqwest::Method::POST, mock_server.uri() + "/never-reached"),
        )
        .await;

    match result.unwrap_err() {
        ArtfulError::EventListenerError { listener_name, .. } => {
            assert_eq!(listener_name, "Failing");
        }
        other => panic!("expected EventListenerError, got {other:?}"),
    }

    // 监听器在 execute 之前失败：服务端零请求
    match mock_server.received_requests().await {
        None => {}
        Some(requests) => assert!(requests.is_empty(), "server received requests"),
    }
}

#[tokio::test]
async fn http_error_listener_failure_preserves_original_error() {
    // HttpError 分发中监听器自身失败：传播 EventListenerError，
    // 被顶替的原始 RequestFailed 保留在 original 字段（错误链不丢失）
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = format!("http://127.0.0.1:{port}/boom");

    let artful = artful_with_listeners(vec![Arc::new(HttpErrorFailingListener)]);

    let result = artful
        .artful(HashMap::new(), plugin_chain(reqwest::Method::POST, url))
        .await;

    match result.unwrap_err() {
        ArtfulError::EventListenerError {
            listener_name,
            original,
            ..
        } => {
            assert_eq!(listener_name, "HttpErrorFailing");
            assert!(matches!(*original.unwrap(), ArtfulError::RequestFailed(_)));
        }
        other => panic!("expected EventListenerError, got {other:?}"),
    }
}

// ============ 新契约回归测试（设计文档 §6 触发矩阵）============

#[tokio::test]
async fn empty_chain_dispatches_http_start_and_fails_fast() {
    // 空插件链 + 默认方向（Json）：框架自动挂载的链尾核心动作必执行，
    // radar 缺失 fail-fast 报 MissingRequest（HttpStart 已于 radar.take
    // 之前分发；MissingRequest 属前置失败，不触发 HttpError）
    let (records, recorder) = recorder_records();
    let artful = artful_with_listeners(vec![recorder]);

    let result = artful.artful(HashMap::new(), vec![]).await;

    assert!(matches!(result.unwrap_err(), ArtfulError::MissingRequest));
    assert_eq!(*records.lock().unwrap(), vec!["ArtfulStart", "HttpStart"]);
}

#[tokio::test]
async fn no_request_plugin_returns_none() {
    // NoRequest 方向经插件写入（非空链，与 src/artful.rs 单测
    // artful_no_request_returns_none 同形，走集成入口锁定契约）：
    // 链尾核心动作直接返回，不发起请求、不触发任何 HTTP 事件
    let (records, recorder) = recorder_records();
    let artful = artful_with_listeners(vec![recorder]);

    let result = artful
        .artful(HashMap::new(), vec![Arc::new(SetNoRequestPlugin)])
        .await
        .unwrap();

    assert!(matches!(result, Destination::None));
    assert_eq!(*records.lock().unwrap(), vec!["ArtfulStart", "ArtfulEnd"]);
}

#[tokio::test]
async fn plugin_not_calling_next_skips_http_events() {
    // 插件不调用 next 直接返回 Ok：链视为成功（ArtfulEnd 照常分发），
    // 但链尾核心动作不执行，无任何 HTTP 事件；destination 保持 None
    let (records, recorder) = recorder_records();
    let artful = artful_with_listeners(vec![recorder]);

    let result = artful
        .artful(HashMap::new(), vec![Arc::new(ShortCircuitOkPlugin)])
        .await
        .unwrap();

    assert!(matches!(result, Destination::None));
    assert_eq!(*records.lock().unwrap(), vec!["ArtfulStart", "ArtfulEnd"]);
}
