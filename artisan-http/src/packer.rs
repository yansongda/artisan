//! 序列化器 trait 定义
//!
//! 定义数据序列化/反序列化的抽象接口。
//!
//! # 内置实现
//!
//! - [`JsonPacker`](crate::packers::JsonPacker) - JSON 序列化器（默认）
//!
//! # 强类型入口
//!
//! - [`pack_typed`] - 将 `Serialize` 类型序列化为请求体字符串（要求序列化为 JSON 对象）
//! - [`unpack_typed`] - 将响应体字符串反序列化为 `DeserializeOwned` 类型

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::Result;
use crate::error::ArtfulError;

/// 序列化器 trait
///
/// 定义数据序列化/反序列化的抽象接口，用于将 payload 与请求体互转。
pub trait Packer: Send + Sync + std::fmt::Debug {
    /// 序列化数据
    ///
    /// `params` 携带调用方附加参数（如 `_unpack_raw`），实现方可忽略。
    ///
    /// # Errors
    ///
    /// 返回错误当序列化失败。
    fn pack(&self, data: &Map<String, Value>, params: &Map<String, Value>) -> Result<String>;

    /// 反序列化数据
    ///
    /// `params` 携带调用方附加参数（如 `_unpack_raw`），实现方可忽略。
    ///
    /// # Errors
    ///
    /// 返回错误当反序列化失败。
    fn unpack(&self, data: &str, params: &Map<String, Value>) -> Result<Value>;

    /// 获取序列化后请求体的 Content-Type
    ///
    /// 返回 `None` 表示不声明 Content-Type（默认）。
    /// 框架仅在请求头缺失 `Content-Type` 时补填该值，不会覆盖用户显式设置。
    fn content_type(&self) -> Option<&'static str> {
        None
    }
}

/// 强类型序列化入口：T 需序列化为 JSON 对象
///
/// # Errors
///
/// 返回 [`ArtfulError::InvalidParameter`] 当 `T` 未序列化为 JSON 对象；
/// 返回 [`ArtfulError::JsonSerializeError`] 当序列化失败。
pub fn pack_typed<T: Serialize>(
    packer: &dyn Packer,
    data: &T,
    params: &Map<String, Value>,
) -> Result<String> {
    let value = serde_json::to_value(data).map_err(ArtfulError::JsonSerializeError)?;
    let obj = value
        .as_object()
        .ok_or_else(|| ArtfulError::InvalidParameter {
            param: "data".to_string(),
            message: "pack_typed 要求 T 序列化为 JSON 对象".to_string(),
        })?;
    packer.pack(obj, params)
}

/// 强类型反序列化入口
///
/// # Errors
///
/// 返回 [`ArtfulError::JsonDeserializeError`] 当反序列化失败。
pub fn unpack_typed<T: DeserializeOwned>(
    packer: &dyn Packer,
    data: &str,
    params: &Map<String, Value>,
) -> Result<T> {
    let value = packer.unpack(data, params)?;
    serde_json::from_value(value).map_err(|e| ArtfulError::JsonDeserializeError {
        message: format!("cannot deserialize into {}", std::any::type_name::<T>()),
        source: Some(e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[derive(Debug)]
    struct NoContentTypePacker;

    impl Packer for NoContentTypePacker {
        fn pack(&self, _data: &Map<String, Value>, _params: &Map<String, Value>) -> Result<String> {
            Ok(String::new())
        }

        fn unpack(&self, _data: &str, _params: &Map<String, Value>) -> Result<Value> {
            Ok(Value::Null)
        }
    }

    #[test]
    fn default_content_type_is_none() {
        // trait 默认实现不声明 Content-Type
        assert_eq!(NoContentTypePacker.content_type(), None);
    }

    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
    struct User {
        name: String,
        age: String,
    }

    #[test]
    fn pack_typed_serializes_struct_to_json() {
        // happy path：结构体 → JsonPacker → 请求体字符串
        let user = User {
            name: "yansongda".to_string(),
            age: "29".to_string(),
        };

        let s = pack_typed(&crate::packers::JsonPacker, &user, &Map::new()).unwrap();
        let value: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "name": "yansongda", "age": "29" })
        );
    }

    #[test]
    fn pack_typed_rejects_non_object() {
        // 非 Object（数组）→ InvalidParameter
        let data = vec![1i32, 2, 3];

        let err = pack_typed(&crate::packers::JsonPacker, &data, &Map::new()).unwrap_err();
        match err {
            ArtfulError::InvalidParameter { param, message } => {
                assert_eq!(param, "data");
                assert!(message.contains("JSON 对象"));
            }
            other => panic!("expected InvalidParameter, got {other:?}"),
        }
    }

    #[test]
    fn unpack_typed_deserializes_xml() {
        // happy path：XmlPacker + XML 报文 → 结构体（typed 跨 packer）
        let xml = "<xml><name><![CDATA[yansongda]]></name><age>29</age></xml>";

        let user: User = unpack_typed(&crate::packers::XmlPacker, xml, &Map::new()).unwrap();
        assert_eq!(
            user,
            User {
                name: "yansongda".to_string(),
                age: "29".to_string(),
            }
        );
    }

    #[test]
    fn unpack_typed_type_mismatch_reports_type_name() {
        // 类型不匹配 → JsonDeserializeError 且 message 含类型名
        let json = r#"{"name":"yansongda","age":"29"}"#;

        let err: ArtfulError =
            unpack_typed::<u32>(&crate::packers::JsonPacker, json, &Map::new()).unwrap_err();
        match err {
            ArtfulError::JsonDeserializeError { message, source } => {
                assert!(message.contains("u32"), "message: {message}");
                assert!(source.is_some());
            }
            other => panic!("expected JsonDeserializeError, got {other:?}"),
        }
    }
}
