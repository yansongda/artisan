use artisan_http::Rocket;
use artisan_http::direction::{Destination, Direction, DirectionKind};
use artisan_http::plugins::{AddRadarPlugin, ParserPlugin};
use artisan_http::{Artful, ArtfulError, Plugin, flow_ctrl::Next};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use wiremock::matchers::{method, path};
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

#[derive(Debug)]
struct CustomJsonDirection {
    prefix: String,
}

#[async_trait]
impl Direction for CustomJsonDirection {
    async fn parse(&self, rocket: &mut Rocket) -> artisan_http::Result<Destination> {
        match rocket.destination_origin.take() {
            Some(response) => {
                let text = response
                    .text()
                    .await
                    .map_err(artisan_http::ArtfulError::RequestFailed)?;
                let mut json: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                    artisan_http::ArtfulError::JsonDeserializeError {
                        message: e.to_string(),
                        source: Some(e),
                    }
                })?;
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("_custom_prefix".to_string(), json!(self.prefix.clone()));
                }
                Ok(Destination::Json(json))
            }
            None => Err(artisan_http::ArtfulError::MissingResponse),
        }
    }
}

#[derive(Debug)]
struct FailingDirection;

#[async_trait]
impl Direction for FailingDirection {
    async fn parse(&self, _rocket: &mut Rocket) -> artisan_http::Result<Destination> {
        Err(artisan_http::ArtfulError::DirectionParseError(
            "Custom parse failed".to_string(),
        ))
    }
}

#[test]
fn test_custom_direction_kind_creation() {
    let custom = Arc::new(CustomJsonDirection {
        prefix: "test_prefix".to_string(),
    });
    let kind = DirectionKind::Custom(custom);
    assert!(matches!(kind, DirectionKind::Custom(_)));
}

#[tokio::test]
async fn custom_direction_executes_in_chain() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/custom-direction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: mock_server.uri() + "/custom-direction",
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let mut rocket = Rocket::new(HashMap::new());
    rocket.config.direction = DirectionKind::Custom(Arc::new(CustomJsonDirection {
        prefix: "prefix1".to_string(),
    }));

    let mut ctrl = artisan_http::FlowCtrl::new(plugins);
    ctrl.call_next(&mut rocket).await.unwrap();

    let destination = rocket.destination.expect("destination should be set");
    if let Destination::Json(json) = destination {
        assert_eq!(json["_custom_prefix"], "prefix1");
        assert_eq!(json["ok"], true);
    } else {
        panic!("Expected JSON destination");
    }
}

#[tokio::test]
async fn custom_direction_error_propagates() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/failing-direction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&mock_server)
        .await;

    let mut rocket = Rocket::new(HashMap::new());
    rocket.config.method = reqwest::Method::GET;
    rocket.config.url = mock_server.uri() + "/failing-direction";
    rocket.config.direction = DirectionKind::Custom(Arc::new(FailingDirection));

    let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(AddRadarPlugin), Arc::new(ParserPlugin)];
    let mut ctrl = artisan_http::FlowCtrl::new(plugins);
    let result = ctrl.call_next(&mut rocket).await;

    assert!(matches!(
        result.unwrap_err(),
        ArtfulError::DirectionParseError(_)
    ));
}

#[tokio::test]
async fn no_request_skips_http_and_keeps_chain() {
    // NoRequest 不发起 HTTP 请求，链路继续穿透，destination 为 None
    struct NoRequestPlugin;

    #[async_trait]
    impl Plugin for NoRequestPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            rocket.config.direction = DirectionKind::NoRequest;
            next.call(rocket).await
        }
    }

    struct MarkAfterParserPlugin;

    #[async_trait]
    impl Plugin for MarkAfterParserPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            // 后向阶段：ParserPlugin 已返回，若其发起了请求 radar 已被消费
            assert!(rocket.destination.is_none());
            rocket
                .payload
                .insert("after_parser".to_string(), json!(true));
            next.call(rocket).await
        }
    }

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(NoRequestPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            // 指向不存在的 host：若 NoRequest 失效发起请求，链路将报错
            url: "http://nonexistent-host-12345.local/no-request".to_string(),
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
        Arc::new(MarkAfterParserPlugin),
    ];

    let artful = Artful::new().unwrap();
    let result = artful.artful(HashMap::new(), plugins).await.unwrap();

    // artful() 返回 rocket.destination.unwrap_or_default()，NoRequest 时应为 Destination::None
    assert!(matches!(result, Destination::None));
}

#[tokio::test]
async fn response_direction_consumes_origin() {
    // DirectionKind::Response 解析后原始响应被移入 destination，destination_origin 变为 None
    struct AssertOriginTakenPlugin;

    #[async_trait]
    impl Plugin for AssertOriginTakenPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
            next.call(rocket).await?;
            assert!(rocket.destination_origin.is_none());
            assert!(matches!(rocket.destination, Some(Destination::Response(_))));
            Ok(())
        }
    }

    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/raw-take"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("raw response", "text/plain"))
        .mount(&mock_server)
        .await;

    let mut rocket = Rocket::new(HashMap::new());
    rocket.config.method = reqwest::Method::GET;
    rocket.config.url = mock_server.uri() + "/raw-take";
    rocket.config.direction = DirectionKind::Response;

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
        Arc::new(AssertOriginTakenPlugin),
    ];

    let mut ctrl = artisan_http::FlowCtrl::new(plugins);
    ctrl.call_next(&mut rocket).await.unwrap();

    let response = match rocket
        .destination
        .take()
        .expect("destination should be set")
    {
        Destination::Response(response) => response,
        other => panic!("Expected Response destination, got {:?}", other),
    };
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "raw response");
}
