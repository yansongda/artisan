//! 错误类型定义
//!
//! 定义框架中所有可能出现的错误类型，包括：
//! - HTTP 请求错误（RequestFailed）
//! - HTTP 客户端/请求构建错误（ClientBuild, `RequestBuildError`）
//! - 序列化错误（JsonSerializeError, `JsonDeserializeError`)
//! - 插件错误（PluginExecutionError）
//! - 参数错误（MissingParameter, `InvalidParameter`)
//! - 响应解析错误（DirectionParseError）

use thiserror::Error;

/// 框架错误类型
#[derive(Debug, Error)]
pub enum ArtfulError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    /// HTTP 客户端构建失败
    ///
    /// 注意：`reqwest::Error` 的 `#[from]` 已被 [`ArtfulError::RequestFailed`]
    /// 占用，此处须使用显式 `#[source]`。
    #[error("failed to build HTTP client: {source}")]
    ClientBuild {
        #[source]
        source: reqwest::Error,
    },

    #[error("failed to build HTTP request: {source}")]
    RequestBuildError {
        #[source]
        source: reqwest::Error,
    },

    #[error("failed to serialize JSON: {0}")]
    JsonSerializeError(#[from] serde_json::Error),

    #[error("failed to deserialize JSON: {message}")]
    JsonDeserializeError {
        message: String,
        #[source]
        source: Option<serde_json::Error>,
    },

    #[error("plugin execution failed: {plugin_name} - {message}")]
    PluginExecutionError {
        plugin_name: String,
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("missing required parameter: {0}")]
    MissingParameter(String),

    #[error("invalid parameter: {param} - {message}")]
    InvalidParameter { param: String, message: String },

    #[error("failed to parse response: {0}")]
    DirectionParseError(String),

    #[error("missing HTTP request")]
    MissingRequest,

    #[error("missing HTTP response")]
    MissingResponse,

    #[error("{0}")]
    Other(String),
}

/// 框架 Result 类型别名
pub type Result<T> = std::result::Result<T, ArtfulError>;
