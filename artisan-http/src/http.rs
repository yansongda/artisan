//! HTTP 客户端构建模块
//!
//! 提供 HTTP 客户端构建与框架默认客户端，基于 reqwest 实现。
//!
//! # 设计说明
//!
//! - 客户端在 `Artful` 构造时按 [`ClientOptions`] 显式构建（fail-fast）
//! - 连接池参数、超时、User-Agent 均从 [`ClientOptions`] 消费
//! - Per-request timeout 通过 [`RocketConfig::http`](crate::rocket::RocketConfig::http) 设置
//! - 默认客户端构建失败时使用 fallback

use std::sync::OnceLock;
use std::time::Duration;

use crate::rocket::ClientOptions;

const DEFAULT_POOL_IDLE_TIMEOUT: u64 = 90;
const DEFAULT_POOL_MAX_IDLE_PER_HOST: usize = 20;
const DEFAULT_USER_AGENT: &str = concat!("yansongda/artisan-http:", env!("CARGO_PKG_VERSION"));

/// 获取框架默认 HTTP 客户端
///
/// 按 [`ClientOptions::default()`] 构建客户端（惰性初始化，仅构建一次），
/// 构建失败时使用 fallback 默认客户端。
/// 供 [`Rocket::new`](crate::rocket::Rocket::new) 初始化默认 client。
pub(crate) fn default_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    CLIENT.get_or_init(|| {
        build_client(ClientOptions::default()).unwrap_or_else(|_| fallback_client())
    })
}

/// 按 [`ClientOptions`] 配置 `reqwest::ClientBuilder`（消费全部字段，不构建）
///
/// 框架默认值兜底：pool_idle_timeout=90s、pool_max_idle_per_host=20、
/// UA=`yansongda/artisan-http:{version}`；未设置的 timeout/connect_timeout
/// 保持 reqwest 默认（无超时）。
pub(crate) fn build_builder(options: ClientOptions) -> reqwest::ClientBuilder {
    let pool_idle_timeout = options
        .pool_idle_timeout
        .unwrap_or(DEFAULT_POOL_IDLE_TIMEOUT);
    let pool_max_idle_per_host = options
        .pool_max_idle_per_host
        .unwrap_or(DEFAULT_POOL_MAX_IDLE_PER_HOST);
    let user_agent = options
        .user_agent
        .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string());

    let mut builder = reqwest::Client::builder()
        .pool_idle_timeout(Some(Duration::from_secs(pool_idle_timeout)))
        .pool_max_idle_per_host(pool_max_idle_per_host)
        .user_agent(user_agent);

    if let Some(secs) = options.timeout {
        builder = builder.timeout(Duration::from_secs(secs));
    }

    if let Some(secs) = options.connect_timeout {
        builder = builder.connect_timeout(Duration::from_secs(secs));
    }

    builder
}

/// 按 [`ClientOptions`] 构建 HTTP 客户端（消费全部字段）
pub(crate) fn build_client(options: ClientOptions) -> Result<reqwest::Client, reqwest::Error> {
    build_builder(options).build()
}

fn fallback_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(DEFAULT_USER_AGENT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
