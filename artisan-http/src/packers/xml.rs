//! XML 序列化器实现
//!
//! 实现 [`Packer`] trait，提供 XML 序列化/反序列化功能。
//!
//! 行为契约对齐 yansongda/artful 的 `XmlPacker`：pack 对应 PHP
//! `Collection::toXml()`（`<xml>` 包裹 + `is_numeric` 纯文本 / CDATA 分支），
//! unpack 对应 PHP `Arr::wrapXml()`（simplexml_load_string → json_encode →
//! json_decode，复刻其结构怪癖）。

use quick_xml::Reader;
use quick_xml::events::Event;
use quick_xml::name::QName;
use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::Result;
use crate::error::ArtfulError;
use crate::packer::Packer;

/// XML 序列化器
///
/// pack 产出 `<xml>` 包裹的单层节点：数值为纯文本，其余标量为 CDATA（空数据
/// 输出 `<xml></xml>`）；unpack 基于 quick-xml 事件流，叶子文本一律产出
/// [`Value::String`]（保真复刻 PHP simplexml → json_encode → json_decode
/// 全程无数字转换），同名兄弟元素转数组、无文本元素转空对象、混合内容丢弃
/// 直接文本。
#[derive(Debug, Clone, Copy, Default)]
pub struct XmlPacker;

impl Packer for XmlPacker {
    /// 将 HashMap 序列化为 XML 字符串
    ///
    /// XML 序列化器忽略 params（无附加序列化开关）。
    ///
    /// # Errors
    ///
    /// 返回错误当值包含嵌套数组/对象（PHP 产出 `<![CDATA[Array]]>` 垃圾值，
    /// 此处有意差异：显式报错）。
    fn pack(
        &self,
        data: &HashMap<String, Value>,
        _params: &HashMap<String, Value>,
    ) -> Result<String> {
        // 空集合 → "<xml></xml>"（对齐 PHP Collection::toXml；区别于 JsonPacker 空输入的 "{}"）
        if data.is_empty() {
            return Ok("<xml></xml>".to_string());
        }

        let mut out = String::from("<xml>");
        for (key, value) in data {
            out.push_str(&Self::render_entry(key, value)?);
        }
        out.push_str("</xml>");
        Ok(out)
    }

    /// 将 XML 字符串反序列化为 Value
    ///
    /// XML 序列化器忽略 params（无附加反序列化开关）。
    ///
    /// # Errors
    ///
    /// 返回错误当 XML 格式非法（有意差异：PHP 侧对应
    /// simplexml_load_string 失败后抛 TypeError，此处返回结构化错误）。
    fn unpack(&self, data: &str, _params: &HashMap<String, Value>) -> Result<Value> {
        // 对齐 PHP Arr::wrapXml 的 empty() 语义："" 与 "0" 直接返回空对象
        if data.is_empty() || data == "0" {
            return Ok(Value::Object(Map::new()));
        }

        let mut reader = Reader::from_str(data);

        // stack：已打开元素栈；root_value：根元素完成后的值
        // （unpack 结果对齐 PHP 语义：json_encode(simplexml) 编码根元素的子结构，不含根名）
        let mut stack: Vec<XmlElement> = Vec::new();
        let mut root_value: Option<Value> = None;

        loop {
            let event = reader.read_event().map_err(to_deserialize_error)?;

            match event {
                Event::Start(bs) => {
                    // 多个根元素为非法 XML（PHP simplexml_load_string 同样失败）
                    if stack.is_empty() && root_value.is_some() {
                        return Err(deserialize_error("multiple root elements", None));
                    }
                    stack.push(XmlElement::new(qname_to_string(bs.name())));
                }
                Event::End(be) => {
                    let element = stack
                        .pop()
                        .ok_or_else(|| deserialize_error("unmatched end element", None))?;
                    // 防御性检查：quick-xml 默认 check_end_names = true 已在读取时报错，此处兜底
                    let end_name = qname_to_string(be.name());
                    if element.name != end_name {
                        return Err(deserialize_error(
                            format!(
                                "mismatched end element: expected `{}`, got `{}`",
                                element.name, end_name
                            ),
                            None,
                        ));
                    }

                    let name = element.name.clone();
                    let value = element.finish();
                    match stack.last_mut() {
                        Some(parent) => parent.insert_child(name, value),
                        // 根元素完成：其值即 unpack 结果（根元素仅含直接文本时为
                        // Value::String，此边角 PHP 实际产出 {"0": ...}，有意不复刻）
                        None => root_value = Some(value),
                    }
                }
                Event::Empty(bs) => {
                    // 自闭合元素 → 该 key 值为空 Object（对齐 PHP SimpleXML→json 怪癖）
                    // 有意差异：属性被丢弃（PHP 会产出 "@attributes" 键）
                    let name = qname_to_string(bs.name());
                    if let Some(parent) = stack.last_mut() {
                        parent.insert_child(name, Value::Object(Map::new()));
                    } else {
                        if root_value.is_some() {
                            return Err(deserialize_error("multiple root elements", None));
                        }
                        root_value = Some(Value::Object(Map::new()));
                    }
                }
                Event::Text(t) => {
                    let text = t.decode().map_err(to_deserialize_error)?.into_owned();
                    match stack.last_mut() {
                        Some(element) => element.text.push_str(&text),
                        // 根级文本：空白忽略（对齐 PHP 对缩进/换行的容错），
                        // 非空白为非法 XML（如 "not-xml"，PHP 抛 TypeError）
                        None if !text.trim().is_empty() => {
                            return Err(deserialize_error(
                                "text content outside of root element",
                                None,
                            ));
                        }
                        None => {}
                    }
                }
                Event::CData(c) => {
                    let text = c.decode().map_err(to_deserialize_error)?.into_owned();
                    if let Some(element) = stack.last_mut() {
                        element.text.push_str(&text);
                    } else if !text.trim().is_empty() {
                        return Err(deserialize_error(
                            "text content outside of root element",
                            None,
                        ));
                    }
                }
                Event::Eof => break,
                // Comment / Decl / PI / DocType / GeneralRef 忽略
                // （有意差异：PHP simplexml 会为注释/处理指令产出假节点；
                // 实体引用不合并入相邻文本）
                _ => {}
            }
        }

        // 元素未闭合：quick-xml 默认 allow_unmatched_ends = false 已在读取时报错，此处兜底
        if !stack.is_empty() {
            return Err(deserialize_error("unclosed element(s) remain", None));
        }
        // 无根元素（如仅空白输入）：PHP simplexml_load_string 返回 false 后抛 TypeError
        root_value.ok_or_else(|| deserialize_error("no root element", None))
    }

    /// XML 请求体的 Content-Type
    fn content_type(&self) -> Option<&'static str> {
        Some("application/xml")
    }
}

impl XmlPacker {
    /// 渲染单个键值对为 XML 片段 `<key>...</key>`
    fn render_entry(key: &str, value: &Value) -> Result<String> {
        // 有意差异：PHP 对数组/对象产出 "<![CDATA[Array]]>" 垃圾值，此处显式报错
        if matches!(value, Value::Array(_) | Value::Object(_)) {
            return Err(ArtfulError::XmlSerializeError {
                message: "XmlPacker 仅支持一维标量".to_string(),
                source: None,
            });
        }

        let inner = if Self::is_numeric(value) {
            // 数值 → 纯文本；数值字符串原样输出（对齐 PHP 字符串拼接）
            match value {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                // is_numeric 仅对 Number/String 为 true，其余类型不可达
                Value::Bool(_) | Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
            }
        } else {
            // CDATA 分支字符串化对齐 PHP 隐式转换：true→"1"、false→""、null→""
            let text = match value {
                Value::String(s) => s.as_str(),
                Value::Bool(true) => "1",
                Value::Bool(false) | Value::Null => "",
                // Array/Object 已提前报错，Number 已走 is_numeric 分支，均不可达
                Value::Number(_) | Value::Array(_) | Value::Object(_) => "",
            };
            format!("<![CDATA[{text}]]>")
        };

        // 键不做 XML 转义（对齐 PHP 现状）
        Ok(format!("<{key}>{inner}</{key}>"))
    }

    /// 判定值是否符合 PHP `is_numeric($val)` 语义（纯文本/CDATA 分支的选择依据）
    ///
    /// 字符串近似判定：i64/u64/f64 解析成功即视为数值（覆盖 "29"/"1.5"/"1e5"）。
    ///
    /// 已知有意差异：
    /// - 前导/尾随空白：PHP 8 `is_numeric(" 29 ")` 为 true，此处解析失败为 false；
    /// - `"inf"`/`"NaN"`：Rust f64 解析成功为 true，PHP 为 false；
    /// - 整值浮点：serde_json `29.0` 序列化为 `"29.0"`，而 PHP `(float)29.0` 为
    ///   `"29"`（precision 截断）。
    fn is_numeric(value: &Value) -> bool {
        match value {
            Value::Number(_) => true,
            Value::String(s) => {
                s.parse::<i64>().is_ok() || s.parse::<u64>().is_ok() || s.parse::<f64>().is_ok()
            }
            _ => false,
        }
    }
}

/// quick-xml 栈上元素的构建状态
struct XmlElement {
    name: String,
    text: String,
    children: Vec<(String, Value)>,
}

impl XmlElement {
    fn new(name: String) -> Self {
        Self {
            name,
            text: String::new(),
            children: Vec::new(),
        }
    }

    /// 挂载子节点；同名兄弟元素第二次出现 → 该 key 转为
    /// [`Value::Array`] 追加（对齐 PHP SimpleXML → json_encode）
    fn insert_child(&mut self, name: String, value: Value) {
        if let Some(existing) = self.children.iter_mut().find(|(k, _)| *k == name) {
            match &mut existing.1 {
                Value::Array(arr) => arr.push(value),
                slot => {
                    let prev = std::mem::take(slot);
                    *slot = Value::Array(vec![prev, value]);
                }
            }
        } else {
            self.children.push((name, value));
        }
    }

    /// 元素结束 → 构建 [`Value`]：
    /// - 有子元素 → [`Value::Object`]（混合内容：直接文本被丢弃，对齐 PHP）；
    /// - 仅直接文本 → [`Value::String`]（叶子文本保真复刻，PHP 全程无数字转换）；
    /// - 无文本无子元素（含自闭合）→ 空 [`Value::Object`]（对齐 PHP SimpleXML→json 怪癖）。
    fn finish(self) -> Value {
        if !self.children.is_empty() {
            let mut map = Map::new();
            for (key, value) in self.children {
                // insert_child 已保证 children 中无同名键（第二次出现已转 Array）
                map.insert(key, value);
            }
            Value::Object(map)
        } else if !self.text.is_empty() {
            Value::String(self.text)
        } else {
            Value::Object(Map::new())
        }
    }
}

/// XML 元素名（QName）转 String
fn qname_to_string(name: QName<'_>) -> String {
    String::from_utf8_lossy(name.as_ref()).into_owned()
}

/// quick-xml 错误 → XmlDeserializeError
fn to_deserialize_error(e: impl std::error::Error + Send + Sync + 'static) -> ArtfulError {
    ArtfulError::XmlDeserializeError {
        message: e.to_string(),
        source: Some(Box::new(e)),
    }
}

/// 无底层错误源时构建 XmlDeserializeError
fn deserialize_error(
    message: impl Into<String>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
) -> ArtfulError {
    ArtfulError::XmlDeserializeError {
        message: message.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_xml_packer_pack() {
        let packer = XmlPacker;
        let data = HashMap::from([
            ("name".to_string(), json!("yansongda")),
            ("age".to_string(), json!(29)),
        ]);

        let result = packer.pack(&data, &HashMap::new()).unwrap();
        // HashMap 无序：仅断言首尾标签与各项子串，顺序无关
        assert!(result.starts_with("<xml>"));
        assert!(result.ends_with("</xml>"));
        assert!(result.contains("<name><![CDATA[yansongda]]></name>"));
        assert!(result.contains("<age>29</age>"));
    }

    #[test]
    fn test_xml_packer_pack_empty() {
        let packer = XmlPacker;
        let data = HashMap::new();

        let result = packer.pack(&data, &HashMap::new()).unwrap();
        assert_eq!(result, "<xml></xml>");
    }

    #[test]
    fn test_xml_packer_pack_nested_error() {
        let packer = XmlPacker;

        // 嵌套对象/数组：PHP 产出 "<![CDATA[Array]]>" 垃圾值，此处显式报错
        let data = HashMap::from([("obj".to_string(), json!({"k": "v"}))]);
        let err = packer.pack(&data, &HashMap::new()).unwrap_err();
        assert!(matches!(err, ArtfulError::XmlSerializeError { .. }));

        let data = HashMap::from([("arr".to_string(), json!(["a"]))]);
        assert!(matches!(
            packer.pack(&data, &HashMap::new()),
            Err(ArtfulError::XmlSerializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_pack_numeric_string() {
        // is_numeric 判定："29" 符合 PHP is_numeric("29") === true → 纯文本分支
        let packer = XmlPacker;
        let data = HashMap::from([("age".to_string(), json!("29"))]);

        let result = packer.pack(&data, &HashMap::new()).unwrap();
        assert_eq!(result, "<xml><age>29</age></xml>");
    }

    // 注释性差异（不设断言，依 todo 说明）：pack `{"f": 29.0}` 时 serde_json
    // 整值浮点输出 "<f>29.0</f>"，而 PHP (float)29.0 为 "<f>29</f>"
    // （precision 截断）。

    #[test]
    fn test_xml_packer_unpack() {
        let packer = XmlPacker;

        let result = packer
            .unpack(
                "<xml><name><![CDATA[yansongda]]></name><age>29</age></xml>",
                &HashMap::new(),
            )
            .unwrap();
        // age 锁定为 String "29"：PHP simplexml → json_encode → json_decode
        // 全程无数字转换；契约测试名义期望数字 29 系 PHPUnit 宽松比较
        // （'29' == 29）通过，实际行为为字符串
        assert_eq!(result["name"], json!("yansongda"));
        assert_eq!(result["age"], json!("29"));
    }

    #[test]
    fn test_xml_packer_unpack_repeated_tags_to_array() {
        let packer = XmlPacker;

        // 同名兄弟元素第二次出现 → 转 Array 追加（对齐 PHP SimpleXML → json_encode）
        let result = packer
            .unpack("<xml><tags><t>a</t><t>b</t></tags></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["tags"]["t"], json!(["a", "b"]));
    }

    #[test]
    fn test_xml_packer_unpack_nested() {
        let packer = XmlPacker;

        let result = packer
            .unpack("<xml><deep><k>v</k></deep></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["deep"]["k"], json!("v"));
    }

    #[test]
    fn test_xml_packer_unpack_empty_variants() {
        let packer = XmlPacker;

        // 根下无子元素 → 空 Object（对齐 PHP SimpleXML → json）
        assert_eq!(
            packer.unpack("<xml></xml>", &HashMap::new()).unwrap(),
            json!({})
        );
        // 对齐 PHP Arr::wrapXml 的 empty() 语义："" 与 "0" 直接返回空对象
        assert_eq!(packer.unpack("", &HashMap::new()).unwrap(), json!({}));
        assert_eq!(packer.unpack("0", &HashMap::new()).unwrap(), json!({}));
    }

    #[test]
    fn test_xml_packer_unpack_blank_error() {
        let packer = XmlPacker;

        // 仅空白输入：无根元素，PHP simplexml_load_string 失败后抛 TypeError → Err
        assert!(matches!(
            packer.unpack(" ", &HashMap::new()),
            Err(ArtfulError::XmlDeserializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_unpack_empty_elements() {
        let packer = XmlPacker;

        // 无文本元素与自闭合元素 → 该 key 值为空 Object（对齐 PHP SimpleXML→json 怪癖）
        let result = packer
            .unpack("<xml><empty1></empty1><empty2/></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["empty1"], json!({}));
        assert_eq!(result["empty2"], json!({}));
    }

    #[test]
    fn test_xml_packer_unpack_mixed_content() {
        let packer = XmlPacker;

        // 混合内容：元素同时含直接文本与子元素时丢弃直接文本（对齐 PHP）
        let result = packer
            .unpack("<xml><a>text<b>sub</b></a></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["a"], json!({"b": "sub"}));
    }

    #[test]
    fn test_xml_packer_unpack_invalid() {
        let packer = XmlPacker;

        let result = packer.unpack("not-xml", &HashMap::new());
        assert!(matches!(
            result,
            Err(ArtfulError::XmlDeserializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_content_type() {
        assert_eq!(XmlPacker.content_type(), Some("application/xml"));
    }
}
