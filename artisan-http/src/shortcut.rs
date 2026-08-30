//! 快捷方式 trait 定义
//!
//! 定义插件组合的快捷方式接口。
//!
//! 用于简化多个 API 使用相同插件组合的场景。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::plugin::Plugin;

/// 快捷方式 trait
///
/// 该 trait 是 dyn compatible 的，可以用作 trait object：
///
/// ```ignore
/// let shortcuts: Vec<Box<dyn Shortcut>> = vec![...];
/// ```
pub trait Shortcut {
    /// 根据参数返回插件列表
    fn get_plugins(&self, params: &HashMap<String, Value>) -> Vec<Arc<dyn Plugin>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};

    struct TestShortcut;

    impl Shortcut for TestShortcut {
        fn get_plugins(&self, _params: &HashMap<String, Value>) -> Vec<Arc<dyn Plugin>> {
            vec![
                Arc::new(StartPlugin),
                Arc::new(AddPayloadBodyPlugin),
                Arc::new(AddRadarPlugin),
                Arc::new(ParserPlugin),
            ]
        }
    }

    #[test]
    fn test_shortcut_basic() {
        let shortcut = TestShortcut;
        let plugins = shortcut.get_plugins(&HashMap::new());

        assert_eq!(plugins.len(), 4);
    }
}
