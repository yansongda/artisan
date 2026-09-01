//! 不解析方向
//!
//! 对齐 artful PHP 的 `NoHttpRequestDirection`：不做任何解析，原样透传
//! `rocket.destination` 的现有值（无值时为 [`Destination::None`]），与 PHP 版本
//! "直接返回原响应、null 透传 null"的语义一致。
//!
//! 说明：因 [`Destination::Response`]（内含 reqwest::Response 流式 body）
//! 不可克隆，现有值经 `take` 取走；在 `ignite` 流程中取走的结果随即被
//! 写回 `rocket.destination`，行为与透传一致；原始响应
//! （`destination_origin`）始终不被消费。

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction};

/// 不解析方向：以 `rocket.destination` 现有值作为解析结果
#[derive(Debug, Clone)]
pub struct NoHttpRequestDirection;

#[async_trait]
impl Direction for NoHttpRequestDirection {
    /// 以 `rocket.destination` 的现有值作为解析结果
    ///
    /// # Errors
    ///
    /// 不返回错误。
    async fn parse(&self, rocket: &mut Rocket) -> crate::Result<Destination> {
        Ok(rocket.destination.take().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[tokio::test]
    async fn passes_through_existing_destination() {
        // destination 已有值：原样透传，不消费
        let mut rocket = Rocket::new(HashMap::new());
        rocket.destination = Some(Destination::Json(json!({"a": 1})));

        let result = NoHttpRequestDirection.parse(&mut rocket).await.unwrap();

        match result {
            Destination::Json(value) => assert_eq!(value, json!({"a": 1})),
            other => panic!("Expected JSON destination, got {:?}", other),
        }
        // 现有 destination 被 take 取走（ignite 流程中随即写回解析结果）
        assert!(rocket.destination.is_none());
    }

    #[tokio::test]
    async fn returns_none_when_destination_absent() {
        // destination 为 None：返回 Destination::None（对齐 PHP null 透传 null）
        let mut rocket = Rocket::new(HashMap::new());

        let result = NoHttpRequestDirection.parse(&mut rocket).await.unwrap();

        assert!(matches!(result, Destination::None));
    }
}
