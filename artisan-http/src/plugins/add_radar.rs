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

            if !rocket.config.headers.contains_key("Content-Type") {
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
