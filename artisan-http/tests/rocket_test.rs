use artisan_http::{ClientOptions, RequestOptions, Rocket, RocketConfig};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_rocket_config_default() {
    let config = RocketConfig::default();
    assert_eq!(config.method, reqwest::Method::POST);
    assert_eq!(config.url, "");
    assert!(config.headers.is_empty());
    assert!(config.body.is_none());
}

#[test]
fn test_rocket_new() {
    let mut params = HashMap::new();
    params.insert("key".to_string(), json!("value"));

    let rocket = Rocket::new(params);

    assert_eq!(rocket.config.method, reqwest::Method::POST);
    assert_eq!(rocket.config.url, "");
    assert!(rocket.radar.is_none());
    assert!(rocket.destination_origin.is_none());
    assert!(rocket.destination.is_none());
}

#[test]
fn test_http_options_default() {
    let client = ClientOptions::default();
    assert!(client.timeout.is_none());
    assert!(client.connect_timeout.is_none());
    assert!(client.pool_idle_timeout.is_none());
    assert!(client.pool_max_idle_per_host.is_none());
    assert!(client.user_agent.is_none());

    let request = RequestOptions::default();
    assert!(request.timeout.is_none());
}

#[test]
fn test_client_options_with_timeout() {
    let http = ClientOptions {
        timeout: Some(30),
        connect_timeout: Some(10),
        ..Default::default()
    };
    assert_eq!(http.timeout, Some(30));
    assert_eq!(http.connect_timeout, Some(10));
}

#[test]
fn test_rocket_from_hashmap() {
    let mut params = HashMap::new();
    params.insert("test".to_string(), json!("data"));

    let rocket: Rocket = params.clone().into();
    let retrieved = rocket.get_params();
    assert_eq!(retrieved.get("test"), Some(&json!("data")));
}

#[test]
fn test_rocket_merge_params_to_payload() {
    let mut params = HashMap::new();
    params.insert("merged".to_string(), json!("value"));

    let mut rocket = Rocket::new(params);

    rocket.merge_params_to_payload();
    assert!(rocket.payload.contains_key("merged"));
    // params 保持不变
    assert!(rocket.get_params().contains_key("merged"));
}

#[test]
fn test_rocket_set_method() {
    let mut rocket = Rocket::new(HashMap::new());
    rocket.set_method(reqwest::Method::GET);
    assert_eq!(rocket.config.method, reqwest::Method::GET);
}

#[test]
fn test_rocket_set_url() {
    let mut rocket = Rocket::new(HashMap::new());
    rocket.set_url("https://example.com/api");
    assert_eq!(rocket.config.url, "https://example.com/api");
}

#[test]
fn test_rocket_add_header() {
    let mut rocket = Rocket::new(HashMap::new());
    rocket.add_header("Content-Type", "application/json");
    rocket.add_header("Authorization", "Bearer token");

    assert_eq!(
        rocket.config.headers.get("Content-Type"),
        Some(&"application/json".to_string())
    );
    assert_eq!(
        rocket.config.headers.get("Authorization"),
        Some(&"Bearer token".to_string())
    );
}

#[test]
fn test_rocket_set_body() {
    let mut rocket = Rocket::new(HashMap::new());
    rocket.set_body("{\"data\": \"test\"}");
    assert_eq!(rocket.config.body, Some("{\"data\": \"test\"}".to_string()));
}

#[test]
fn test_rocket_set_timeout() {
    let mut rocket = Rocket::new(HashMap::new());
    rocket.set_timeout(60);
    assert_eq!(rocket.config.http.timeout, Some(60));
}

#[test]
fn test_rocket_get_params() {
    let mut params = HashMap::new();
    params.insert("key1".to_string(), json!("value1"));
    params.insert("key2".to_string(), json!(123));

    let rocket = Rocket::new(params);
    let retrieved = rocket.get_params();

    assert_eq!(retrieved.len(), 2);
    assert_eq!(retrieved.get("key1"), Some(&json!("value1")));
    assert_eq!(retrieved.get("key2"), Some(&json!(123)));
}

#[test]
fn test_rocket_debug() {
    let mut params = HashMap::new();
    params.insert("test".to_string(), json!("value"));

    let rocket = Rocket::new(params);
    let debug_str = format!("{:?}", rocket);

    assert!(debug_str.contains("params"));
    assert!(debug_str.contains("payload"));
    assert!(debug_str.contains("config"));
}

#[test]
fn test_rocket_convenience_methods_chained() {
    let mut rocket = Rocket::new(HashMap::new());

    rocket.set_method(reqwest::Method::PUT);
    rocket.set_url("https://api.example.com/resource");
    rocket.add_header("X-Custom", "custom-value");
    rocket.add_header("Accept", "application/json");
    rocket.set_body("{\"update\": true}");
    rocket.set_timeout(30);

    assert_eq!(rocket.config.method, reqwest::Method::PUT);
    assert_eq!(rocket.config.url, "https://api.example.com/resource");
    assert_eq!(rocket.config.headers.len(), 2);
    assert!(rocket.config.body.is_some());
    assert_eq!(rocket.config.http.timeout, Some(30));
}

#[test]
fn test_rocket_add_header_overwrite() {
    let mut rocket = Rocket::new(HashMap::new());

    rocket.add_header("X-Test", "first-value");
    assert_eq!(
        rocket.config.headers.get("X-Test"),
        Some(&"first-value".to_string())
    );

    rocket.add_header("X-Test", "second-value");
    assert_eq!(
        rocket.config.headers.get("X-Test"),
        Some(&"second-value".to_string())
    );
}

#[test]
fn test_rocket_set_body_overwrite() {
    let mut rocket = Rocket::new(HashMap::new());

    rocket.set_body("first body");
    assert_eq!(rocket.config.body, Some("first body".to_string()));

    rocket.set_body("second body");
    assert_eq!(rocket.config.body, Some("second body".to_string()));
}

#[test]
fn test_client_options_pool_settings() {
    let http = ClientOptions {
        pool_idle_timeout: Some(90),
        pool_max_idle_per_host: Some(20),
        ..Default::default()
    };
    assert_eq!(http.pool_idle_timeout, Some(90));
    assert_eq!(http.pool_max_idle_per_host, Some(20));
}

#[test]
fn test_rocket_config_direction_default() {
    use artisan_http::DirectionKind;

    let config = RocketConfig::default();
    assert!(matches!(config.direction, DirectionKind::Json));
}

#[test]
fn test_rocket_config_custom_direction() {
    use artisan_http::DirectionKind;

    let config = RocketConfig {
        direction: DirectionKind::Response,
        ..Default::default()
    };
    assert!(matches!(config.direction, DirectionKind::Response));
}
