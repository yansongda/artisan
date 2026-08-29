//! 框架配置模块
//!
//! 定义框架级别的配置 [`Config`]。

use serde_json::Value;
use std::collections::HashMap;

use crate::rocket::ClientOptions;

/// 框架全局配置
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// HTTP 客户端级默认选项
    pub http: ClientOptions,

    /// 扩展配置：支持任意渠道/模块参数
    ///
    /// # 示例
    ///
    /// ```rust
    /// use artisan_http::Config;
    /// use serde_json::json;
    /// use std::collections::HashMap;
    ///
    /// let mut extra = HashMap::new();
    /// extra.insert("name".to_string(), json!("yansongda"));
    /// extra.insert("http".to_string(), json!({"timeout": 5.0}));
    ///
    /// let config = Config {
    ///     extra,
    ///     ..Default::default()
    /// };
    /// ```
    pub extra: HashMap<String, Value>,
}
