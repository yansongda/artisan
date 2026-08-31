//! 链尾核心动作模块
//!
//! 执行 HTTP 请求并解析响应，是洋葱链的固定终点（对齐 artful PHP 的 `ignite()`）。
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
//! - `HttpStart`：到达链尾核心动作执行点、请求即将发出前（正常链中 radar 已构建、
//!   尚未被消费，监听器可经 `rocket.radar` 的 `*_mut` 访问器修改请求；链中缺
//!   `AddRadarPlugin` 时 radar 为 `None`，事件仍触发；此时改 `rocket.config`
//!   不影响本次请求）
//! - `HttpEnd`：请求成功返回后、响应解析之前（只读；响应体消费权属于
//!   direction 解析，只读视图下不可读 body，仅可读 status / headers）
//! - `HttpError`：请求执行失败时（错误照常向上传播；只读）
//!
//! 注意：监听器返回 `Err` 将中止请求流程，错误包装为
//! [`ArtfulError::EventListenerError`] 向上传播；其中 `HttpError` 分发中监听器
//! 自身失败时，向上传播的是 `EventListenerError`，原始 `RequestFailed`
//! 保留在其 `original` 字段（错误链不丢失）。
//!
//! `NoRequest` 方向不发起请求，不触发任何 HTTP 事件。
#![allow(dead_code)] // Task 2 接线后移除

use async_trait::async_trait;

use crate::Rocket;
use crate::direction::{Destination, Direction, DirectionKind};
use crate::directions::JsonDirection;
use crate::error::ArtfulError;
use crate::event::Event;
use crate::flow_ctrl::CoreAction;

/// 链尾核心动作
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IgniteCore;

#[async_trait]
impl CoreAction for IgniteCore {
    async fn run(&self, rocket: &mut Rocket) -> crate::Result<()> {
        // NoRequest - 不发起请求，直接结束（不触发任何 HTTP 事件）
        if let DirectionKind::NoRequest = rocket.config.direction {
            return Ok(());
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
                // 原始 RequestFailed 保留在其 original 字段（错误链不丢失）；
                // 否则错误照常传播
                if let Some(events) = &events {
                    if let Err(mut listener_err) = events.dispatch(Event::HttpError {
                        rocket: &*rocket,
                        error: &err,
                    }) {
                        if let ArtfulError::EventListenerError { original, .. } = &mut listener_err
                        {
                            *original = Some(Box::new(err));
                        }

                        return Err(listener_err);
                    }
                }

                return Err(err);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::test_util::VariantRecorder;
    use crate::event::{Event, EventDispatcher, EventListener};
    use crate::flow_ctrl::FlowCtrl;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn no_request_skips_execution() {
        // NoRequest:不发起请求,destination 保持 None
        let mut rocket = Rocket::new(HashMap::new());
        rocket.config.direction = DirectionKind::NoRequest;

        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(IgniteCore));
        ctrl.call_next(&mut rocket).await.unwrap();

        assert!(rocket.destination.is_none());
        assert!(rocket.destination_origin.is_none());
    }

    #[tokio::test]
    async fn missing_request_when_radar_absent() {
        // radar 为 None(链中缺少 AddRadarPlugin)→ MissingRequest,不触碰网络
        let mut rocket = Rocket::new(HashMap::new());

        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(IgniteCore));
        let result = ctrl.call_next(&mut rocket).await;

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

        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(IgniteCore));
        let result = ctrl.call_next(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingRequest));
        assert_eq!(*records.lock().unwrap(), vec!["HttpStart"]);
    }

    /// HttpError 分发时失败的测试监听器(其余事件恒 Ok)
    struct HttpErrorFailingListener;

    impl EventListener for HttpErrorFailingListener {
        fn name(&self) -> &'static str {
            "HttpErrorFailing"
        }

        fn on_event(&self, event: &mut Event<'_>) -> crate::Result<()> {
            if matches!(event, Event::HttpError { .. }) {
                return Err(ArtfulError::Other("listener boom".to_string()));
            }

            Ok(())
        }
    }

    #[tokio::test]
    async fn http_error_listener_failure_preserves_original() {
        // execute 失败且 HttpError 分发中监听器也失败:传播 EventListenerError,
        // 原始 RequestFailed 保留在 original 字段(错误链不丢失)
        // 向一个必然拒绝连接的地址发起请求(不经网络出网卡,回环即拒)
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut rocket = Rocket::new(HashMap::new());
        rocket.config.method = reqwest::Method::POST;
        rocket.config.url = format!("http://127.0.0.1:{port}/boom");
        rocket.client = reqwest::Client::new();
        rocket.radar = Some(
            rocket
                .client
                .request(rocket.config.method.clone(), &rocket.config.url)
                .build()
                .unwrap(),
        );

        let mut dispatcher = EventDispatcher::default();
        dispatcher.add_listener(Arc::new(HttpErrorFailingListener));
        rocket.events = Some(Arc::new(dispatcher));

        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(IgniteCore));
        let result = ctrl.call_next(&mut rocket).await;

        match result.unwrap_err() {
            ArtfulError::EventListenerError {
                listener_name,
                original,
                ..
            } => {
                assert_eq!(listener_name, "HttpErrorFailing");
                assert!(matches!(*original.unwrap(), ArtfulError::RequestFailed(_)));
            }
            other => panic!("expected EventListenerError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_events_field_no_dispatch() {
        // events 为 None:行为与改造前完全一致
        // 场景一:NoRequest 不发起请求(镜像 no_request_skips_execution 断言)
        let mut rocket = Rocket::new(HashMap::new());
        rocket.config.direction = DirectionKind::NoRequest;

        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(IgniteCore));
        ctrl.call_next(&mut rocket).await.unwrap();

        assert!(rocket.destination.is_none());
        assert!(rocket.destination_origin.is_none());

        // 场景二:radar 缺失返回 MissingRequest(镜像 missing_request_when_radar_absent 断言)
        let mut rocket = Rocket::new(HashMap::new());

        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(IgniteCore));
        let result = ctrl.call_next(&mut rocket).await;

        assert!(matches!(result.unwrap_err(), ArtfulError::MissingRequest));
    }
}
