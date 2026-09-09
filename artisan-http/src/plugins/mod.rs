//! 内置插件模块
//!
//! 导出所有内置插件实现。
//!
//! # 内置插件列表
//!
//! | 插件 | 功能 |
//! |------|------|
//! | [`StartPlugin`] | 将原始参数初始化到 payload |
//! | [`AddPayloadBodyPlugin`] | 将 payload 序列化为请求体 |
//! | [`AddRadarPlugin`] | 构建 HTTP Request |
//! | [`ParserPlugin`] | 解析响应为 destination，必须挂在链尾 |

use std::collections::HashMap;

use serde_json::Value;

mod add_payload_body;
mod add_radar;
mod parser;
mod start;

pub use add_payload_body::AddPayloadBodyPlugin;
pub use add_radar::AddRadarPlugin;
pub use parser::ParserPlugin;
pub use start::StartPlugin;

/// 过滤用于请求体序列化的 payload：剔除 `_` 前缀键与 `null` 值
///
/// 对齐 PHP artful 的 `filter_params()`（`AddPayloadBodyPlugin` 打包请求体时
/// 调用，见 `src/Functions.php` / `src/Plugin/AddPayloadBodyPlugin.php`）：
/// `_unpack_raw` 等控制参数只应影响本侧行为，不得进入发往网关的请求体
/// （银联等网关对全部报文字段验签，多出的字段会导致验签失败）。
/// 响应解包侧不做此过滤（对齐 PHP `guide()` 第三参传全量 payload）。
pub(crate) fn filter_params(payload: &HashMap<String, Value>) -> HashMap<String, Value> {
    payload
        .iter()
        .filter(|(k, v)| !k.starts_with('_') && !v.is_null())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
