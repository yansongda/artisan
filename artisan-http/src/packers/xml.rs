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
use std::collections::{BTreeMap, HashMap};

use crate::Result;
use crate::error::ArtfulError;
use crate::packer::Packer;

/// XML 序列化器
///
/// pack 产出 `<xml>` 包裹的单层节点（顶层键按字典序升序输出，确定性）：数值为
/// 纯文本，其余标量为 CDATA（空数据输出 `<xml></xml>`）；unpack 基于 quick-xml
/// 事件流，叶子文本一律产出 [`Value::String`]（保真复刻 PHP simplexml →
/// json_encode → json_decode 全程无数字转换），同名兄弟元素转数组、无文本元素
/// 转空对象。单元素取值复刻 PHP `_get_base_node_value`：首个直接内容为文本且
/// **非全空白**时输出全部直接文本的拼接字符串（子元素被丢弃，实测 PHP 8.5
/// `<a>1<b>2</b>3</a>` → `"13"`）；否则输出子元素对象（混合内容的直接文本
/// 被丢弃，实测 `<a> <b>x</b> </a>` → `{"b":"x"}`）；实体引用（含数字字符
/// 引用）解引用后并入文本，XML 1.0 非法字符引用与未定义实体报错（对齐 libxml）。
/// 根元素恒为 JSON 对象（实测 `<xml>foo<a>1</a></xml>` → `{"a":"1"}`，根直接
/// 文本丢弃；单文本根的 PHP `{"0":"foo"}` 怪癖有意不复刻，输出字符串 `"foo"`）。
///
/// 与 PHP 的已知差异：XML 属性被丢弃（PHP 8.5 仅对 JSON 对象形态的节点——根
/// 元素与无文本/空子元素——产出 `@attributes` 键）；解析结果的键序按字母序
/// （serde_json Map 默认 BTreeMap，PHP 保持文档序，JSON 语义上无影响）；
/// 命名空间前缀实测双方一致（均保留原文，如 `ns:a`）；pack 键序为字典序
/// （PHP 保持数组插入序），签名场景请复核拼接顺序。
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
        // 顶层键升序排序后输出（确定性；PHP 保持数组插入序，见模块级文档）
        let mut keys: Vec<&String> = data.keys().collect();
        keys.sort_unstable();
        for key in keys {
            out.push_str(&Self::render_entry(key, &data[key])?);
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
    /// simplexml_load_string 失败后由 wrapXml 抛 InvalidArgumentException，
    /// 此处返回结构化错误）。
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
                    let value = element.finish(stack.is_empty());
                    match stack.last_mut() {
                        Some(parent) => parent.insert_child(name, value),
                        // 根元素完成：其值即 unpack 结果（PHP 根恒为对象，单文本根
                        // 的 {"0": "foo"} 怪癖有意不复刻，见 XmlElement::finish）
                        None => root_value = Some(value),
                    }
                }
                Event::Empty(bs) => {
                    // 自闭合元素 → 该 key 值为空 Object（对齐 PHP SimpleXML→json 怪癖）
                    // 有意差异：属性被丢弃（PHP 8.5 对对象形态节点产出 "@attributes" 键）
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
                        Some(element) => element.append_text(&text),
                        // 根级文本：空白忽略（对齐 PHP 对缩进/换行的容错），
                        // 非空白为非法 XML（如 "not-xml"，PHP 侧 wrapXml 抛
                        // InvalidArgumentException）
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
                        element.append_text(&text);
                    } else if !text.trim().is_empty() {
                        return Err(deserialize_error(
                            "text content outside of root element",
                            None,
                        ));
                    }
                }
                Event::GeneralRef(g) => {
                    // 实体引用：解引用后并入当前元素文本（对齐 PHP simplexml 的实体
                    // 解析语义——quick-xml 只给出引用名，解引用由本侧完成）
                    let decoded = resolve_general_ref(&g)?;
                    match stack.last_mut() {
                        Some(element) => element.append_text(&decoded),
                        None if !decoded.trim().is_empty() => {
                            return Err(deserialize_error(
                                "entity reference outside of root element",
                                None,
                            ));
                        }
                        None => {}
                    }
                }
                Event::Eof => break,
                // Comment / Decl / PI / DocType 忽略（SimpleXML 同样不暴露注释与
                // 处理指令节点，二者 json_encode 结果一致）
                Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            }
        }

        // 元素未闭合：quick-xml 默认 allow_unmatched_ends = false 已在读取时报错，此处兜底
        if !stack.is_empty() {
            return Err(deserialize_error("unclosed element(s) remain", None));
        }
        // 无根元素（如仅空白输入）：PHP simplexml_load_string 返回 false 后
        // 由 wrapXml 抛 InvalidArgumentException
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
    /// 已知有意差异（PHP `is_numeric` 为版本间移动靶，以下 PHP 行为
    /// 实测于 8.5.10）：
    /// - 空白：PHP 8.5 前导/尾随空白均为 true（8.0~8.4 前导空白为 false），
    ///   此处一律不允许；
    /// - `".5"`：PHP true，Rust f64 解析失败为 false；
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
    /// 首个直接内容节点为文本时的空白判定（`None` = 首个内容不是文本）；
    /// 决定 `finish` 是否走"字符串拼接"分支（复刻 PHP `_get_base_node_value`）
    first_text_blank: Option<bool>,
    children: BTreeMap<String, Value>,
}

impl XmlElement {
    fn new(name: String) -> Self {
        Self {
            name,
            text: String::new(),
            first_text_blank: None,
            children: BTreeMap::new(),
        }
    }

    /// 记录直接文本内容；首个直接内容节点为文本时缓存其空白判定
    ///
    /// 空文本不产生内容节点（libxml 空 CDATA 不构成文本节点，无首个文本）
    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.children.is_empty() && self.first_text_blank.is_none() {
            self.first_text_blank = Some(is_blank_text(text));
        }
        self.text.push_str(text);
    }

    /// 挂载子节点；同名兄弟元素第二次出现 → 该 key 转为
    /// [`Value::Array`] 追加（对齐 PHP SimpleXML → json_encode）
    fn insert_child(&mut self, name: String, value: Value) {
        use std::collections::btree_map::Entry;

        match self.children.entry(name) {
            Entry::Occupied(mut entry) => match entry.get_mut() {
                Value::Array(arr) => arr.push(value),
                slot => {
                    let prev = std::mem::take(slot);
                    *slot = Value::Array(vec![prev, value]);
                }
            },
            Entry::Vacant(entry) => {
                entry.insert(value);
            }
        }
    }

    /// 元素结束 → 构建 [`Value`]（复刻 PHP `sxe_get_prop_hash` / `_get_base_node_value`）
    ///
    /// - 根元素（`is_root = true`）：PHP 根恒为 JSON 对象（根直接文本丢弃，
    ///   实测 `<xml>foo<a>1</a></xml>` → `{"a":"1"}`）；有子元素 → Object，
    ///   仅有文本 → String（单文本根的 PHP `{"0": "foo"}` 怪癖有意不复刻），
    ///   否则空 Object
    /// - 非根元素：首个直接内容为文本且**非全空白** → String（全部直接文本拼接、
    ///   子元素丢弃，实测 `<a>1<b>2</b>3</a>` → `"13"`）；否则有子元素 →
    ///   Object（混合内容的直接文本丢弃，实测 `<a> <b>x</b> </a>` →
    ///   `{"b":"x"}`）；否则空 Object
    fn finish(self, is_root: bool) -> Value {
        if is_root {
            return if self.children.is_empty() {
                if self.text.is_empty() {
                    Value::Object(Map::new())
                } else {
                    Value::String(self.text)
                }
            } else {
                Value::Object(self.children.into_iter().collect())
            };
        }

        if self.first_text_blank == Some(false) {
            return Value::String(self.text);
        }

        if !self.children.is_empty() {
            return Value::Object(self.children.into_iter().collect());
        }

        Value::Object(Map::new())
    }
}

/// libxml `xmlIsBlankNode` 语义：非空且全部为空白字符（space/tab/CR/LF）
fn is_blank_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
}

/// XML 元素名（QName）转 String
fn qname_to_string(name: QName<'_>) -> String {
    String::from_utf8_lossy(name.as_ref()).into_owned()
}

/// 解引用实体引用（`&name;`，含数字字符引用）为文本
///
/// 对齐 PHP simplexml 的实体解析语义（libxml）：
/// - 五个 XML 预定义实体（`amp`/`lt`/`gt`/`quot`/`apos`）→ 对应字符
/// - `#N`（十进制）与 `#xH`/`#XH`（十六进制）数字字符引用 → 对应 Unicode 字符
///   （限定 XML 1.0 合法字符集，对齐 libxml `xmlParseCharRef`：`&#0;`、`&#x8;`、
///   `&#xFFFE;` 等非法字符引用报 "invalid xmlChar value"，实测 PHP 8.5 报错）
/// - 其余（未在 DTD 声明的实体名、非法码点）→ 错误
///   （PHP 侧 libxml 报 "Entity not defined"，simplexml_load_string 返回 false，
///   wrapXml 抛 InvalidArgumentException；此处返回 [`ArtfulError::XmlDeserializeError`]）
fn resolve_general_ref(g: &quick_xml::events::BytesRef<'_>) -> Result<String> {
    let name = g.decode().map_err(to_deserialize_error)?;

    let resolved: String = match name.as_ref() {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        numeric => {
            let invalid = || {
                deserialize_error(
                    format!("undefined or invalid entity reference: `{name}`"),
                    None,
                )
            };
            let digits = numeric.strip_prefix('#').ok_or_else(invalid)?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).map_err(|_| invalid())?,
                None => digits.parse::<u32>().map_err(|_| invalid())?,
            };
            // XML 1.0 Char 集合过滤（对齐 libxml：非法字符引用报 invalid xmlChar value）
            char::from_u32(code)
                .filter(|&c| is_xml_char(c))
                .ok_or_else(invalid)?
                .to_string()
        }
    };

    Ok(resolved)
}

/// XML 1.0 合法字符集（`spec`: `Char ::= #x9 | #xA | #xD | [#x20-#xD7FF] |
/// [#xE000-#xFFFD] | [#x10000-#x10FFFF]`）
fn is_xml_char(c: char) -> bool {
    matches!(c, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&c)
        || ('\u{E000}'..='\u{FFFD}').contains(&c)
        || ('\u{10000}'..='\u{10FFFF}').contains(&c)
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
        // 顶层键升序排序后输出（确定性）：age < name
        assert_eq!(
            result,
            "<xml><age>29</age><name><![CDATA[yansongda]]></name></xml>"
        );
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

        // 仅空白输入：无根元素，PHP 侧 wrapXml 抛 InvalidArgumentException → Err
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

        // 复刻 PHP `_get_base_node_value`（下述全部实测 PHP 8.5）：
        // 首直接内容为文本且非空白 → 全部直接文本拼接、子元素丢弃
        let result = packer
            .unpack("<xml><a>1<b>2</b>3</a></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["a"], json!("13"));

        let result = packer
            .unpack("<xml><a>text<b>sub</b></a></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["a"], json!("text"));

        // 首直接内容为空白文本 → 对象分支：直接文本全部丢弃
        let result = packer
            .unpack("<xml><a> <b>x</b> </a></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["a"], json!({"b": "x"}));

        // 首直接内容是子元素 → 对象分支（尾部文本丢弃）
        let result = packer
            .unpack("<xml><a><b>1</b>tail</a></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result["a"], json!({"b": "1"}));
    }

    #[test]
    fn test_xml_packer_unpack_root_mixed_content() {
        let packer = XmlPacker;

        // PHP 根恒为对象：根直接文本丢弃、子元素保留
        // （实测 wrapXml('<xml>foo<a>1</a></xml>') → {'a': '1'}）
        let result = packer
            .unpack("<xml>foo<a>1</a></xml>", &HashMap::new())
            .unwrap();
        assert_eq!(result, json!({"a": "1"}));
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
    fn test_xml_packer_unpack_decodes_entities() {
        let packer = XmlPacker;

        // 预定义实体解引用后并入文本（对齐 PHP simplexml；quick-xml 拆分出的
        // GeneralRef 事件不能丢弃，否则字符静默丢失）
        let result = packer
            .unpack(
                "<xml><a>x&amp;y</a><b>1&lt;2</b><c>&quot;q&quot;</c><d>&apos;</d></xml>",
                &HashMap::new(),
            )
            .unwrap();
        assert_eq!(result["a"], "x&y");
        assert_eq!(result["b"], "1<2");
        assert_eq!(result["c"], "\"q\"");
        assert_eq!(result["d"], "'");
    }

    #[test]
    fn test_xml_packer_unpack_decodes_numeric_char_refs() {
        let packer = XmlPacker;

        // 数字字符引用（十进制/十六进制）解引用（部分网关以此编码中文）
        let result = packer
            .unpack(
                "<xml><a>&#20013;&#25991;</a><b>&#x4E2D;&#x6587;</b></xml>",
                &HashMap::new(),
            )
            .unwrap();
        assert_eq!(result["a"], "中文");
        assert_eq!(result["b"], "中文");
    }

    #[test]
    fn test_xml_packer_unpack_rejects_undefined_entity() {
        let packer = XmlPacker;

        // 未定义实体：libxml 报 Entity not defined，simplexml 返回 false 后
        // wrapXml 抛 InvalidArgumentException → 此处 XmlDeserializeError
        assert!(matches!(
            packer.unpack("<xml><a>&foo;</a></xml>", &HashMap::new()),
            Err(ArtfulError::XmlDeserializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_unpack_rejects_invalid_char_ref() {
        let packer = XmlPacker;

        // 非法数字引用：超码点范围 / 空数字 / 非数字 / XML 1.0 非法字符
        // （&#0;/&#x8;/&#xFFFE; 实测 libxml 报 "invalid xmlChar value"）
        // → XmlDeserializeError
        for input in [
            "<xml><a>&#x110000;</a></xml>",
            "<xml><a>&#;</a></xml>",
            "<xml><a>&#xZZ;</a></xml>",
            "<xml><a>&#0;</a></xml>",
            "<xml><a>&#x8;</a></xml>",
            "<xml><a>&#xFFFE;</a></xml>",
        ] {
            assert!(
                matches!(
                    packer.unpack(input, &HashMap::new()),
                    Err(ArtfulError::XmlDeserializeError { .. })
                ),
                "应拒绝非法字符引用：{input}"
            );
        }
    }

    #[test]
    fn test_xml_packer_content_type() {
        assert_eq!(XmlPacker.content_type(), Some("application/xml"));
    }
}
