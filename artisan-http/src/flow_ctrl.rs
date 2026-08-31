//! 流向控制器模块
//!
//! 管理洋葱模型的插件执行流程。
//!
//! # 核心类型
//!
//! - [`FlowCtrl`] - 流向控制器，管理插件执行顺序
//! - [`Next`] - 闭包穿透，调用下一个插件
//!
//! # 执行流程
//!
//! 插件按顺序执行：前向阶段层层穿透，后向阶段层层返回。

use std::sync::Arc;

use async_trait::async_trait;

use crate::Rocket;
use crate::plugin::Plugin;

/// 链尾核心动作 trait - 洋葱链固定终点
///
/// 终点无下一层：执行完毕即整个链路结束，返回值沿洋葱后向阶段回退传播。
#[async_trait]
pub(crate) trait CoreAction: Send + Sync {
    /// 执行核心动作
    ///
    /// # Errors
    ///
    /// 返回错误当核心动作执行失败。
    async fn run(&self, rocket: &mut Rocket) -> crate::Result<()>;
}

/// 洋葱模型流向控制器
pub struct FlowCtrl {
    /// 当前执行位置
    cursor: usize,

    /// 插件列表
    plugins: Vec<Arc<dyn Plugin>>,

    /// 链尾核心动作（终点，一次性消费）
    core: Option<Arc<dyn CoreAction>>,

    /// 是否已终止
    is_ceased: bool,
}

impl std::fmt::Debug for FlowCtrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowCtrl")
            .field("cursor", &self.cursor)
            .field("plugins_count", &self.plugins.len())
            .field("core", &self.core.is_some())
            .field("is_ceased", &self.is_ceased)
            .finish()
    }
}

impl FlowCtrl {
    /// 创建新的流向控制器
    #[must_use]
    pub fn new(plugins: Vec<Arc<dyn Plugin>>) -> Self {
        Self {
            cursor: 0,
            plugins,
            core: None,
            is_ceased: false,
        }
    }

    /// 设置链尾核心动作
    pub(crate) fn set_core(&mut self, core: Arc<dyn CoreAction>) {
        self.core = Some(core);
    }

    /// 调用下一层插件（洋葱穿透）
    ///
    /// # Errors
    ///
    /// 返回错误当插件执行失败。
    pub async fn call_next(&mut self, rocket: &mut Rocket) -> crate::Result<()> {
        if self.is_ceased {
            return Ok(());
        }

        if !self.has_next() {
            // 链尾：执行核心动作，返回值沿洋葱后向阶段回退传播；
            // 未挂 core 时行为与纯插件链直用场景一致（静默结束）
            if let Some(core) = self.core.take() {
                return core.run(rocket).await;
            }

            return Ok(());
        }

        let plugin = self.plugins[self.cursor].clone();
        self.cursor += 1;
        plugin.assembly(rocket, Next { ctrl: self }).await
    }

    /// 检查是否还有下一层
    #[must_use]
    pub fn has_next(&self) -> bool {
        self.cursor < self.plugins.len()
    }

    /// 跳过剩余所有插件
    pub fn skip_rest(&mut self) {
        self.cursor = self.plugins.len();
        self.is_ceased = true;
    }

    /// 检查是否已终止
    #[must_use]
    pub fn is_ceased(&self) -> bool {
        self.is_ceased
    }
}

/// 下一个插件的闭包（洋葱穿透）
pub struct Next<'a> {
    pub(crate) ctrl: &'a mut FlowCtrl,
}

impl Next<'_> {
    /// 调用下一个插件
    ///
    /// # Errors
    ///
    /// 返回错误当插件执行失败。
    pub async fn call(self, rocket: &mut Rocket) -> crate::Result<()> {
        self.ctrl.call_next(rocket).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct TestPlugin {
        name: String,
    }

    #[async_trait]
    impl Plugin for TestPlugin {
        async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
            rocket
                .payload
                .insert("visited".to_string(), serde_json::json!(self.name.clone()));
            next.call(rocket).await
        }
    }

    #[tokio::test]
    async fn test_flow_ctrl_basic() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(TestPlugin {
                name: "plugin1".to_string(),
            }),
            Arc::new(TestPlugin {
                name: "plugin2".to_string(),
            }),
        ];

        let mut ctrl = FlowCtrl::new(plugins);
        let mut rocket = Rocket::new(HashMap::new());

        ctrl.call_next(&mut rocket).await.unwrap();

        assert!(rocket.payload.contains_key("visited"));
    }

    #[tokio::test]
    async fn test_flow_ctrl_cease() {
        struct CeasePlugin;

        #[async_trait]
        impl Plugin for CeasePlugin {
            async fn assembly(&self, rocket: &mut Rocket, _next: Next<'_>) -> crate::Result<()> {
                rocket
                    .payload
                    .insert("ceased".to_string(), serde_json::json!(true));
                Ok(())
            }
        }

        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(CeasePlugin),
            Arc::new(TestPlugin {
                name: "should_not_run".to_string(),
            }),
        ];

        let mut ctrl = FlowCtrl::new(plugins);
        let mut rocket = Rocket::new(HashMap::new());

        ctrl.call_next(&mut rocket).await.unwrap();

        assert!(rocket.payload.contains_key("ceased"));
        assert!(!rocket.payload.contains_key("visited"));
    }

    #[test]
    fn test_flow_ctrl_has_next() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(TestPlugin {
            name: "p1".to_string(),
        })];

        let ctrl = FlowCtrl::new(plugins);
        assert!(ctrl.has_next());
    }

    #[test]
    fn test_flow_ctrl_no_next() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![];
        let ctrl = FlowCtrl::new(plugins);
        assert!(!ctrl.has_next());
    }

    #[test]
    fn test_flow_ctrl_skip_rest() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(TestPlugin {
                name: "p1".to_string(),
            }),
            Arc::new(TestPlugin {
                name: "p2".to_string(),
            }),
            Arc::new(TestPlugin {
                name: "p3".to_string(),
            }),
        ];

        let mut ctrl = FlowCtrl::new(plugins);
        assert!(ctrl.has_next());
        assert!(!ctrl.is_ceased());

        ctrl.skip_rest();

        assert!(!ctrl.has_next());
        assert!(ctrl.is_ceased());
    }

    #[test]
    fn test_flow_ctrl_is_ceased() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![Arc::new(TestPlugin {
            name: "p1".to_string(),
        })];
        let ctrl = FlowCtrl::new(plugins.clone());
        assert!(!ctrl.is_ceased());

        let mut ceased_ctrl = FlowCtrl::new(plugins);
        ceased_ctrl.skip_rest();
        assert!(ceased_ctrl.is_ceased());
    }

    #[test]
    fn test_flow_ctrl_debug() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(TestPlugin {
                name: "p1".to_string(),
            }),
            Arc::new(TestPlugin {
                name: "p2".to_string(),
            }),
        ];

        let ctrl = FlowCtrl::new(plugins);
        let debug_str = format!("{:?}", ctrl);

        assert!(debug_str.contains("cursor"));
        assert!(debug_str.contains("plugins_count"));
        assert!(debug_str.contains("is_ceased"));
    }

    #[tokio::test]
    async fn test_flow_ctrl_empty_plugins() {
        let plugins: Vec<Arc<dyn Plugin>> = vec![];
        let mut ctrl = FlowCtrl::new(plugins);
        let mut rocket = Rocket::new(HashMap::new());

        let result = ctrl.call_next(&mut rocket).await;
        assert!(result.is_ok());
        assert!(rocket.payload.is_empty());
    }

    /// 链尾核心动作测试替身：执行时向 payload 插入标记
    struct MarkCore;

    #[async_trait]
    impl CoreAction for MarkCore {
        async fn run(&self, rocket: &mut Rocket) -> crate::Result<()> {
            rocket
                .payload
                .insert("core_ran".to_string(), serde_json::json!(true));

            Ok(())
        }
    }

    #[tokio::test]
    async fn test_flow_ctrl_core_runs_at_tail() {
        // 空插件链 + set_core：链尾核心动作执行
        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(MarkCore));
        let mut rocket = Rocket::new(HashMap::new());

        ctrl.call_next(&mut rocket).await.unwrap();

        assert!(rocket.payload.contains_key("core_ran"));
    }

    #[tokio::test]
    async fn test_flow_ctrl_core_skipped_after_skip_rest() {
        // skip_rest 后核心动作不执行（主动中止优先于终点）
        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(MarkCore));
        let mut rocket = Rocket::new(HashMap::new());

        ctrl.skip_rest();

        let result = ctrl.call_next(&mut rocket).await;
        assert!(result.is_ok());
        assert!(rocket.payload.is_empty());
    }

    #[tokio::test]
    async fn test_flow_ctrl_core_runs_only_once() {
        // 核心动作执行后再次 call_next：take 已消费，回落 Ok(())（幂等）
        struct CountCore {
            count: Arc<Mutex<usize>>,
        }

        #[async_trait]
        impl CoreAction for CountCore {
            async fn run(&self, _rocket: &mut Rocket) -> crate::Result<()> {
                *self.count.lock().unwrap() += 1;

                Ok(())
            }
        }

        let count = Arc::new(Mutex::new(0));
        let mut ctrl = FlowCtrl::new(vec![]);
        ctrl.set_core(Arc::new(CountCore {
            count: count.clone(),
        }));
        let mut rocket = Rocket::new(HashMap::new());

        ctrl.call_next(&mut rocket).await.unwrap();
        let result = ctrl.call_next(&mut rocket).await;

        assert!(result.is_ok());
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_flow_ctrl_call_next_after_skip_rest() {
        struct MarkPlugin {
            name: String,
        }

        #[async_trait]
        impl Plugin for MarkPlugin {
            async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> crate::Result<()> {
                rocket
                    .payload
                    .insert(self.name.clone(), serde_json::json!(true));
                next.call(rocket).await
            }
        }

        let plugins: Vec<Arc<dyn Plugin>> = vec![
            Arc::new(MarkPlugin {
                name: "first".to_string(),
            }),
            Arc::new(MarkPlugin {
                name: "second".to_string(),
            }),
        ];

        let mut ctrl = FlowCtrl::new(plugins);
        let mut rocket = Rocket::new(HashMap::new());

        // 先手动调用 skip_rest
        ctrl.skip_rest();

        // 调用 call_next 应该立即返回 Ok(())
        let result = ctrl.call_next(&mut rocket).await;
        assert!(result.is_ok());
        assert!(rocket.payload.is_empty());
    }
}
