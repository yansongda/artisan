//! 初始化插件
//!
//! 请求链的起点插件，负责将原始参数初始化到 payload。
//!
//! # 行为
//!
//! 将 rocket.params 复制到 rocket.payload，使 payload 成为可修改的工作参数。

use async_trait::async_trait;

use crate::Rocket;
use crate::flow_ctrl::Next;
use crate::plugin::Plugin;

/// 初始化插件
#[derive(Clone, Copy, Debug, Default)]
pub struct StartPlugin;

#[async_trait]
impl Plugin for StartPlugin {
    fn name(&self) -> &'static str {
        "StartPlugin"
    }

    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
        if rocket.payload.is_empty() {
            rocket.merge_params_to_payload();
        }

        next.call(rocket).await
    }
}
