//! 序列化器 trait 定义
//!
//! 定义数据序列化/反序列化的抽象接口。
//!
//! # 内置实现
//!
//! - [`JsonPacker`](crate::packers::JsonPacker) - JSON 序列化器（默认）

use serde_json::Value;
use std::collections::HashMap;

use crate::Result;

/// 序列化器 trait
///
/// 定义数据序列化/反序列化的抽象接口，用于将 payload 与请求体互转。
pub trait Packer: Send + Sync + std::fmt::Debug {
    /// 序列化数据
    ///
    /// # Errors
    ///
    /// 返回错误当序列化失败。
    fn pack(&self, data: &HashMap<String, Value>) -> Result<String>;

    /// 反序列化数据
    ///
    /// # Errors
    ///
    /// 返回错误当反序列化失败。
    fn unpack(&self, data: &str) -> Result<Value>;

    /// 获取序列化后请求体的 Content-Type
    ///
    /// 返回 `None` 表示不声明 Content-Type（默认）。
    /// 框架仅在请求头缺失 `Content-Type` 时补填该值，不会覆盖用户显式设置。
    fn content_type(&self) -> Option<&'static str> {
        None
    }
}
