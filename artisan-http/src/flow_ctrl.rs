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

use crate::Rocket;
use crate::plugin::Plugin;

/// 洋葱模型流向控制器
pub struct FlowCtrl {
    /// 当前执行位置
    cursor: usize,

    /// 插件列表
    plugins: Vec<Arc<dyn Plugin>>,

    /// 是否已终止
    is_ceased: bool,
}

impl std::fmt::Debug for FlowCtrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowCtrl")
            .field("cursor", &self.cursor)
            .field("plugins_count", &self.plugins.len())
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
            is_ceased: false,
        }
    }

    /// 调用下一层插件（洋葱穿透）
    ///
    /// # Errors
    ///
    /// 返回错误当插件执行失败。
    pub async fn call_next(&mut self, rocket: &mut Rocket) -> crate::Result<()> {
        if self.is_ceased || !self.has_next() {
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
    use std::sync::Arc;

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
