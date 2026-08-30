//! Artisan 主入口模块
//!
//! 框架的核心入口，提供三种请求方式：
//!
//! # 方法
//!
//! - [`Artful::new`] - 以默认配置创建实例
//! - [`Artful::with_config`] - 以指定配置创建实例（构造时构建 HTTP 客户端，fail-fast）
//! - [`Artful::with_builder`] - 以指定配置与自定义构建流程创建实例（config.http 生效 + 回调叠加）
//! - [`Artful::with_client`] - 以指定配置与外部构建的 HTTP 客户端创建实例
//! - [`Artful::artful`] - 执行完整插件链
//! - [`Artful::shortcut`] - 使用 Shortcut 快捷方式
//! - [`Artful::raw`] - 直接 HTTP 请求（跳过插件）

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::Result;
use crate::config::Config;
use crate::direction::Destination;
use crate::error::ArtfulError;
use crate::flow_ctrl::FlowCtrl;
use crate::http::build_builder;
use crate::plugin::Plugin;
use crate::rocket::Rocket;
use crate::shortcut::Shortcut;

/// Artful 主类 - 框架入口
///
/// 实例类型：配置与 HTTP 客户端在构造时显式解析（fail-fast），
/// 配置错误在构造期即暴露，支持多实例共存与测试隔离。
/// 每个实例持有独立的连接池（多渠道场景池相互隔离）；
/// `reqwest::Client` 内部为 `Arc`，[`Clone`] 廉价且共享连接池，
/// 单实例场景可配合 `std::sync::LazyLock` 构建全局单例（参见 README）。
#[derive(Debug, Clone)]
pub struct Artful {
    config: Config,
    client: reqwest::Client,
}

impl Artful {
    /// 以默认配置创建实例
    ///
    /// # Errors
    ///
    /// 返回错误当 HTTP 客户端构建失败。
    pub fn new() -> Result<Self> {
        Self::with_config(Config::default())
    }

    /// 以指定配置创建实例
    ///
    /// 构造时即按 `config.http` 构建 HTTP 客户端，配置错误立即暴露。
    ///
    /// # Errors
    ///
    /// 返回 [`ArtfulError::ClientBuildError`] 当 HTTP 客户端构建失败。
    pub fn with_config(config: Config) -> Result<Self> {
        Self::with_builder(config, |builder| builder)
    }

    /// 以指定配置与自定义构建流程创建实例
    ///
    /// 构建顺序：先由框架按 `config.http` 应用默认值（pool/UA/timeout/connect_timeout，
    /// 未设置项使用框架默认），再交由 `customize` 回调叠加 `ClientOptions` 无法表达的
    /// 能力——代理、TLS 客户端证书、cookie 会话、重定向策略等——最后构建。
    /// 回调内后写的 setter 覆盖先写的值（reqwest 覆盖语义），如可覆盖框架默认 UA。
    ///
    /// # Errors
    ///
    /// 返回 [`ArtfulError::ClientBuildError`] 当 HTTP 客户端构建失败
    /// （含回调叠加后仍不合法的情况，如非法 user_agent）。
    pub fn with_builder(
        config: Config,
        customize: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
    ) -> Result<Self> {
        let client = customize(build_builder(config.http.clone()))
            .build()
            .map_err(|source| ArtfulError::ClientBuildError { source })?;

        Ok(Self { config, client })
    }

    /// 以指定配置与外部构建的 HTTP 客户端创建实例
    ///
    /// 适用于 [`crate::ClientOptions`] 无法表达的 client 级能力——代理、TLS 证书、
    /// cookie 会话、重定向策略等——由调用方自行构建 [`reqwest::Client`] 后注入。
    ///
    /// 注意：传入的 client 原样生效，`config.http` 中的选项**不会**作用于它，
    /// 仅作为配置记录（可经 [`Artful::config`] 读取）。
    pub fn with_client(config: Config, client: reqwest::Client) -> Self {
        Self { config, client }
    }

    /// 获取实例配置
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// 获取实例 HTTP 客户端
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 执行插件链
    ///
    /// # 参数
    ///
    /// - `params`: 原始参数（存储在 rocket.params，不可变）
    /// - `plugins`: 插件列表（负责设置 method、url 等配置）
    ///
    /// # Errors
    ///
    /// 返回错误当：
    /// - 插件执行失败
    /// - HTTP 请求失败
    /// - 响应解析失败
    pub async fn artful(
        &self,
        params: HashMap<String, Value>,
        plugins: Vec<Arc<dyn Plugin>>,
    ) -> Result<Destination> {
        let mut rocket = Rocket::new(params);
        rocket.client = self.client.clone();

        let mut ctrl = FlowCtrl::new(plugins);

        ctrl.call_next(&mut rocket).await?;

        Ok(rocket.destination.unwrap_or_default())
    }

    /// 使用 Shortcut 快捷方式
    ///
    /// # 参数
    ///
    /// - `shortcut`: Shortcut 实例
    /// - `params`: 原始参数
    ///
    /// # Errors
    ///
    /// 返回错误当：
    /// - 插件执行失败
    /// - HTTP 请求失败
    /// - 响应解析失败
    pub async fn shortcut<S: Shortcut>(
        &self,
        shortcut: S,
        params: HashMap<String, Value>,
    ) -> Result<Destination> {
        let plugins = shortcut.get_plugins(&params);
        self.artful(params, plugins).await
    }

    /// 直接调用 HTTP（跳过插件链）
    ///
    /// # Errors
    ///
    /// 返回错误当 HTTP 请求失败。
    pub async fn raw(&self, request: reqwest::Request) -> Result<reqwest::Response> {
        self.client
            .execute(request)
            .await
            .map_err(ArtfulError::RequestFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocket::ClientOptions;

    #[test]
    fn test_artful_config_default() {
        // new() 使用默认配置，config() 返回构造时配置
        let artful = Artful::new().unwrap();
        assert!(artful.config().extra.is_empty());
        assert!(artful.config().http.timeout.is_none());
    }

    #[test]
    fn test_artful_config_roundtrip() {
        // with_config 保存的配置可经 config() 完整读回
        let config = Config {
            http: ClientOptions {
                timeout: Some(30),
                ..Default::default()
            },
            ..Default::default()
        };
        let artful = Artful::with_config(config).unwrap();
        assert_eq!(artful.config().http.timeout, Some(30));
    }

    #[test]
    fn test_artful_new_and_accessors() {
        // 成功路径：with_config 保存配置并构建 client
        let config = Config {
            http: ClientOptions {
                timeout: Some(10),
                ..Default::default()
            },
            ..Default::default()
        };
        let artful = Artful::with_config(config).unwrap();
        assert_eq!(artful.config().http.timeout, Some(10));
        let _client: &reqwest::Client = artful.client();

        // 默认构造路径
        let artful = Artful::new().unwrap();
        assert!(artful.config().extra.is_empty());

        // 失败路径：非法 user_agent 导致 client 构建失败 → ClientBuildError
        let bad = Config {
            http: ClientOptions {
                user_agent: Some("bad\nua".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = Artful::with_config(bad).unwrap_err();
        assert!(matches!(err, ArtfulError::ClientBuildError { .. }));
    }

    #[test]
    fn with_builder_build_error_propagates() {
        // 回调叠加后仍不合法 → ClientBuildError
        let config = Config {
            http: ClientOptions {
                user_agent: Some("bad\nua".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = Artful::with_builder(config, |builder| builder).unwrap_err();

        assert!(matches!(err, ArtfulError::ClientBuildError { .. }));
    }
}
