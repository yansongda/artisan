//! 内置序列化器模块
//!
//! 导出所有内置的序列化器实现。
//!
//! # 内置序列化器
//!
//! | 序列化器 | 功能 |
//! |----------|------|
//! | [`JsonPacker`] | JSON 序列化/反序列化（默认） |
//! | [`QueryPacker`] | 表单 `application/x-www-form-urlencoded` 序列化/反序列化（RFC1738） |

mod json;
mod query;

pub use json::JsonPacker;
pub use query::QueryPacker;
