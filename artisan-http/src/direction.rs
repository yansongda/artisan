//! 响应解析模块
//!
//! 定义响应解析的抽象接口和解析策略。
//!
//! # 核心类型
//!
//! - [`Direction`] trait - 响应解析器接口
//! - [`DirectionKind`] - 解析策略枚举
//! - [`Destination`] - 解析结果类型
//!
//! # 解析策略
//!
//! - `Json` - 解析为 JSON（默认）
//! - `Response` - 返回原始 Response
//! - `NoRequest` - 不发起 HTTP 请求
//! - `Custom` - 自定义解析器

use std::sync::Arc;

use crate::error::ArtfulError;

/// 响应解析器 trait
#[async_trait::async_trait]
pub trait Direction: Send + Sync + std::fmt::Debug {
    /// 解析 HTTP 响应
    ///
    /// # Errors
    ///
    /// 返回错误当响应解析失败。
    async fn parse(&self, rocket: &mut crate::Rocket) -> crate::Result<Destination>;
}

/// 解析策略枚举
#[derive(Debug, Clone)]
pub enum DirectionKind {
    /// 解析为 JSON（默认）
    Json,
    /// 返回原始 Response
    Response,
    /// 不发起 HTTP 请求
    NoRequest,
    /// 自定义解析器
    Custom(Arc<dyn Direction>),
}

/// 解析结果类型
#[derive(Default)]
pub enum Destination {
    /// JSON 解析结果
    Json(serde_json::Value),
    /// 原始 HTTP Response
    Response(reqwest::Response),
    /// 无结果
    #[default]
    None,
}

impl std::fmt::Debug for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Destination::Json(v) => f.debug_tuple("Json").field(v).finish(),
            Destination::Response(_) => f
                .debug_tuple("Response")
                .field(&"<reqwest::Response>")
                .finish(),
            Destination::None => write!(f, "None"),
        }
    }
}

impl std::fmt::Display for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Destination::Json(v) => write!(f, "{v}"),
            Destination::Response(_) => write!(f, "<HTTP Response>"),
            Destination::None => write!(f, "None"),
        }
    }
}

impl From<serde_json::Value> for Destination {
    fn from(value: serde_json::Value) -> Self {
        Destination::Json(value)
    }
}

impl Destination {
    /// 取出 JSON 解析结果
    ///
    /// # Errors
    ///
    /// 返回 [`ArtfulError::DestinationMismatch`] 当目标不是 JSON 解析结果
    /// （`actual` 为变体名 `Response` / `None`）。
    pub fn into_json(self) -> crate::Result<serde_json::Value> {
        match self {
            Destination::Json(v) => Ok(v),
            Destination::Response(_) => Err(ArtfulError::DestinationMismatch {
                expected: "Json",
                actual: "Response".to_string(),
            }),
            Destination::None => Err(ArtfulError::DestinationMismatch {
                expected: "Json",
                actual: "None".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_direction_kind_variants() {
        assert!(matches!(DirectionKind::Json, DirectionKind::Json));
        assert!(matches!(DirectionKind::Response, DirectionKind::Response));
        assert!(matches!(DirectionKind::NoRequest, DirectionKind::NoRequest));
    }

    #[test]
    fn test_destination_default() {
        let dest = Destination::default();
        assert!(matches!(dest, Destination::None));
    }

    #[test]
    fn test_destination_from_json() {
        let value = json!({"key": "value"});
        let dest: Destination = value.into();
        assert!(matches!(dest, Destination::Json(_)));
    }

    #[test]
    fn test_destination_into_json_ok() {
        // Json → Ok(value)
        let dest = Destination::Json(json!({"key": "value"}));
        assert_eq!(dest.into_json().unwrap(), json!({"key": "value"}));
    }

    #[test]
    fn test_destination_into_json_response_mismatch() {
        // Response → DestinationMismatch，actual 为变体名（非 Display 文案）
        let dest = Destination::Response(sample_response());
        let err = dest.into_json().unwrap_err();
        match err {
            ArtfulError::DestinationMismatch { expected, actual } => {
                assert_eq!(expected, "Json");
                assert_eq!(actual, "Response");
            }
            other => panic!("expected DestinationMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_destination_into_json_none_mismatch() {
        // None → DestinationMismatch，actual 为变体名
        let dest = Destination::None;
        let err = dest.into_json().unwrap_err();
        match err {
            ArtfulError::DestinationMismatch { expected, actual } => {
                assert_eq!(expected, "Json");
                assert_eq!(actual, "None");
            }
            other => panic!("expected DestinationMismatch, got {other:?}"),
        }
    }

    fn sample_response() -> reqwest::Response {
        let inner = http::Response::builder()
            .status(200)
            .body(Vec::new())
            .unwrap();
        reqwest::Response::from(inner)
    }

    #[test]
    fn test_destination_debug() {
        let dest = Destination::Json(json!({"test": 1}));
        let debug_str = format!("{:?}", dest);
        assert!(debug_str.contains("Json"));

        let dest_none = Destination::None;
        assert_eq!(format!("{:?}", dest_none), "None");

        let dest_resp = Destination::Response(sample_response());
        assert!(format!("{:?}", dest_resp).contains("Response"));
    }

    #[test]
    fn test_destination_display() {
        let dest = Destination::Json(json!({"key": "value"}));
        let display_str = format!("{}", dest);
        assert!(display_str.contains("key"));

        let dest_none = Destination::None;
        assert_eq!(format!("{}", dest_none), "None");

        let dest_resp = Destination::Response(sample_response());
        assert_eq!(format!("{}", dest_resp), "<HTTP Response>");
    }

    #[test]
    fn test_direction_kind_custom() {
        #[derive(Debug)]
        struct NullDirection;

        #[async_trait::async_trait]
        impl Direction for NullDirection {
            async fn parse(&self, _rocket: &mut crate::Rocket) -> crate::Result<Destination> {
                Err(crate::error::ArtfulError::MissingResponse)
            }
        }

        let kind = DirectionKind::Custom(Arc::new(NullDirection));
        assert!(matches!(kind, DirectionKind::Custom(_)));
    }
}
