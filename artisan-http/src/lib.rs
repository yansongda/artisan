//! artisan-http - 基于洋葱模型的 Rust HTTP 客户端框架
//!
//! 灵感来自 [yansongda/artful](https://github.com/yansongda/artful)，
//! 请求层层穿透、响应层层返回，插件化组合每个请求。
//!
//! # 核心概念
//!
//! - **洋葱模型**: 请求前向穿透插件链，响应后向逐层返回
//! - **插件化**: 每个请求由一组 [`Plugin`] 组合驱动，高度灵活
//! - **[`Rocket`]**: 请求生命周期中的数据载体，贯穿整个插件链
//! - **[`Direction`]**: 响应解析策略，支持 JSON / 原始 Response / 自定义
//! - **[`Shortcut`]**: 插件预设 trait，封装常用请求模式
//!
//! # 关键类型
//!
//! | 类型 | 职责 | 所在模块 |
//! |------|------|----------|
//! | [`Artful`] | 框架入口 | [`artful`] |
//! | [`Rocket`] | 请求/响应载体 | [`rocket`] |
//! | [`Plugin`] | 插件 trait | [`plugin`] |
//! | [`FlowCtrl`] / [`Next`] | 洋葱链控制 | [`flow_ctrl`] |
//! | [`Direction`] | 响应解析 trait | [`direction`] |
//! | [`Packer`] | 序列化 trait | [`packer`] |
//! | [`Shortcut`] | 插件预设 trait | [`shortcut`] |
//! | [`Event`] / [`EventListener`] / [`EventDispatcher`] | 生命周期事件与监听 | [`event`] |
//!
//! # 内置插件
//!
//! | 插件 | 功能 |
//! |------|------|
//! | [`StartPlugin`] | 将 params 初始化到 payload |
//! | [`AddPayloadBodyPlugin`] | 将 payload 序列化为请求体 |
//! | [`AddRadarPlugin`] | 构建 HTTP Request |
//! | [`ParserPlugin`] | 解析响应为 destination，必须挂在链尾 |
//!
//! # 使用示例
//!
//! ```rust
//! use artisan_http::{Artful, Plugin, Rocket, flow_ctrl::Next};
//! use artisan_http::plugins::{StartPlugin, AddPayloadBodyPlugin, AddRadarPlugin};
//! use async_trait::async_trait;
//! use std::sync::Arc;
//!
//! struct MethodUrlPlugin {
//!     method: reqwest::Method,
//!     url: String,
//! }
//!
//! #[async_trait]
//! impl Plugin for MethodUrlPlugin {
//!     async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
//!         rocket.config.method = self.method.clone();
//!         rocket.config.url = self.url.clone();
//!         next.call(rocket).await
//!     }
//! }
//! ```

use serde_json::{Map, Value};

pub mod direction;
pub mod directions;
pub mod error;
pub use directions::{JsonDirection, NoHttpRequestDirection, OriginResponseDirection};
pub mod artful;
pub mod config;
pub mod event;
pub mod flow_ctrl;
mod http;
mod ignite;
pub mod packer;
pub mod packers;
pub mod plugin;
pub mod plugins;
pub mod rocket;
pub mod shortcut;

pub use artful::{Artful, ArtfulBuilder};
pub use config::Config;
pub use direction::{Destination, Direction, DirectionKind};
pub use error::{ArtfulError, Result};
pub use event::{Event, EventDispatcher, EventListener};
pub use flow_ctrl::{FlowCtrl, Next};
pub use packer::{Packer, pack_typed, unpack_typed};
pub use packers::{JsonPacker, QueryPacker, XmlPacker};
pub use plugin::Plugin;
pub use plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};
pub use rocket::{ClientOptions, RequestOptions, Rocket, RocketConfig};
pub use shortcut::Shortcut;

/// 过滤用于请求体序列化的 payload：剔除 `_` 前缀键与 `null` 值
///
/// `_` 前缀键是控制参数（如 [`QueryPacker`] 的 `_unpack_raw`），只应影响
/// 本侧行为，不得进入发往网关的请求体——部分网关对全部报文字段验签，
/// 多出的字段会导致验签失败。`null` 值无业务含义，同样剔除。
///
/// [`AddPayloadBodyPlugin`](plugins::AddPayloadBodyPlugin) 打包请求体时
/// 调用；响应解包侧不做此过滤。
pub fn filter_params(payload: &Map<String, Value>) -> Map<String, Value> {
    payload
        .iter()
        .filter(|(k, v)| !k.starts_with('_') && !v.is_null())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>() {}

    #[test]
    fn core_types_are_send_sync() {
        // 框架契约：核心类型可跨 tokio task 共享
        assert_send_sync::<Artful>();
        // builder 为一次性消费对象：只断言 Send（装箱的 FnOnce 回调非 Sync）
        assert_send::<ArtfulBuilder>();
        assert_send_sync::<Rocket>();
        assert_send_sync::<FlowCtrl>();
        assert_send_sync::<Config>();
        assert_send_sync::<RocketConfig>();
        assert_send_sync::<ClientOptions>();
        assert_send_sync::<JsonPacker>();
        assert_send_sync::<QueryPacker>();
        assert_send_sync::<XmlPacker>();
        assert_send_sync::<JsonDirection>();
        assert_send_sync::<NoHttpRequestDirection>();
        assert_send_sync::<OriginResponseDirection>();
        assert_send_sync::<EventDispatcher>();
        assert_send_sync::<ParserPlugin>();
    }

    #[test]
    fn artful_default_construction_succeeds() {
        let artful = Artful::new().unwrap();
        assert!(artful.config().extra.is_empty());
        let _client: &reqwest::Client = artful.client();
    }
}
