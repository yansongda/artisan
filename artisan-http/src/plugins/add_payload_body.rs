//! Payload Body 插件
//!
//! 将 payload 序列化为 HTTP 请求体。
//!
//! # 行为
//!
//! - 仅在 `rocket.config.body` 未设置时生效
//! - 使用 rocket.packer 序列化 payload
//! - 设置结果到 `rocket.config.body`
//! - 请求头缺失 `Content-Type` 时，按 packer 声明的 [`Packer::content_type`] 补填（不覆盖用户显式设置）

use async_trait::async_trait;
use std::collections::HashMap;

use crate::Rocket;
use crate::flow_ctrl::Next;
use crate::plugin::Plugin;

/// 添加 payload body 插件
#[derive(Clone, Copy, Debug, Default)]
pub struct AddPayloadBodyPlugin;

#[async_trait]
impl Plugin for AddPayloadBodyPlugin {
    fn name(&self) -> &'static str {
        "AddPayloadBodyPlugin"
    }

    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
        if rocket.config.body.is_none() && !rocket.payload.is_empty() {
            rocket.config.body = Some(rocket.packer.pack(&rocket.payload, &HashMap::new())?);

            // 判重按头名不区分大小写，用户以任意大小写键显式设置的值都不覆盖
            if let Some(ct) = rocket.packer.content_type() {
                if !rocket.has_header("Content-Type") {
                    rocket
                        .config
                        .headers
                        .insert("Content-Type".to_string(), ct.to_string());
                }
            }
        }

        next.call(rocket).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_ctrl::FlowCtrl;
    use crate::packer::Packer;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn drive(rocket: &mut Rocket) -> crate::Result<()> {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(AddPayloadBodyPlugin)];
        FlowCtrl::new(plugins).call_next(rocket).await
    }

    #[tokio::test]
    async fn packs_payload_and_sets_content_type() {
        let params = HashMap::from([("order_id".to_string(), json!("123"))]);
        let mut rocket = Rocket::new(params);
        rocket.merge_params_to_payload();

        drive(&mut rocket).await.unwrap();

        let body = rocket.config.body.expect("body should be packed");
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["order_id"], "123");
        assert_eq!(
            rocket.config.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[tokio::test]
    async fn skips_when_body_preset() {
        // config.body 已预设:不打包、不补 CT
        let mut rocket = Rocket::new(HashMap::new());
        rocket.payload.insert("order_id".to_string(), json!("123"));
        rocket.set_body("preset body");

        drive(&mut rocket).await.unwrap();

        assert_eq!(rocket.config.body, Some("preset body".to_string()));
        assert!(!rocket.config.headers.contains_key("Content-Type"));
    }

    #[tokio::test]
    async fn skips_when_payload_empty() {
        let mut rocket = Rocket::new(HashMap::new());

        drive(&mut rocket).await.unwrap();

        assert!(rocket.config.body.is_none());
        assert!(!rocket.config.headers.contains_key("Content-Type"));
    }

    #[tokio::test]
    async fn respects_explicit_content_type_case_insensitive() {
        // 用户以小写键显式设置 CT:不应被覆盖为 application/json
        let mut rocket = Rocket::new(HashMap::new());
        rocket.payload.insert("order_id".to_string(), json!("123"));
        rocket.add_header("content-type", "application/custom");

        drive(&mut rocket).await.unwrap();

        assert_eq!(
            rocket.config.headers.get("content-type"),
            Some(&"application/custom".to_string())
        );
        // 不得再新增一个 "Content-Type" 键(判重按头名不区分大小写)
        assert!(!rocket.config.headers.contains_key("Content-Type"));
    }

    #[tokio::test]
    async fn no_content_type_when_packer_declares_none() {
        #[derive(Debug)]
        struct NullContentTypePacker;

        impl Packer for NullContentTypePacker {
            fn pack(
                &self,
                data: &HashMap<String, Value>,
                _params: &HashMap<String, Value>,
            ) -> crate::Result<String> {
                Ok(format!("packed:{}", data.len()))
            }

            fn unpack(
                &self,
                _data: &str,
                _params: &HashMap<String, Value>,
            ) -> crate::Result<Value> {
                Ok(Value::Null)
            }
        }

        let mut rocket = Rocket::new(HashMap::new());
        rocket.payload.insert("order_id".to_string(), json!("123"));
        rocket.packer = Arc::new(NullContentTypePacker);

        drive(&mut rocket).await.unwrap();

        assert_eq!(rocket.config.body, Some("packed:1".to_string()));
        assert!(!rocket.config.headers.contains_key("Content-Type"));
    }

    #[tokio::test]
    async fn pack_error_propagates() {
        #[derive(Debug)]
        struct FailingPacker;

        impl Packer for FailingPacker {
            fn pack(
                &self,
                _data: &HashMap<String, Value>,
                _params: &HashMap<String, Value>,
            ) -> crate::Result<String> {
                Err(crate::error::ArtfulError::Other("pack failed".to_string()))
            }

            fn unpack(
                &self,
                _data: &str,
                _params: &HashMap<String, Value>,
            ) -> crate::Result<Value> {
                Ok(Value::Null)
            }
        }

        let mut rocket = Rocket::new(HashMap::new());
        rocket.payload.insert("order_id".to_string(), json!("123"));
        rocket.packer = Arc::new(FailingPacker);

        let result = drive(&mut rocket).await;

        assert!(matches!(
            result.unwrap_err(),
            crate::error::ArtfulError::Other(_)
        ));
    }
}
