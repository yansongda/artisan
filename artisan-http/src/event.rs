//! 事件系统模块
//!
//! 定义请求生命周期事件（[`Event`]）、监听器（[`EventListener`]）与分发器
//! （[`EventDispatcher`]），为日志、metrics、审计等横切逻辑提供旁路挂点。
//!
//! # 生命周期事件
//!
//! | 事件 | 触发时机 | 可变性 |
//! |------|---------|--------|
//! | [`Event::ArtfulStart`] | 插件链启动前 | 只读 |
//! | [`Event::HttpStart`] | HTTP 请求即将发出（radar 已构建） | 可修改请求 |
//! | [`Event::HttpEnd`] | HTTP 请求成功返回、响应解析之前 | 只读 |
//! | [`Event::HttpError`] | HTTP 请求执行失败（错误照常向上传播） | 只读 |
//! | [`Event::ArtfulEnd`] | 插件链执行完毕、即将返回 destination | 可改写 destination |
//!
//! `Event` 是借用视图而非数据拷贝：变体内为对框架真实数据的引用，
//! 监听器对可变事件（如 [`Event::HttpStart`]）的修改在主流程真实生效；
//! 借用仅在 `on_event` 调用内有效，无法逃逸到监听器自身状态。
//!
//! # 语义约定
//!
//! - **同步执行**：监听器按注册顺序同步调用，**必须非阻塞**——
//!   耗时任务（IO、重计算）请自行 spawn，避免阻塞 tokio worker；
//! - **首错中止**：任一监听器返回 `Err` 即停止后续监听器，错误包装为
//!   [`ArtfulError::EventListenerError`] 向上传播、中断主流程；
//!   仅需旁路观察（如日志）的监听器应内部消化错误、恒返回 `Ok(())`；
//! - 监听器 panic 按 Rust 惯例直接传播（不做 `catch_unwind`）。
//!
//! # 示例
//!
//! ```
//! use artisan_http::{Event, EventDispatcher, EventListener};
//! use artisan_http::Result;
//! use std::sync::Arc;
//!
//! /// 旁路日志监听器：记录事件、内部消化错误（标准写法）
//! struct LoggingListener;
//!
//! impl EventListener for LoggingListener {
//!     fn name(&self) -> &'static str {
//!         "LoggingListener"
//!     }
//!
//!     fn on_event(&self, _event: &mut Event<'_>) -> Result<()> {
//!         // 真实场景在此记录日志 / 上报 metrics；不返回 Err 即不影响主流程
//!         Ok(())
//!     }
//! }
//!
//! let mut dispatcher = EventDispatcher::default();
//! dispatcher.add_listener(Arc::new(LoggingListener));
//! assert_eq!(dispatcher.len(), 1);
//! assert!(!dispatcher.is_empty());
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::Result;
use crate::error::ArtfulError;
use crate::plugin::Plugin;
use crate::rocket::Rocket;

/// 生命周期事件（借用视图，变体内为对框架真实数据的引用）
///
/// 可写权限由变体内部引用类型决定（`&mut Rocket` 可变 / `&Rocket` 只读）；
/// 事件集合封闭，枚举不可克隆。
pub enum Event<'a> {
    /// 插件链启动前（只读观测：params 已装入 rocket，plugins 未执行）
    ArtfulStart {
        /// 原始请求参数
        params: &'a HashMap<String, Value>,
        /// 即将执行的插件链
        plugins: &'a [Arc<dyn Plugin>],
    },
    /// HTTP 请求即将发出（radar 已构建；修改请求须经 `rocket.radar` 的
    /// `*_mut` 访问器——此时改 `rocket.config` 不影响本次请求）
    HttpStart {
        /// 请求载体（可修改 radar）
        rocket: &'a mut Rocket,
    },
    /// HTTP 请求成功返回（direction 解析之前；只读）
    HttpEnd {
        /// 请求载体（destination_origin 已就位）
        rocket: &'a Rocket,
    },
    /// HTTP 请求执行失败（仅指 execute 失败；错误照常向上传播；只读）
    HttpError {
        /// 请求载体
        rocket: &'a Rocket,
        /// 执行失败的原始错误（[`ArtfulError::RequestFailed`]）
        error: &'a ArtfulError,
    },
    /// 插件链执行完毕、即将返回 destination（可改写 `rocket.destination`）
    ArtfulEnd {
        /// 请求载体（可改写 destination）
        rocket: &'a mut Rocket,
    },
}

/// 事件监听器（对象安全，可存入 [`Arc`]；同步，禁止阻塞——耗时任务请自行 spawn）
pub trait EventListener: Send + Sync + 'static {
    /// 监听器名称（错误信息/调试用）
    fn name(&self) -> &'static str {
        "UnknownEventListener"
    }

    /// 处理事件；返回 `Err` 将中断后续监听器并中止主流程
    ///
    /// # Errors
    ///
    /// 监听器内部失败时返回错误，由分发器包装为
    /// [`ArtfulError::EventListenerError`] 传播；旁路观察型监听器应内部
    /// 消化错误、恒返回 `Ok(())`。
    fn on_event(&self, event: &mut Event<'_>) -> Result<()>;
}

/// 事件分发器（`Artful` 实例持有；`Clone` 共享监听器 `Arc`；空表时 dispatch 为 no-op）
///
/// 经 [`crate::ArtfulBuilder::event_listener`] 注册监听器，实例克隆后
/// 各克隆共享同一批监听器（内部状态共享，计数器等需自行加锁）。
#[derive(Clone, Default)]
pub struct EventDispatcher {
    listeners: Vec<Arc<dyn EventListener>>,
}

impl std::fmt::Debug for EventDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // dyn EventListener 不满足 Debug，仅打印监听器数量
        f.debug_struct("EventDispatcher")
            .field("listeners", &self.listeners.len())
            .finish()
    }
}

impl EventDispatcher {
    /// 追加一个监听器（注册顺序即执行顺序）
    pub fn add_listener(&mut self, listener: Arc<dyn EventListener>) {
        self.listeners.push(listener);
    }

    /// 返回已注册监听器数量
    pub fn len(&self) -> usize {
        self.listeners.len()
    }

    /// 是否未注册任何监听器
    pub fn is_empty(&self) -> bool {
        self.listeners.is_empty()
    }

    /// 按注册顺序同步分发事件；首错即中止并包装为 [`ArtfulError::EventListenerError`]
    ///
    /// # Errors
    ///
    /// 任一监听器返回 `Err` 时立即停止后续监听器，返回包装错误
    /// （`listener_name` 为该监听器名称，原始错误挂在 `source`）。
    pub(crate) fn dispatch(&self, mut event: Event<'_>) -> Result<()> {
        for listener in &self.listeners {
            // event 由本函数持有，每轮迭代重新可变借用，事件内引用在监听器间顺序复用
            match listener.on_event(&mut event) {
                Ok(()) => {}
                Err(err) => {
                    return Err(ArtfulError::EventListenerError {
                        listener_name: listener.name().to_string(),
                        message: err.to_string(),
                        source: Some(Box::new(err)),
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 测试监听器：记录自身名称；`fail` 为真时返回错误
    struct Recorder {
        name: &'static str,
        records: Arc<Mutex<Vec<&'static str>>>,
        fail: bool,
    }

    impl Recorder {
        fn new(name: &'static str, records: Arc<Mutex<Vec<&'static str>>>, fail: bool) -> Self {
            Self {
                name,
                records,
                fail,
            }
        }
    }

    impl EventListener for Recorder {
        fn name(&self) -> &'static str {
            self.name
        }

        fn on_event(&self, _event: &mut Event<'_>) -> Result<()> {
            self.records.lock().unwrap().push(self.name);

            if self.fail {
                return Err(ArtfulError::Other("listener boom".to_string()));
            }

            Ok(())
        }
    }

    #[test]
    fn empty_dispatcher_dispatch_is_noop() {
        // 空注册表：dispatch 为 no-op，返回 Ok
        let rocket = Rocket::new(HashMap::new());
        let dispatcher = EventDispatcher::default();

        let result = dispatcher.dispatch(Event::HttpEnd { rocket: &rocket });

        assert!(result.is_ok());
        assert!(dispatcher.is_empty());
        assert_eq!(dispatcher.len(), 0);
    }

    #[test]
    fn listeners_execute_in_registration_order() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = EventDispatcher::default();
        dispatcher.add_listener(Arc::new(Recorder::new("First", records.clone(), false)));
        dispatcher.add_listener(Arc::new(Recorder::new("Second", records.clone(), false)));

        let rocket = Rocket::new(HashMap::new());
        dispatcher
            .dispatch(Event::HttpEnd { rocket: &rocket })
            .unwrap();

        assert_eq!(*records.lock().unwrap(), vec!["First", "Second"]);
    }

    #[test]
    fn first_listener_error_aborts_and_wraps() {
        // 首监听器返回 Err：第二监听器不执行，错误包装为 EventListenerError
        let records = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = EventDispatcher::default();
        dispatcher.add_listener(Arc::new(Recorder::new("Failing", records.clone(), true)));
        dispatcher.add_listener(Arc::new(Recorder::new("Second", records.clone(), false)));

        let rocket = Rocket::new(HashMap::new());
        let result = dispatcher.dispatch(Event::HttpEnd { rocket: &rocket });

        match result.unwrap_err() {
            ArtfulError::EventListenerError {
                listener_name,
                source,
                ..
            } => {
                assert_eq!(listener_name, "Failing");
                assert!(source.is_some());
            }
            other => panic!("expected EventListenerError, got {other:?}"),
        }
        assert_eq!(*records.lock().unwrap(), vec!["Failing"]);
    }
}
