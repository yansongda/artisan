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
//!
//! # 事件分发点
//!
//! 请求执行路径上分发三个 HTTP 生命周期事件（需 `Artful` 实例注册监听器，
//! 经 `rocket.events` 传载）：
//!
//! - `HttpStart`：请求即将发出前（radar 已构建、尚未被消费，监听器可经
//!   `rocket.radar` 的 `*_mut` 访问器修改请求；此时改 `rocket.config` 不影响本次请求）
//! - `HttpEnd`：请求成功返回后、响应解析之前（只读）
//! - `HttpError`：请求执行失败时（错误照常向上传播；只读）
//!
//! 注意：监听器返回 `Err` 将中止请求流程，错误包装为
//! [`ArtfulError::EventListenerError`] 向上传播；其中 `HttpError` 分发中监听器
//! 自身失败时，向上传播的是 `EventListenerError`，原始 `RequestFailed`
//! 不再出现在错误链中。
//!
//! `NoRequest` 方向不发起请求，不触发任何 HTTP 事件。

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction, DirectionKind};
use crate::directions::JsonDirection;
use crate::error::ArtfulError;
use crate::event::Event;
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
        // NoRequest - 不发起请求，直接调用下一层（不触发任何 HTTP 事件）
        if let DirectionKind::NoRequest = rocket.config.direction {
            return next.call(rocket).await;
        }

        // 先克隆分发器 Arc 再分发，规避 rocket 的自借用冲突
        let events = rocket.events.clone();

        // HttpStart：请求即将发出（radar 尚未被消费，监听器可见并可修改 radar）
        if let Some(events) = &events {
            events.dispatch(Event::HttpStart {
                rocket: &mut *rocket,
            })?;
        }

        // 发送 HTTP 请求
        let response = rocket
            .client
            .execute(rocket.radar.take().ok_or(ArtfulError::MissingRequest)?)
            .await
            .map_err(ArtfulError::RequestFailed);

        match response {
            Ok(response) => {
                rocket.destination_origin = Some(response);

                // HttpEnd：请求成功返回、响应解析之前
                if let Some(events) = &events {
                    events.dispatch(Event::HttpEnd { rocket: &*rocket })?;
                }

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
            }
            Err(err) => {
                // HttpError：仅 execute 失败触发（MissingRequest 属请求前置失败，不触发）；
                // 若分发中监听器自身返回 Err，向上传播的是 EventListenerError，
                // 原始 RequestFailed 不再出现在错误链中；否则错误照常传播
                if let Some(events) = &events {
                    events.dispatch(Event::HttpError {
                        rocket: &*rocket,
                        error: &err,
                    })?;
                }

                return Err(err);
            }
        }

        next.call(rocket).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, EventDispatcher, EventListener};
    use crate::flow_ctrl::FlowCtrl;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// 记录事件变体名的测试监听器
    struct VariantRecorder {
        records: Arc<Mutex<Vec<&'static str>>>,
    }

    impl EventListener for VariantRecorder {
        fn name(&self) -> &'static str {
            "VariantRecorder"
        }

        fn on_event(&self, event: &mut Event<'_>) -> crate::Result<()> {
            let name = match event {
                Event::ArtfulStart { .. } => "ArtfulStart",
                Event::HttpStart { .. } => "HttpStart",
                Event::HttpEnd { .. } => "HttpEnd",
                Event::HttpError { .. } => "HttpError",
                Event::ArtfulEnd { .. } => "ArtfulEnd",
            };
            self.records.lock().unwrap().push(name);

            Ok(())
        }
    }

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

    #[tokio::test]
    async fn missing_request_fires_http_start_only() {
        // radar 为 None:HttpStart 已分发(radar.take 之前),但 MissingRequest
        // 属请求前置失败,不触发 HttpError
        let records = Arc::new(Mutex::new(Vec::new()));
        let mut rocket = Rocket::new(HashMap::new());

        let mut dispatcher = EventDispatcher::default();
        dispatcher.add_listener(Arc::new(VariantRecorder {
            records: records.clone(),
        }));
        rocket.events = Some(Arc::new(dispatcher));

        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ParserPlugin)];
        let result = FlowCtrl::new(plugins).call_next(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingRequest));
        assert_eq!(*records.lock().unwrap(), vec!["HttpStart"]);
    }

    #[tokio::test]
    async fn no_events_field_no_dispatch() {
        // events 为 None:行为与改造前完全一致
        // 场景一:NoRequest 不发起请求(镜像 no_request_skips_execution 断言)
        let mut rocket = Rocket::new(HashMap::new());
        rocket.config.direction = DirectionKind::NoRequest;

        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ParserPlugin)];
        FlowCtrl::new(plugins).call_next(&mut rocket).await.unwrap();

        assert!(rocket.destination.is_none());
        assert!(rocket.destination_origin.is_none());

        // 场景二:radar 缺失返回 MissingRequest(镜像 missing_request_when_radar_absent 断言)
        let mut rocket = Rocket::new(HashMap::new());

        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(ParserPlugin)];
        let result = FlowCtrl::new(plugins).call_next(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingRequest));
    }
}
