//! 初始化插件
//!
//! 请求链的起点插件，负责将原始参数初始化到 payload。
//!
//! # 行为
//!
//! 将 rocket.params 复制到 rocket.payload，使 payload 成为可修改的工作参数。

use async_trait::async_trait;

use crate::Rocket;
use crate::flow_ctrl::Next;
use crate::plugin::Plugin;

/// 初始化插件
#[derive(Clone, Copy, Debug, Default)]
pub struct StartPlugin;

#[async_trait]
impl Plugin for StartPlugin {
    fn name(&self) -> &'static str {
        "StartPlugin"
    }

    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
        if rocket.payload.is_empty() {
            rocket.merge_params_to_payload();
        }

        next.call(rocket).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_ctrl::FlowCtrl;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    async fn drive(rocket: &mut Rocket, extra: Option<Arc<dyn Plugin>>) -> crate::Result<()> {
        let mut plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(StartPlugin)];
        if let Some(extra) = extra {
            plugins.push(extra);
        }
        FlowCtrl::new(plugins).call_next(rocket).await
    }

    #[tokio::test]
    async fn merges_params_to_empty_payload() {
        let params = HashMap::from([("order_id".to_string(), json!("123"))]);
        let mut rocket = Rocket::new(params);

        drive(&mut rocket, None).await.unwrap();

        assert_eq!(rocket.payload.get("order_id"), Some(&json!("123")));
        // params 保持不变
        assert!(rocket.get_params().contains_key("order_id"));
    }

    #[tokio::test]
    async fn keeps_existing_payload_when_params_present() {
        // payload 已被先前插件填充时,不应再用 params 覆盖
        let params = HashMap::from([("outer".to_string(), json!("param"))]);
        let mut rocket = Rocket::new(params);
        rocket.payload.insert("inner".to_string(), json!("prefill"));

        drive(&mut rocket, None).await.unwrap();

        assert_eq!(rocket.payload.get("inner"), Some(&json!("prefill")));
        assert!(!rocket.payload.contains_key("outer"));
    }

    #[tokio::test]
    async fn no_op_when_both_empty() {
        let mut rocket = Rocket::new(HashMap::new());

        drive(&mut rocket, None).await.unwrap();

        assert!(rocket.payload.is_empty());
    }

    #[tokio::test]
    async fn passes_through_to_next_plugin() {
        struct MarkPlugin;

        #[async_trait]
        impl Plugin for MarkPlugin {
            async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
                rocket.payload.insert("marked".to_string(), json!(true));
                next.call(rocket).await
            }
        }

        let mut rocket = Rocket::new(HashMap::new());

        drive(&mut rocket, Some(Arc::new(MarkPlugin)))
            .await
            .unwrap();

        assert!(rocket.payload.contains_key("marked"));
    }
}
