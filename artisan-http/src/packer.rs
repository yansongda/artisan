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
    /// `params` 携带调用方附加参数（如 `_unpack_raw`），实现方可忽略；
    /// 对齐 PHP `PackerInterface::pack` 的 `$params` 形参。
    ///
    /// # Errors
    ///
    /// 返回错误当序列化失败。
    fn pack(
        &self,
        data: &HashMap<String, Value>,
        params: &HashMap<String, Value>,
    ) -> Result<String>;

    /// 反序列化数据
    ///
    /// `params` 携带调用方附加参数（如 `_unpack_raw`），实现方可忽略；
    /// 对齐 PHP `PackerInterface::unpack` 的 `$params` 形参。
    ///
    /// # Errors
    ///
    /// 返回错误当反序列化失败。
    fn unpack(&self, data: &str, params: &HashMap<String, Value>) -> Result<Value>;

    /// 获取序列化后请求体的 Content-Type
    ///
    /// 返回 `None` 表示不声明 Content-Type（默认）。
    /// 框架仅在请求头缺失 `Content-Type` 时补填该值，不会覆盖用户显式设置。
    fn content_type(&self) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;

    #[derive(Debug)]
    struct NoContentTypePacker;

    impl Packer for NoContentTypePacker {
        fn pack(
            &self,
            _data: &HashMap<String, Value>,
            _params: &HashMap<String, Value>,
        ) -> Result<String> {
            Ok(String::new())
        }

        fn unpack(&self, _data: &str, _params: &HashMap<String, Value>) -> Result<Value> {
            Ok(Value::Null)
        }
    }

    #[test]
    fn default_content_type_is_none() {
        // trait 默认实现不声明 Content-Type
        assert_eq!(NoContentTypePacker.content_type(), None);
    }
}
