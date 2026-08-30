//! Artisan 主入口模块
//!
//! 框架的核心入口，提供三种请求方式：
//!
//! # 方法
//!
//! - [`Artful::new`] - 以默认配置创建实例
//! - [`Artful::with_config`] - 以指定配置创建实例（构造时构建 HTTP 客户端，fail-fast）
//! - [`Artful::with_client_builder`] - 以指定配置与自定义构建流程创建实例（config.http 生效 + 回调叠加）
//! - [`Artful::with_client`] - 以指定配置与外部构建的 HTTP 客户端创建实例
//! - [`Artful::builder`] - 链式构建器入口（config / customize / client 可选叠加，build 时按优先级构建）
//! - [`Artful::artful`] - 执行完整插件链
//! - [`Artful::shortcut`] - 使用 Shortcut 快捷方式
//! - [`Artful::raw`] - 直接 HTTP 请求（跳过插件）

use std::collections::HashMap;
use std::fmt;
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
        Self::with_client_builder(config, |builder| builder)
    }

    /// 以指定配置与自定义构建流程创建实例
    ///
    /// 构建顺序：先由框架按 `config.http` 应用默认值（pool/UA/timeout/connect_timeout，
    /// 未设置项使用框架默认），再交由 `customize` 回调叠加 `ClientOptions` 无法表达的
    /// 能力——代理、TLS 客户端证书、cookie 会话、重定向策略等——最后构建。
    /// 回调内后写的 setter 覆盖先写的值（reqwest 覆盖语义），如可覆盖框架默认 UA。
    ///
    /// 亦可经 [`Artful::builder`] 链式构建。
    ///
    /// # Errors
    ///
    /// 返回 [`ArtfulError::ClientBuildError`] 当 HTTP 客户端构建失败
    /// （含回调叠加后仍不合法的情况，如非法 user_agent）。
    pub fn with_client_builder(
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
    ///
    /// 亦可经 [`Artful::builder`] 链式构建。
    pub fn with_client(config: Config, client: reqwest::Client) -> Self {
        Self { config, client }
    }

    /// 创建链式构建器（统一构建入口）
    ///
    /// 等价于 [`ArtfulBuilder::default()`]，可依次叠加 `config` / `customize` /
    /// `client`（后写覆盖先写），最终经 [`ArtfulBuilder::build`] 构建 [`Artful`] 实例。
    pub fn builder() -> ArtfulBuilder {
        ArtfulBuilder::default()
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

/// [`Artful`] 链式构建器
///
/// 统一构建入口（经 [`Artful::builder`] 创建，等价 [`ArtfulBuilder::default()`]），
/// `config` / `customize` / `client` 三项可选叠加，后写覆盖先写；
/// [`ArtfulBuilder::build`] 按优先级构建：已注入 `client` 时直接使用
/// （`config.http` 与 `customize` 均不参与构建），否则按 `config.http`
/// 应用框架默认值后叠加 `customize` 回调构建（fail-fast）。
pub struct ArtfulBuilder {
    config: Config,
    customize:
        Option<Box<dyn FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder + Send + 'static>>,
    client: Option<reqwest::Client>,
}

// 三字段默认值显式可读，按设计保留手写 impl（clippy 建议 derive，见 lint allow）
#[allow(clippy::derivable_impls)]
impl Default for ArtfulBuilder {
    fn default() -> Self {
        Self {
            config: Config::default(),
            customize: None,
            client: None,
        }
    }
}

impl fmt::Debug for ArtfulBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // customize / client 装箱内容不可 Debug，仅打印是否注入
        struct Presence<T>(Option<T>);

        impl<T> fmt::Debug for Presence<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self.0 {
                    Some(_) => f.write_str("Some(_)"),
                    None => f.write_str("None"),
                }
            }
        }

        f.debug_struct("ArtfulBuilder")
            .field("config", &self.config)
            .field("customize", &Presence(self.customize.as_ref()))
            .field("client", &Presence(self.client.as_ref()))
            .finish()
    }
}

impl ArtfulBuilder {
    /// 设置实例配置（覆盖式：后写覆盖先写，未设置则使用 [`Config::default()`]）
    ///
    /// `config.http` 仅在未注入 client 时参与构建。
    pub fn config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// 设置 HTTP 客户端自定义构建回调（覆盖式：后写覆盖先写）
    ///
    /// 语义与 [`Artful::with_client_builder`] 一致：回调在框架按 `config.http`
    /// 应用默认值后叠加，回调内后写的 setter 覆盖先写的值（reqwest 覆盖语义）。
    ///
    /// 注意：相较 [`Artful::with_client_builder`] 的参数，本方法多 `Send + 'static`
    /// 约束（内部装箱所需）；捕获非 `Send` / 非 `'static` 值的闭包请改用
    /// [`Artful::with_client_builder`]。
    pub fn customize<F>(mut self, f: F) -> Self
    where
        F: FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder + Send + 'static,
    {
        self.customize = Some(Box::new(f));
        self
    }

    /// 注入外部构建的 HTTP 客户端
    ///
    /// 传入的 client 原样生效，`config.http` 中的选项**不会**作用于它，
    /// 仅作为配置记录（可经 [`Artful::config`] 读取）。
    /// 该设置优先级最高：build 时忽略 `config.http` 与 `customize`，
    /// 同时设置 [`ArtfulBuilder::customize`] 时后者将被忽略。
    pub fn client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// 按优先级构建 [`Artful`] 实例
    ///
    /// 已注入 client 时直接使用（不构建、不校验）；
    /// 否则按 `config.http` 应用框架默认值后叠加 `customize` 回调构建（fail-fast）。
    ///
    /// # Errors
    ///
    /// 返回 [`ArtfulError::ClientBuildError`] 当 HTTP 客户端构建失败
    /// （含回调叠加后仍不合法的情况，如非法 user_agent）。
    pub fn build(self) -> Result<Artful> {
        if let Some(client) = self.client {
            return Ok(Artful {
                config: self.config,
                client,
            });
        }

        let customize = self.customize.unwrap_or_else(|| {
            Box::new(|builder: reqwest::ClientBuilder| builder)
                as Box<dyn FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder + Send>
        });
        Artful::with_client_builder(self.config, customize)
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
    fn with_client_builder_build_error_propagates() {
        // 回调叠加后仍不合法 → ClientBuildError
        let config = Config {
            http: ClientOptions {
                user_agent: Some("bad\nua".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = Artful::with_client_builder(config, |builder| builder).unwrap_err();

        assert!(matches!(err, ArtfulError::ClientBuildError { .. }));
    }

    #[test]
    fn builder_default_builds_like_new() {
        // 默认 builder 构建结果与 new() 一致：均使用默认配置
        let built = Artful::builder().build().unwrap();
        let artful = Artful::new().unwrap();

        assert!(built.config().extra.is_empty());
        assert!(artful.config().extra.is_empty());
    }

    #[test]
    fn builder_config_customize_build_error() {
        // config.http 非法（回调为空）→ build 返回 ClientBuildError
        let err = Artful::builder()
            .config(Config {
                http: ClientOptions {
                    user_agent: Some("bad\nua".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            })
            .customize(|builder| builder)
            .build()
            .unwrap_err();

        assert!(matches!(err, ArtfulError::ClientBuildError { .. }));
    }

    #[test]
    fn builder_customize_build_error() {
        // 回调自身产出非法 client → build 返回 ClientBuildError
        let err = Artful::builder()
            .customize(|builder| builder.user_agent("bad\nua"))
            .build()
            .unwrap_err();

        assert!(matches!(err, ArtfulError::ClientBuildError { .. }));
    }

    #[test]
    fn builder_debug_impl() {
        // 手写 Debug：输出非空且含类型名
        let debug = format!("{:?}", Artful::builder());

        assert!(!debug.is_empty());
        assert!(debug.contains("ArtfulBuilder"));
    }
}
