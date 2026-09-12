//! 构建 HTTP Request 插件
//!
//! 根据 `RocketConfig` 构建 HTTP Request 对象。
//!
//! # 行为
//!
//! - 使用 `rocket.client` 与 config.method、config.url
//! - 添加 config.headers
//! - 设置请求体（仅 `config.body`；payload 的序列化由
//!   [`AddPayloadBodyPlugin`](crate::plugins::AddPayloadBodyPlugin) 负责）
//! - 应用 config.http.timeout
//! - 结果存入 rocket.radar

use async_trait::async_trait;
use std::time::Duration;

use crate::Rocket;
use crate::flow_ctrl::Next;
use crate::plugin::Plugin;

/// 构建 HTTP Request 插件
#[derive(Clone, Copy, Debug, Default)]
pub struct AddRadarPlugin;

#[async_trait]
impl Plugin for AddRadarPlugin {
    fn name(&self) -> &'static str {
        "AddRadarPlugin"
    }

    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
        let mut request_builder = rocket
            .client
            .request(rocket.config.method.clone(), &rocket.config.url);

        for (key, value) in &rocket.config.headers {
            request_builder = request_builder.header(key, value);
        }

        if let Some(body) = &rocket.config.body {
            request_builder = request_builder.body(body.clone());
        }

        if let Some(timeout) = rocket.config.http.timeout {
            request_builder = request_builder.timeout(Duration::from_secs(timeout));
        }

        let request = request_builder
            .build()
            .map_err(|e| crate::error::ArtfulError::RequestBuildError { source: e })?;
        rocket.radar = Some(request);

        next.call(rocket).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_ctrl::FlowCtrl;
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn drive(rocket: &mut Rocket) -> crate::Result<()> {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(AddRadarPlugin)];
        FlowCtrl::new(plugins).call_next(rocket).await
    }

    #[tokio::test]
    async fn builds_request_from_config() {
        let mut rocket = Rocket::new(HashMap::new());
        rocket.set_method(reqwest::Method::PUT);
        rocket.set_url("http://example.com/anything");
        rocket.add_header("X-Test", "1");
        rocket.set_body("preset body");
        rocket.set_timeout(7);

        drive(&mut rocket).await.unwrap();

        let request = rocket.radar.take().expect("radar should be built");
        assert_eq!(*request.method(), reqwest::Method::PUT);
        assert_eq!(request.url().as_str(), "http://example.com/anything");
        assert_eq!(
            request
                .headers()
                .get("x-test")
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
        assert_eq!(
            request.body().and_then(|b| b.as_bytes()),
            Some(&b"preset body"[..])
        );
        assert_eq!(request.timeout(), Some(&Duration::from_secs(7)));
    }

    #[tokio::test]
    async fn empty_payload_no_body() {
        // payload 为空且未设置 body:请求不应携带 body,也不补 CT
        let mut rocket = Rocket::new(HashMap::new());
        rocket.set_url("http://example.com/anything");

        drive(&mut rocket).await.unwrap();

        let request = rocket.radar.take().expect("radar should be built");
        assert!(request.body().is_none());
        assert!(request.headers().get("content-type").is_none());
    }

    #[tokio::test]
    async fn build_error_propagates_on_invalid_url() {
        let mut rocket = Rocket::new(HashMap::new());
        rocket.set_url("not a valid url");

        let result = drive(&mut rocket).await;

        assert!(matches!(
            result.unwrap_err(),
            crate::error::ArtfulError::RequestBuildError { .. }
        ));
    }
}
