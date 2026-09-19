//! XML 序列化器实现
//!
//! 实现 [`Packer`] trait，提供 XML 序列化/反序列化功能。
//!
//! pack 产出 `<xml>` 包裹的单层报文（数值纯文本、其余标量 CDATA）；
//! unpack 把 XML 解析为 [`serde_json::Value`]，遵循网关报文的常见约定：
//! 叶子文本一律字符串、同名兄弟元素转数组、无文本元素转空对象。

use quick_xml::Reader;
use quick_xml::events::Event;
use quick_xml::name::QName;
use serde_json::{Map, Value};

use crate::Result;
use crate::error::ArtfulError;
use crate::packer::Packer;

/// XML 序列化器
///
/// # pack 行为
///
/// 产出 `<xml>` 包裹的单层节点：数值为纯文本，其余标量为 CDATA；
/// 空数据输出 `<xml></xml>`；顶层键按字典序输出（Map 天然有序，
/// 签名场景可复现）。
///
/// # unpack 行为
///
/// 基于 quick-xml 事件流解析为 [`Value`]，规则：
///
/// - 叶子文本一律产出 [`Value::String`]，不做数字转换（`"29"` 保持
///   字符串；字段类型由调用方自行转换）
/// - 同名兄弟元素第二次出现时该键转为 [`Value::Array`] 追加
/// - 无文本元素与自闭合元素 → 空对象 `{}`
/// - 单元素取值：首个直接内容为文本且**非全空白** → 全部直接文本的拼接
///   字符串（子元素被丢弃，`<a>1<b>2</b>3</a>` → `"13"`）；否则有子元素
///   → 子元素对象（混合内容的直接文本被丢弃，`<a> <b>x</b> </a>` →
///   `{"b":"x"}`）；否则空对象
/// - 根元素恒为 JSON 对象（根直接文本丢弃，`<xml>foo<a>1</a></xml>` →
///   `{"a":"1"}`）；但根下无子元素仅有文本时输出字符串
/// - 实体引用（含数字字符引用）解引用后并入文本；未定义实体与 XML 1.0
///   非法字符引用报错
/// - XML 属性被丢弃；命名空间前缀保留原文（如 `ns:a`）；解析结果的键序
///   按字母序（Map 天然有序，JSON 语义上无影响）
#[derive(Debug, Clone, Copy, Default)]
pub struct XmlPacker;

impl Packer for XmlPacker {
    /// 将 Map 序列化为 XML 字符串
    ///
    /// XML 序列化器忽略 params（无附加序列化开关）。
    ///
    /// # Errors
    ///
    /// 返回错误当值包含嵌套数组/对象——网关报文约定为一维键值对，
    /// 嵌套结构无法表达，显式报错优于产出错误报文。
    fn pack(&self, data: &Map<String, Value>, _params: &Map<String, Value>) -> Result<String> {
        // 空集合 → "<xml></xml>"（区别于 JsonPacker 空输入的 "{}"）
        if data.is_empty() {
            return Ok("<xml></xml>".to_string());
        }

        let mut out = String::from("<xml>");
        // Map 天然有序，直接按字典序遍历
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
    /// 返回错误当 XML 格式非法（无法定位根元素、元素未闭合、
    /// 非法实体引用等），返回 [`ArtfulError::XmlDeserializeError`]。
    fn unpack(&self, data: &str, _params: &Map<String, Value>) -> Result<Value> {
        // 空输入约定："" 与 "0" 视为空报文，直接返回空对象
        if data.is_empty() || data == "0" {
            return Ok(Value::Object(Map::new()));
        }

        let mut reader = Reader::from_str(data);

        // stack：已打开元素栈；root_value：根元素完成后的值
        // （unpack 结果是根元素的“内容”，不含根元素名本身）
        let mut stack: Vec<XmlElement> = Vec::new();
        let mut root_value: Option<Value> = None;

        loop {
            let event = reader.read_event().map_err(to_deserialize_error)?;

            match event {
                Event::Start(bs) => {
                    // 多个根元素为非法 XML
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
                        // 根元素完成：其值即 unpack 结果（取值规则见 XmlElement::finish）
                        None => root_value = Some(value),
                    }
                }
                Event::Empty(bs) => {
                    // 自闭合元素 → 该 key 值为空 Object；属性一律丢弃
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
                        // 根级文本：空白忽略（缩进/换行容错），
                        // 非空白为非法 XML（如 "not-xml"）
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
                    // 实体引用：解引用后并入当前元素文本
                    // （quick-xml 只给出引用名，解引用由本侧完成）
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
                // Comment / Decl / PI / DocType 不参与报文数据，忽略
                Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            }
        }

        // 元素未闭合：quick-xml 默认 allow_unmatched_ends = false 已在读取时报错，此处兜底
        if !stack.is_empty() {
            return Err(deserialize_error("unclosed element(s) remain", None));
        }
        // 无根元素（如仅空白输入）
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
        // 嵌套结构无法表达为单层报文，显式报错
        if matches!(value, Value::Array(_) | Value::Object(_)) {
            return Err(ArtfulError::XmlSerializeError {
                message: "XmlPacker 仅支持一维标量".to_string(),
                source: None,
            });
        }

        let inner = if Self::is_numeric(value) {
            // 数值 → 纯文本；数值字符串原样输出
            match value {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                // is_numeric 仅对 Number/String 为 true，其余类型不可达
                Value::Bool(_) | Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
            }
        } else {
            // CDATA 分支字符串化：true→"1"、false→""、null→""
            let text = match value {
                Value::String(s) => s.as_str(),
                Value::Bool(true) => "1",
                Value::Bool(false) | Value::Null => "",
                // Array/Object 已提前报错，Number 已走 is_numeric 分支，均不可达
                Value::Number(_) | Value::Array(_) | Value::Object(_) => "",
            };
            format!("<![CDATA[{text}]]>")
        };

        // 键约定为合法 XML 名称，不做转义
        Ok(format!("<{key}>{inner}</{key}>"))
    }

    /// 判定值是否按纯文本分支输出（否则走 CDATA 分支）
    ///
    /// 数值与“可解析为数值的字符串”走纯文本（`"29"`/`"1.5"`/`"1e5"`）。
    /// 字符串判定标准：i64/u64/f64 解析成功即视为数值，因此：
    /// - 前导/尾随空白不允许（`" 29"` 走 CDATA）
    /// - `".5"` 走 CDATA（Rust f64 解析失败）
    /// - `"inf"`/`"NaN"` 走纯文本（Rust f64 解析成功）
    /// - serde_json 的整值浮点 `29.0` 输出 `"29.0"` 而非 `"29"`
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
    /// 决定 `finish` 是否走“字符串拼接”分支
    first_text_blank: Option<bool>,
    children: Map<String, Value>,
}

impl XmlElement {
    fn new(name: String) -> Self {
        Self {
            name,
            text: String::new(),
            first_text_blank: None,
            children: Map::new(),
        }
    }

    /// 记录直接文本内容；首个直接内容节点为文本时缓存其空白判定
    ///
    /// 空文本不构成内容节点
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
    /// [`Value::Array`] 追加
    fn insert_child(&mut self, name: String, value: Value) {
        use serde_json::map::Entry;

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

    /// 元素结束 → 构建 [`Value`]
    ///
    /// - 根元素（`is_root = true`）：有子元素 → Object（根直接文本丢弃，
    ///   `<xml>foo<a>1</a></xml>` → `{"a":"1"}`）；仅有文本 → String
    ///   （`<xml>foo</xml>` → `"foo"`）；否则空 Object
    /// - 非根元素：首个直接内容为文本且**非全空白** → String（全部直接文本拼接、
    ///   子元素丢弃，`<a>1<b>2</b>3</a>` → `"13"`）；否则有子元素 →
    ///   Object（混合内容的直接文本丢弃，`<a> <b>x</b> </a>` →
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

/// 空白文本判定：非空且全部为空白字符（space/tab/CR/LF）
fn is_blank_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
}

/// XML 元素名（QName）转 String
fn qname_to_string(name: QName<'_>) -> String {
    String::from_utf8_lossy(name.as_ref()).into_owned()
}

/// 解引用实体引用（`&name;`，含数字字符引用）为文本
///
/// - 五个 XML 预定义实体（`amp`/`lt`/`gt`/`quot`/`apos`）→ 对应字符
/// - `#N`（十进制）与 `#xH`/`#XH`（十六进制）数字字符引用 → 对应 Unicode 字符
///   （限定 XML 1.0 合法字符集：`&#0;`、`&#x8;`、`&#xFFFE;` 等非法字符引用报错）
/// - 其余（未在 DTD 声明的实体名、非法码点）→
///   [`ArtfulError::XmlDeserializeError`]
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
            // XML 1.0 Char 集合过滤（非法字符引用报错）
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
        let data = Map::from_iter([
            ("name".to_string(), json!("yansongda")),
            ("age".to_string(), json!(29)),
        ]);

        let result = packer.pack(&data, &Map::new()).unwrap();
        // 顶层键按字典序输出（Map 天然有序）：age < name
        assert_eq!(
            result,
            "<xml><age>29</age><name><![CDATA[yansongda]]></name></xml>"
        );
    }

    #[test]
    fn test_xml_packer_pack_empty() {
        let packer = XmlPacker;
        let data = Map::new();

        let result = packer.pack(&data, &Map::new()).unwrap();
        assert_eq!(result, "<xml></xml>");
    }

    #[test]
    fn test_xml_packer_pack_nested_error() {
        let packer = XmlPacker;

        // 嵌套对象/数组：无法表达为单层报文，显式报错
        let data = Map::from_iter([("obj".to_string(), json!({"k": "v"}))]);
        let err = packer.pack(&data, &Map::new()).unwrap_err();
        assert!(matches!(err, ArtfulError::XmlSerializeError { .. }));

        let data = Map::from_iter([("arr".to_string(), json!(["a"]))]);
        assert!(matches!(
            packer.pack(&data, &Map::new()),
            Err(ArtfulError::XmlSerializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_pack_numeric_string() {
        // 数值字符串 "29" 判定为数值 → 纯文本分支
        let packer = XmlPacker;
        let data = Map::from_iter([("age".to_string(), json!("29"))]);

        let result = packer.pack(&data, &Map::new()).unwrap();
        assert_eq!(result, "<xml><age>29</age></xml>");
    }

    // 注释性说明（不设断言）：pack `{"f": 29.0}` 输出 "<f>29.0</f>"，
    // 整值浮点不截断小数位。

    #[test]
    fn test_xml_packer_unpack() {
        let packer = XmlPacker;

        let result = packer
            .unpack(
                "<xml><name><![CDATA[yansongda]]></name><age>29</age></xml>",
                &Map::new(),
            )
            .unwrap();
        // age 锁定为 String "29"：叶子文本一律字符串，不做数字转换
        assert_eq!(result["name"], json!("yansongda"));
        assert_eq!(result["age"], json!("29"));
    }

    #[test]
    fn test_xml_packer_unpack_repeated_tags_to_array() {
        let packer = XmlPacker;

        // 同名兄弟元素第二次出现 → 转 Array 追加
        let result = packer
            .unpack("<xml><tags><t>a</t><t>b</t></tags></xml>", &Map::new())
            .unwrap();
        assert_eq!(result["tags"]["t"], json!(["a", "b"]));
    }

    #[test]
    fn test_xml_packer_unpack_nested() {
        let packer = XmlPacker;

        let result = packer
            .unpack("<xml><deep><k>v</k></deep></xml>", &Map::new())
            .unwrap();
        assert_eq!(result["deep"]["k"], json!("v"));
    }

    #[test]
    fn test_xml_packer_unpack_empty_variants() {
        let packer = XmlPacker;

        // 根下无子元素 → 空 Object
        assert_eq!(
            packer.unpack("<xml></xml>", &Map::new()).unwrap(),
            json!({})
        );
        // 空输入约定："" 与 "0" 直接返回空对象
        assert_eq!(packer.unpack("", &Map::new()).unwrap(), json!({}));
        assert_eq!(packer.unpack("0", &Map::new()).unwrap(), json!({}));
    }

    #[test]
    fn test_xml_packer_unpack_blank_error() {
        let packer = XmlPacker;

        // 仅空白输入：无根元素 → Err
        assert!(matches!(
            packer.unpack(" ", &Map::new()),
            Err(ArtfulError::XmlDeserializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_unpack_empty_elements() {
        let packer = XmlPacker;

        // 无文本元素与自闭合元素 → 该 key 值为空 Object
        let result = packer
            .unpack("<xml><empty1></empty1><empty2/></xml>", &Map::new())
            .unwrap();
        assert_eq!(result["empty1"], json!({}));
        assert_eq!(result["empty2"], json!({}));
    }

    #[test]
    fn test_xml_packer_unpack_mixed_content() {
        let packer = XmlPacker;

        // 首直接内容为文本且非空白 → 全部直接文本拼接、子元素丢弃
        let result = packer
            .unpack("<xml><a>1<b>2</b>3</a></xml>", &Map::new())
            .unwrap();
        assert_eq!(result["a"], json!("13"));

        let result = packer
            .unpack("<xml><a>text<b>sub</b></a></xml>", &Map::new())
            .unwrap();
        assert_eq!(result["a"], json!("text"));

        // 首直接内容为空白文本 → 对象分支：直接文本全部丢弃
        let result = packer
            .unpack("<xml><a> <b>x</b> </a></xml>", &Map::new())
            .unwrap();
        assert_eq!(result["a"], json!({"b": "x"}));

        // 首直接内容是子元素 → 对象分支（尾部文本丢弃）
        let result = packer
            .unpack("<xml><a><b>1</b>tail</a></xml>", &Map::new())
            .unwrap();
        assert_eq!(result["a"], json!({"b": "1"}));
    }

    #[test]
    fn test_xml_packer_unpack_root_mixed_content() {
        let packer = XmlPacker;

        // 根恒为对象：根直接文本丢弃、子元素保留
        // （<xml>foo<a>1</a></xml> → {"a":"1"}）
        let result = packer
            .unpack("<xml>foo<a>1</a></xml>", &Map::new())
            .unwrap();
        assert_eq!(result, json!({"a": "1"}));
    }

    #[test]
    fn test_xml_packer_unpack_invalid() {
        let packer = XmlPacker;

        let result = packer.unpack("not-xml", &Map::new());
        assert!(matches!(
            result,
            Err(ArtfulError::XmlDeserializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_unpack_decodes_entities() {
        let packer = XmlPacker;

        // 预定义实体解引用后并入文本（quick-xml 拆分出的 GeneralRef 事件
        // 不能丢弃，否则字符静默丢失）
        let result = packer
            .unpack(
                "<xml><a>x&amp;y</a><b>1&lt;2</b><c>&quot;q&quot;</c><d>&apos;</d></xml>",
                &Map::new(),
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
                &Map::new(),
            )
            .unwrap();
        assert_eq!(result["a"], "中文");
        assert_eq!(result["b"], "中文");
    }

    #[test]
    fn test_xml_packer_unpack_rejects_undefined_entity() {
        let packer = XmlPacker;

        // 未定义实体 → XmlDeserializeError
        assert!(matches!(
            packer.unpack("<xml><a>&foo;</a></xml>", &Map::new()),
            Err(ArtfulError::XmlDeserializeError { .. })
        ));
    }

    #[test]
    fn test_xml_packer_unpack_rejects_invalid_char_ref() {
        let packer = XmlPacker;

        // 非法数字引用：超码点范围 / 空数字 / 非数字 / XML 1.0 非法字符
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
                    packer.unpack(input, &Map::new()),
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
