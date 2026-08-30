//! JSON 解析方向
//!
//! 将响应解析为 JSON 格式。

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction};
use crate::error::ArtfulError;

/// JSON 解析方向
#[derive(Debug, Clone)]
pub struct JsonDirection;

#[async_trait]
impl Direction for JsonDirection {
    /// 将 HTTP 响应解析为 JSON
    ///
    /// # Errors
    ///
    /// 返回错误当：
    /// - 响应体读取失败（[`ArtfulError::RequestFailed`]）
    /// - JSON 反序列化失败（[`ArtfulError::JsonDeserializeError`]）
    /// - 无响应对象（[`ArtfulError::MissingResponse`]）
    async fn parse(&self, rocket: &mut Rocket) -> crate::Result<Destination> {
        match rocket.destination_origin.take() {
            Some(response) => {
                let text = response.text().await.map_err(ArtfulError::RequestFailed)?;
                serde_json::from_str(&text)
                    .map(Destination::Json)
                    .map_err(|e| ArtfulError::JsonDeserializeError {
                        message: e.to_string(),
                        source: Some(e),
                    })
            }
            None => Err(ArtfulError::MissingResponse),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 构造携带指定响应体的 Rocket(经 http::Response 转换，无需网络)
    fn rocket_with_response(body: &'static str) -> Rocket {
        let mut rocket = Rocket::new(HashMap::new());
        let inner = http::Response::builder()
            .status(200)
            .body(body.as_bytes().to_vec())
            .unwrap();
        rocket.destination_origin = Some(reqwest::Response::from(inner));
        rocket
    }

    #[tokio::test]
    async fn parses_valid_json() {
        let mut rocket = rocket_with_response(r#"{"ok":true,"code":0}"#);

        let result = JsonDirection.parse(&mut rocket).await.unwrap();

        match result {
            Destination::Json(value) => {
                assert_eq!(value["ok"], true);
                assert_eq!(value["code"], 0);
            }
            other => panic!("Expected JSON destination, got {:?}", other),
        }
        // 解析后原始响应应被消费
        assert!(rocket.destination_origin.is_none());
    }

    #[tokio::test]
    async fn errors_on_invalid_json() {
        let mut rocket = rocket_with_response("this is not json");

        let result = JsonDirection.parse(&mut rocket).await;

        assert!(matches!(
            result.unwrap_err(),
            ArtfulError::JsonDeserializeError { .. }
        ));
    }

    #[tokio::test]
    async fn missing_response_when_origin_absent() {
        // destination_origin 默认为 None → MissingResponse
        let mut rocket = Rocket::new(HashMap::new());

        let result = JsonDirection.parse(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingResponse));
    }
}
