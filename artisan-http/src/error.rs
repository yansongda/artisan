//! 错误类型定义
//!
//! 定义框架中所有可能出现的错误类型，包括：
//! - HTTP 请求错误（RequestFailed）
//! - HTTP 客户端/请求构建错误（ClientBuildError, `RequestBuildError`）
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
    ClientBuildError {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 同步构造真实 reqwest::Error（无效 URL 触发请求构建失败，无网络）
    fn sample_reqwest_error() -> reqwest::Error {
        reqwest::Client::new()
            .get("not a valid url")
            .build()
            .unwrap_err()
    }

    fn sample_serde_error() -> serde_json::Error {
        serde_json::from_str::<serde_json::Value>("not json").unwrap_err()
    }

    #[test]
    fn request_failed_display_and_from() {
        let err: ArtfulError = sample_reqwest_error().into();
        assert!(matches!(err, ArtfulError::RequestFailed(_)));
        assert!(err.to_string().starts_with("HTTP request failed:"));
    }

    #[test]
    fn client_build_error_display_and_source() {
        let err = ArtfulError::ClientBuildError {
            source: sample_reqwest_error(),
        };
        assert!(err.to_string().starts_with("failed to build HTTP client:"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn request_build_error_display_and_source() {
        let err = ArtfulError::RequestBuildError {
            source: sample_reqwest_error(),
        };
        assert!(err.to_string().starts_with("failed to build HTTP request:"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn json_serialize_error_from_serde() {
        let err: ArtfulError = sample_serde_error().into();
        assert!(matches!(err, ArtfulError::JsonSerializeError(_)));
        assert!(err.to_string().starts_with("failed to serialize JSON:"));
    }

    #[test]
    fn json_deserialize_error_display_and_source() {
        let err = ArtfulError::JsonDeserializeError {
            message: "unexpected token".to_string(),
            source: Some(sample_serde_error()),
        };
        assert!(err.to_string().starts_with("failed to deserialize JSON:"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn plugin_execution_error_display() {
        let err = ArtfulError::PluginExecutionError {
            plugin_name: "MyPlugin".to_string(),
            message: "boom".to_string(),
            source: None,
        };
        assert_eq!(err.to_string(), "plugin execution failed: MyPlugin - boom");
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn plugin_execution_error_with_source() {
        let source: Box<dyn std::error::Error + Send + Sync> = "inner cause".into();
        let err = ArtfulError::PluginExecutionError {
            plugin_name: "MyPlugin".to_string(),
            message: "boom".to_string(),
            source: Some(source),
        };
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn simple_variants_display() {
        assert_eq!(
            ArtfulError::MissingParameter("token".to_string()).to_string(),
            "missing required parameter: token"
        );
        assert_eq!(
            ArtfulError::InvalidParameter {
                param: "amount".to_string(),
                message: "must be positive".to_string(),
            }
            .to_string(),
            "invalid parameter: amount - must be positive"
        );
        assert_eq!(
            ArtfulError::DirectionParseError("bad body".to_string()).to_string(),
            "failed to parse response: bad body"
        );
        assert_eq!(
            ArtfulError::MissingRequest.to_string(),
            "missing HTTP request"
        );
        assert_eq!(
            ArtfulError::MissingResponse.to_string(),
            "missing HTTP response"
        );
        assert_eq!(
            ArtfulError::Other("custom failure".to_string()).to_string(),
            "custom failure"
        );
    }
}
