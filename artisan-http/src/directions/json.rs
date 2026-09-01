//! JSON 解析方向
//!
//! 将响应解析为 JSON 格式。

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction};
use crate::error::ArtfulError;

/// JSON 解析方向
///
/// 实际行为是"用 [`Rocket::packer`] 解包响应体"（对齐 PHP `CollectionDirection::guide()`）。
/// 默认 packer 为 [`JsonPacker`](crate::packers::JsonPacker) 时即 JSON 解析；
/// 设置 [`XmlPacker`](crate::packers::XmlPacker) 后响应按 XML 解包。
#[derive(Debug, Clone)]
pub struct JsonDirection;

#[async_trait]
impl Direction for JsonDirection {
    /// 将 HTTP 响应解析为 JSON
    ///
    /// 读取响应体文本后交由 `rocket.packer` 解包（params 传 `rocket.payload` 全量，
    /// 不过滤 `_` 特殊参数——对齐 PHP `$payload?->all()`），结果包装为
    /// [`Destination::Json`]。
    ///
    /// # Errors
    ///
    /// 返回错误当：
    /// - 响应体读取失败（[`ArtfulError::RequestFailed`]）
    /// - packer 解包失败（[`ArtfulError::JsonDeserializeError`] 或自定义 Packer 的错误）
    /// - 无响应对象（[`ArtfulError::MissingResponse`]）
    async fn parse(&self, rocket: &mut Rocket) -> crate::Result<Destination> {
        match rocket.destination_origin.take() {
            Some(response) => {
                let text = response.text().await.map_err(ArtfulError::RequestFailed)?;
                rocket
                    .packer
                    .unpack(&text, &rocket.payload)
                    .map(Destination::Json)
            }
            None => Err(ArtfulError::MissingResponse),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use crate::packers::XmlPacker;

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
    async fn parses_xml_body_when_packer_replaced() {
        // packer 可替换语义：设置 XmlPacker 后，XML 响应体按 XML 解包
        let mut rocket = rocket_with_response("<root><ok>true</ok><code>0</code></root>");
        rocket.packer = Arc::new(XmlPacker);

        let result = JsonDirection.parse(&mut rocket).await.unwrap();

        match result {
            Destination::Json(value) => {
                // XmlPacker 输出：叶子文本为 Value::String，根元素值为结果（不含根名）
                assert!(value.is_object());
                assert_eq!(value["ok"], "true");
                assert_eq!(value["code"], "0");
            }
            other => panic!("Expected JSON destination, got {:?}", other),
        }
        assert!(rocket.destination_origin.is_none());
    }

    #[tokio::test]
    async fn missing_response_when_origin_absent() {
        // destination_origin 默认为 None → MissingResponse
        let mut rocket = Rocket::new(HashMap::new());

        let result = JsonDirection.parse(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingResponse));
    }
}
