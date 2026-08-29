//! Payload Body 插件
//!
//! 将 payload 序列化为 HTTP 请求体。
//!
//! # 行为
//!
//! - 仅在 `rocket.config.body` 未设置时生效
//! - 使用 rocket.packer 序列化 payload
//! - 设置结果到 `rocket.config.body`
//! - 请求头缺失 `Content-Type` 时，按 packer 声明的 [`Packer::content_type`] 补填（不覆盖用户显式设置）

use async_trait::async_trait;

use crate::Rocket;
use crate::flow_ctrl::Next;
use crate::plugin::Plugin;

/// 添加 payload body 插件
#[derive(Clone, Copy, Debug, Default)]
pub struct AddPayloadBodyPlugin;

#[async_trait]
impl Plugin for AddPayloadBodyPlugin {
    fn name(&self) -> &'static str {
        "AddPayloadBodyPlugin"
    }

    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
        if rocket.config.body.is_none() && !rocket.payload.is_empty() {
            rocket.config.body = Some(rocket.packer.pack(&rocket.payload)?);

            // 判重按头名不区分大小写，用户以任意大小写键显式设置的值都不覆盖
            if let Some(ct) = rocket.packer.content_type() {
                if !rocket.has_header("Content-Type") {
                    rocket
                        .config
                        .headers
                        .insert("Content-Type".to_string(), ct.to_string());
                }
            }
        }

        next.call(rocket).await
    }
}
