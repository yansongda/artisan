//! 内置解析方向模块
//!
//! 导出所有内置的响应解析器实现。
//!
//! # 内置解析器
//!
//! | 解析器 | 功能 |
//! |--------|------|
//! | [`JsonDirection`] | 解析响应为 JSON |
//! | [`NoHttpRequestDirection`] | 不解析，透传 `rocket.destination` 现有值 |
//! | [`OriginResponseDirection`] | 不解析，返回原始 HTTP Response |

mod json;
mod no_http_request;
mod origin_response;

pub use json::JsonDirection;
pub use no_http_request::NoHttpRequestDirection;
pub use origin_response::OriginResponseDirection;
