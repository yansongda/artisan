//! JSON 序列化器实现
//!
//! 实现 [`Packer`] trait，提供 JSON 序列化/反序列化功能。

use serde_json::{Map, Value};

use crate::Result;
use crate::packer::Packer;

/// JSON 序列化器
///
/// 使用 [`serde_json`] 实现 [`Packer`] trait，为默认序列化器。
#[derive(Debug)]
pub struct JsonPacker;

impl Packer for JsonPacker {
    /// 将 Map 序列化为 JSON 字符串
    ///
    /// JSON 序列化器忽略 params（无附加序列化开关）。
    ///
    /// # Errors
    ///
    /// 返回错误当序列化失败。
    fn pack(&self, data: &Map<String, Value>, _params: &Map<String, Value>) -> Result<String> {
        serde_json::to_string(data).map_err(Into::into)
    }

    /// 将 JSON 字符串反序列化为 Value
    ///
    /// JSON 序列化器忽略 params（无附加反序列化开关）。
    ///
    /// # Errors
    ///
    /// 返回错误当反序列化失败。
    fn unpack(&self, data: &str, _params: &Map<String, Value>) -> Result<Value> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_packer_pack() {
        let packer = JsonPacker;
        let mut data = Map::new();
        data.insert("key".to_string(), json!("value"));

        let result = packer.pack(&data, &Map::new()).unwrap();
        assert_eq!(result, r#"{"key":"value"}"#);
    }

    #[test]
    fn test_json_packer_pack_key_order_deterministic() {
        // 乱序插入的键：pack 输出由 Map（BTreeMap 后端）天然字典序保证，
        // 直接断言输出字符串（不用 from_str 解析后比较键序，否则恒真无验证力）
        let packer = JsonPacker;
        let data = Map::from_iter([
            ("c".to_string(), json!(3)),
            ("a".to_string(), json!(1)),
            ("b".to_string(), json!(2)),
        ]);

        let packed = packer.pack(&data, &Map::new()).unwrap();
        assert_eq!(packed, r#"{"a":1,"b":2,"c":3}"#);
    }

    #[test]
    fn test_json_packer_pack_empty() {
        let packer = JsonPacker;
        let data = Map::new();

        let result = packer.pack(&data, &Map::new()).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_json_packer_unpack() {
        let packer = JsonPacker;
        let json = r#"{"key":"value"}"#;

        let result = packer.unpack(json, &Map::new()).unwrap();
        assert_eq!(result["key"], json!("value"));
    }

    #[test]
    fn test_json_packer_unpack_invalid() {
        let packer = JsonPacker;
        let invalid_json = "not json";

        let result = packer.unpack(invalid_json, &Map::new());
        assert!(matches!(
            result.unwrap_err(),
            crate::error::ArtfulError::JsonDeserializeError { .. }
        ));
    }

    #[test]
    fn test_json_packer_content_type() {
        assert_eq!(JsonPacker.content_type(), Some("application/json"));
    }
}
