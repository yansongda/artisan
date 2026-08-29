//! JSON 序列化器实现
//!
//! 实现 [`Packer`] trait，提供 JSON 序列化/反序列化功能。

use serde_json::Value;
use std::collections::HashMap;

use crate::Result;
use crate::packer::Packer;

/// JSON 序列化器
///
/// 使用 [`serde_json`] 实现 [`Packer`] trait，为默认序列化器。
#[derive(Debug)]
pub struct JsonPacker;

impl Packer for JsonPacker {
    /// 将 HashMap 序列化为 JSON 字符串
    ///
    /// # Errors
    ///
    /// 返回错误当序列化失败。
    fn pack(&self, data: &HashMap<String, Value>) -> Result<String> {
        serde_json::to_string(data).map_err(Into::into)
    }

    /// 将 JSON 字符串反序列化为 Value
    ///
    /// # Errors
    ///
    /// 返回错误当反序列化失败。
    fn unpack(&self, data: &str) -> Result<Value> {
        serde_json::from_str(data).map_err(|e| crate::error::ArtfulError::JsonDeserializeError {
            message: e.to_string(),
            source: Some(e),
        })
    }

    /// JSON 请求体的 Content-Type
    fn content_type(&self) -> Option<&'static str> {
        Some("application/json")
    }
}
