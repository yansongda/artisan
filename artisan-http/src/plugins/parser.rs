//! 后置响应解析插件
//!
//! 对齐 artful PHP 的 `ParserPlugin`：在洋葱链后向阶段把响应解析为
//! [`Destination`](crate::direction::Destination)。
//!
//! # 执行时机
//!
//! 本插件是后置插件，必须挂在链尾：`assembly` 前向阶段直接穿透调用下一层，
//! HTTP 请求由内层完成（`AddRadarPlugin` 构建请求、链尾核心动作
//! `IgniteCore` 执行并把响应写入 `destination_origin`）后，本插件在后向
//! 阶段按 `rocket.config.direction` 分发解析方向，把 `destination_origin`
//! 解析为 `rocket.destination`。
//!
//! # 与 IgniteCore 的分工
//!
//! [`IgniteCore`](crate::ignite::IgniteCore) 仅执行 HTTP 请求（execute +
//! 事件分发），不解析；响应解析由本插件在链尾后向阶段完成。0.16.0 中
//! 该逻辑曾内联于 IgniteCore，0.17.0 起移回插件形态，二者解析语义一致。
//!
//! # params 传递语义
//!
//! 解析方向内部把 `rocket.payload` 全量作为 params 传给 packer 的
//! [`unpack`](crate::packer::Packer::unpack)（不过滤 `_` 前缀特殊参数，
//! 对齐 PHP `$payload?->all()`），因此 QueryPacker 的 `_unpack_raw` 等
//! 特殊参数可经 payload 直接生效。
//!
//! # 与 PHP 的响应来源差异
//!
//! PHP 版读取 `destination`（ignite 同时写 destination/destinationOrigin），
//! Rust 版读取 `destination_origin`。正常链路二者等价；但用户在前向插件中
//! 预置 `Some(Destination::Response)` 时语义不同：PHP 会解析该预置响应，
//! Rust 则解析 `destination_origin`（预置值仅用于守卫放行）。
//!
//! # 守卫
//!
//! `rocket.destination` 只能是 `None` 或
//! [`Destination::Response`](crate::direction::Destination)（对齐 PHP
//! `InvalidParamsException` 9208：解析插件中 destination 只能是 null 或
//! `ResponseInterface`）；否则返回 [`ArtfulError::InvalidParameter`]。

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction, DirectionKind};
use crate::directions::{JsonDirection, NoHttpRequestDirection, OriginResponseDirection};
use crate::error::ArtfulError;
use crate::flow_ctrl::Next;
use crate::plugin::Plugin;

/// 后置响应解析插件：解析响应为 destination，必须挂在链尾
#[derive(Clone, Copy, Debug, Default)]
pub struct ParserPlugin;

#[async_trait]
impl Plugin for ParserPlugin {
    fn name(&self) -> &'static str {
        "ParserPlugin"
    }

    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
        // 后置插件：前向直接穿透，HTTP 完成后在后向阶段解析
        next.call(rocket).await?;

        // 分发解析方向（0.16.0 曾内联于 IgniteCore Ok 分支，0.17.0 起由本插件承担）
        // 守卫：destination 只能是 None 或 Response（对齐 PHP 9208）
        if let Some(Destination::Json(_)) = rocket.destination {
            return Err(ArtfulError::InvalidParameter {
                param: "destination".to_string(),
                message: "ParserPlugin 中 Rocket 的 destination 只能是 None 或 Response"
                    .to_string(),
            });
        }

        let destination = match &rocket.config.direction {
            DirectionKind::Json => JsonDirection.parse(rocket).await?,
            DirectionKind::Response => OriginResponseDirection.parse(rocket).await?,
            // 透传 destination 现有值（无值时为 Destination::None）
            DirectionKind::NoRequest => NoHttpRequestDirection.parse(rocket).await?,
            DirectionKind::Custom(direction) => direction.clone().parse(rocket).await?,
        };

        rocket.destination = Some(destination);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_ctrl::FlowCtrl;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

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

    /// 经 FlowCtrl 直接驱动 ParserPlugin(链尾静默结束，不经 IgniteCore)
    async fn drive(rocket: &mut Rocket) -> crate::Result<()> {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ParserPlugin)];
        FlowCtrl::new(plugins).call_next(rocket).await
    }

    #[tokio::test]
    async fn parses_origin_into_json_destination() {
        // happy:预置原始响应 + 默认 Json direction → 解析为 Destination::Json
        let mut rocket = rocket_with_response(r#"{"ok":true}"#);

        drive(&mut rocket).await.unwrap();

        match rocket.destination {
            Some(Destination::Json(value)) => assert_eq!(value["ok"], true),
            other => panic!("Expected JSON destination, got {:?}", other),
        }
        // 原始响应已被解析消费
        assert!(rocket.destination_origin.is_none());
    }

    #[tokio::test]
    async fn rejects_non_response_destination() {
        // 守卫:destination 预置为 Json(非 None/Response)→ InvalidParameter(对齐 PHP 9208)
        let mut rocket = Rocket::new(HashMap::new());
        rocket.destination = Some(Destination::Json(json!({"x": 1})));

        let result = drive(&mut rocket).await;

        assert!(matches!(
            result.unwrap_err(),
            ArtfulError::InvalidParameter { param, .. } if param == "destination"
        ));
    }

    #[tokio::test]
    async fn wraps_origin_with_response_direction() {
        // Response direction:原始响应原样包装为 Destination::Response
        let mut rocket = rocket_with_response("raw body");
        rocket.config.direction = DirectionKind::Response;

        drive(&mut rocket).await.unwrap();

        assert!(matches!(rocket.destination, Some(Destination::Response(_))));
        assert!(rocket.destination_origin.is_none());
    }

    #[tokio::test]
    async fn passes_none_with_no_request_direction() {
        // NoRequest direction:不预置 origin,透传现有 destination(无值 → Destination::None),无错误
        let mut rocket = Rocket::new(HashMap::new());
        rocket.config.direction = DirectionKind::NoRequest;

        drive(&mut rocket).await.unwrap();

        assert!(matches!(rocket.destination, Some(Destination::None)));
    }

    #[tokio::test]
    async fn dispatches_to_custom_direction() {
        // Custom direction:走自定义解析器
        #[derive(Debug)]
        struct CustomDirection;

        #[async_trait]
        impl Direction for CustomDirection {
            async fn parse(&self, _rocket: &mut Rocket) -> crate::Result<Destination> {
                Ok(Destination::Json(json!({"custom": true})))
            }
        }

        let mut rocket = Rocket::new(HashMap::new());
        rocket.config.direction = DirectionKind::Custom(Arc::new(CustomDirection));

        drive(&mut rocket).await.unwrap();

        match rocket.destination {
            Some(Destination::Json(value)) => assert_eq!(value["custom"], true),
            other => panic!(
                "Expected JSON destination from custom direction, got {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn probe_plugin_observes_onion_order() {
        // 链位置验证:探针在链首(最外层),ParserPlugin 在内层
        // 洋葱序:探针前向 → ParserPlugin 前向(穿透,链尾静默结束) →
        //        ParserPlugin 后向(解析) → 探针后向
        // 证明解析发生在内层(HTTP 之后),而非探针前向阶段
        struct ProbePlugin {
            records: Arc<Mutex<Vec<&'static str>>>,
        }

        #[async_trait]
        impl Plugin for ProbePlugin {
            fn name(&self) -> &'static str {
                "ProbePlugin"
            }

            async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
                // 前向阶段:解析尚未发生
                self.records
                    .lock()
                    .unwrap()
                    .push(if rocket.destination.is_none() {
                        "forward: destination is none"
                    } else {
                        "forward: destination is filled"
                    });

                next.call(rocket).await?;

                // 后向阶段:ParserPlugin 已在内层完成解析
                self.records
                    .lock()
                    .unwrap()
                    .push(if rocket.destination.is_some() {
                        "backward: destination is filled"
                    } else {
                        "backward: destination is none"
                    });

                Ok(())
            }
        }

        let records = Arc::new(Mutex::new(Vec::new()));
        let mut rocket = rocket_with_response(r#"{"ok":true}"#);

        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(ProbePlugin {
                records: records.clone(),
            }),
            Arc::new(ParserPlugin),
        ];
        FlowCtrl::new(plugins).call_next(&mut rocket).await.unwrap();

        assert_eq!(
            *records.lock().unwrap(),
            vec![
                "forward: destination is none",
                "backward: destination is filled"
            ]
        );
        // 最终 destination 已被 ParserPlugin 填充
        assert!(matches!(rocket.destination, Some(Destination::Json(_))));
    }
}
