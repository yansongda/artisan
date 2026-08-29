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
