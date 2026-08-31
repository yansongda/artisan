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

mod add_payload_body;
mod add_radar;
mod start;

pub use add_payload_body::AddPayloadBodyPlugin;
pub use add_radar::AddRadarPlugin;
pub use start::StartPlugin;
