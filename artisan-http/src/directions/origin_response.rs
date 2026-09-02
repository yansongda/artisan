//! 原始响应方向
//!
//! 对齐 artful PHP 的 `OriginResponseDirection`：不做任何解析，直接将原始
//! HTTP 响应以 [`Destination::Response`] 返回；无原始响应时返回
//! [`ArtfulError::MissingResponse`]（对齐 PHP 抛出 `InvalidResponseException`
//! 9303 的行为）。

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction};
use crate::error::ArtfulError;

/// 原始响应方向：返回未经解析的原始 HTTP 响应
#[derive(Debug, Clone)]
pub struct OriginResponseDirection;

#[async_trait]
impl Direction for OriginResponseDirection {
    /// 返回原始 HTTP 响应
    ///
    /// # Errors
    ///
    /// 返回错误当：
    /// - 无响应对象（[`ArtfulError::MissingResponse`]）
    async fn parse(&self, rocket: &mut Rocket) -> crate::Result<Destination> {
        rocket
            .destination_origin
            .take()
            .map(Destination::Response)
            .ok_or(ArtfulError::MissingResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 构造携带原始响应的 Rocket(经 http::Response 转换，无需网络)
    fn rocket_with_response() -> Rocket {
        let mut rocket = Rocket::new(HashMap::new());
        let inner = http::Response::builder()
            .status(200)
            .body(Vec::new())
            .unwrap();
        rocket.destination_origin = Some(reqwest::Response::from(inner));
        rocket
    }

    #[tokio::test]
    async fn returns_response_and_takes_origin() {
        // origin 存在：返回 Destination::Response，且原始响应被 take 消费
        let mut rocket = rocket_with_response();

        let result = OriginResponseDirection.parse(&mut rocket).await.unwrap();

        assert!(matches!(result, Destination::Response(_)));
        assert!(rocket.destination_origin.is_none());
    }

    #[tokio::test]
    async fn missing_response_when_origin_absent() {
        // origin 缺失：返回 MissingResponse（对齐 PHP 抛 9303）
        let mut rocket = Rocket::new(HashMap::new());

        let result = OriginResponseDirection.parse(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingResponse));
    }
}
