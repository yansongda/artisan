//! 解析响应插件
//!
//! 执行 HTTP 请求并解析响应。
//!
//! # 行为
//!
//! - 检查 rocket.config.direction，决定是否发起请求
//! - 执行 HTTP 请求，存入 `rocket.destination_origin`
//! - 根据 `DirectionKind` 解析响应
//! - 结果存入 rocket.destination

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction, DirectionKind};
use crate::directions::JsonDirection;
use crate::error::ArtfulError;
use crate::flow_ctrl::Next;
use crate::plugin::Plugin;

/// 解析响应插件
#[derive(Clone, Copy, Debug, Default)]
pub struct ParserPlugin;

#[async_trait]
impl Plugin for ParserPlugin {
    fn name(&self) -> &'static str {
        "ParserPlugin"
    }

    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
        // NoRequest - 不发起请求，直接调用下一层
        if let DirectionKind::NoRequest = rocket.config.direction {
            return next.call(rocket).await;
        }

        // 发送 HTTP 请求
        rocket.destination_origin = Some(
            rocket
                .client
                .execute(rocket.radar.take().ok_or(ArtfulError::MissingRequest)?)
                .await
                .map_err(ArtfulError::RequestFailed)?,
        );

        // 解析响应
        let destination = match &rocket.config.direction {
            DirectionKind::Json => JsonDirection.parse(rocket).await?,
            DirectionKind::Response => rocket
                .destination_origin
                .take()
                .map(Destination::Response)
                .ok_or(ArtfulError::MissingResponse)?,
            DirectionKind::Custom(direction) => direction.clone().parse(rocket).await?,
            DirectionKind::NoRequest => Destination::None,
        };

        rocket.destination = Some(destination);

        next.call(rocket).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_ctrl::FlowCtrl;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[tokio::test]
    async fn no_request_skips_execution() {
        // NoRequest:不发起请求,destination 保持 None
        let mut rocket = Rocket::new(HashMap::new());
        rocket.config.direction = DirectionKind::NoRequest;

        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ParserPlugin)];
        FlowCtrl::new(plugins).call_next(&mut rocket).await.unwrap();

        assert!(rocket.destination.is_none());
        assert!(rocket.destination_origin.is_none());
    }

    #[tokio::test]
    async fn missing_request_when_radar_absent() {
        // radar 为 None(链中缺少 AddRadarPlugin)→ MissingRequest,不触碰网络
        let mut rocket = Rocket::new(HashMap::new());

        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ParserPlugin)];
        let result = FlowCtrl::new(plugins).call_next(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingRequest));
    }
}
