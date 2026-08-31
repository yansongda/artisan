//! 错误类型定义
//!
//! 定义框架中所有可能出现的错误类型，包括：
//! - HTTP 请求错误（RequestFailed）
//! - HTTP 客户端/请求构建错误（ClientBuildError, `RequestBuildError`）
//! - 序列化错误（JsonSerializeError, `JsonDeserializeError`)
//! - 插件错误（PluginExecutionError）
//! - 事件监听器错误（EventListenerError）
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

    /// 事件监听器执行失败
    ///
    /// 监听器返回 `Err` 时由 [`crate::event::EventDispatcher::dispatch`] 包装产生，
    /// 首错即中止后续监听器并向主流程传播。
    ///
    /// `original` 仅在分发 [`crate::event::Event::HttpError`] 时监听器失败的场景
    /// 填充：被监听器错误顶替的原始执行错误（如
    /// [`ArtfulError::RequestFailed`]），供下游诊断或分支处理，避免错误链丢失；
    /// 其余分发点恒为 `None`。
    #[error("event listener failed: {listener_name} - {message}")]
    EventListenerError {
        listener_name: String,
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        /// 被监听器错误顶替的原始错误（仅 HttpError 分发场景，见变体文档）
        original: Option<Box<ArtfulError>>,
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
    fn event_listener_error_display() {
        let err = ArtfulError::EventListenerError {
            listener_name: "MyListener".to_string(),
            message: "boom".to_string(),
            source: None,
            original: None,
        };
        assert_eq!(err.to_string(), "event listener failed: MyListener - boom");
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn event_listener_error_with_source() {
        let source: Box<dyn std::error::Error + Send + Sync> = "inner cause".into();
        let err = ArtfulError::EventListenerError {
            listener_name: "MyListener".to_string(),
            message: "boom".to_string(),
            source: Some(source),
            original: None,
        };
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn event_listener_error_original_preserved() {
        // HttpError 分发场景：original 保留被顶替的原始错误（RequestFailed），
        // display 不含 original、source 仍指向监听器错误，两者独立可取
        let source: Box<dyn std::error::Error + Send + Sync> = "listener inner".into();
        let err = ArtfulError::EventListenerError {
            listener_name: "MetricsListener".to_string(),
            message: "metrics sink unreachable".to_string(),
            source: Some(source),
            original: Some(Box::new(ArtfulError::RequestFailed(sample_reqwest_error()))),
        };

        assert_eq!(
            err.to_string(),
            "event listener failed: MetricsListener - metrics sink unreachable"
        );

        match err {
            ArtfulError::EventListenerError {
                original: Some(original),
                source,
                ..
            } => {
                assert!(matches!(*original, ArtfulError::RequestFailed(_)));
                assert!(source.is_some());
            }
            other => panic!("expected EventListenerError, got {other:?}"),
        }
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
