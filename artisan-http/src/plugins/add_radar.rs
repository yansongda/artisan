//! 构建 HTTP Request 插件
//!
//! 根据 `RocketConfig` 构建 HTTP Request 对象。
//!
//! # 行为
//!
//! - 使用 `rocket.client` 与 config.method、config.url
//! - 添加 config.headers
//! - 设置请求体（config.body 或 payload）
//! - body 未设置且 payload 非空时走 fallback 打包：请求头缺失 `Content-Type`
//!   时按 packer 声明的 [`crate::packer::Packer::content_type`] 直接补到 request_builder
//!   （该分支位于 headers 遍历之后，写回 `config.headers` 不会再生效）
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
        } else if !rocket.payload.is_empty() {
            let body = rocket.packer.pack(&rocket.payload)?;

            // 判重按头名不区分大小写（该分支位于 headers 遍历之后，直接补到 request_builder）
            if !rocket.has_header("Content-Type") {
                if let Some(ct) = rocket.packer.content_type() {
                    request_builder = request_builder.header("Content-Type", ct);
                }
            }

            request_builder = request_builder.body(body);
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
    use serde_json::json;
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
    async fn fallback_packs_payload_with_content_type() {
        // body 未设置且 payload 非空:fallback 打包并补 CT 到 request_builder
        let mut rocket = Rocket::new(HashMap::new());
        rocket.set_url("http://example.com/anything");
        rocket.payload.insert("order_id".to_string(), json!("123"));

        drive(&mut rocket).await.unwrap();

        let request = rocket.radar.take().expect("radar should be built");
        let body = request.body().and_then(|b| b.as_bytes()).expect("body");
        let value: serde_json::Value = serde_json::from_slice(body).unwrap();
        assert_eq!(value["order_id"], "123");
        assert_eq!(
            request
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
    }

    #[tokio::test]
    async fn fallback_respects_existing_content_type() {
        // 用户已显式设置 CT(小写):fallback 不应再补,最终只有一个 CT 头
        let mut rocket = Rocket::new(HashMap::new());
        rocket.set_url("http://example.com/anything");
        rocket.payload.insert("order_id".to_string(), json!("123"));
        rocket.add_header("content-type", "application/custom");

        drive(&mut rocket).await.unwrap();

        let request = rocket.radar.take().expect("radar should be built");
        assert_eq!(request.headers().get_all("content-type").iter().count(), 1);
        assert_eq!(
            request
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/custom")
        );
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
