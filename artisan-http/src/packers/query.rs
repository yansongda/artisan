//! Query 序列化器实现
//!
//! 实现 [`Packer`] trait，按 PHP `http_build_query`（`PHP_QUERY_RFC1738`）/
//! `parse_str` 语义编解码 `application/x-www-form-urlencoded` 表单数据，
//! 对齐 [yansongda/artful](https://github.com/yansongda/artful) 的
//! `QueryPacker`（`Collection::query()` / `Arr::wrapQuery()`）。
//!
//! # pack 编码语义（RFC1738）
//!
//! - 保留字符仅 `A-Za-z0-9-_.`；空格转 `+`；其余字节转 `%XX`（大写十六进制）；键与值均编码
//! - `String` 原文编码；`Number` 转数字字符串；`Bool(true)` → `"1"`、`Bool(false)` →
//!   `""`；`Null` → `""`（对齐 PHP `(string)` 强转）
//! - `Array`/`Object` 递归展开为 `k[sub]` 语法（Object 用键名、Array 用下标 `a[0]`），
//!   空容器跳过（不产出任何键值对）；递归深度不限，对齐 PHP
//! - 多项以 `&` 连接为 `k=v`；顶层输入为 `HashMap`，键序不保证，测试断言需顺序无关
//!
//! # unpack 解析语义
//!
//! 默认模式对齐 PHP `parse_str`：
//!
//! - 按 `&` 切段（空段跳过），每段取首个 `=` 分出 key/value，无 `=` 段 value 为 `""`
//! - key/value 先 URL 解码（`+`→空格、`%XX`→字节，非法 `%` 序列原样保留字节）；
//!   key 解码后 `.` 与空格替换为 `_`（PHP parse_str quirk）
//! - 解析 `[...]` 后缀：`k[sub]` → 嵌套对象；`k[]` / `k[0]` 等纯数值下标 →
//!   数组追加；嵌套解析支持一层，更深层级按一层语义近似
//! - 所有值均为 `Value::String`（PHP `parse_str` 不做类型推断）
//!
//! raw 模式（`params` 中 `_unpack_raw` 为 truthy）不做任何解码：按 `&` 切段、
//! 首个 `=` 分出 key/value 后原样保留为 `Value::String`。
//!
//! # raw 模式动机
//!
//! 银联等网关返回的报文中，`signPubKeyCert` 证书串包含 `\r\n`、`+`、`/`：
//! 默认模式的 `+`→空格 与 `%XX` 解码会破坏证书原文，导致
//! `openssl_pkey_get_public` 无法验签；raw 模式保证证书逐字符无损。
//!
//! # `_unpack_raw` truthy 判定
//!
//! 对齐 PHP truthy / `!empty()`（含容器类型）：非空 `Array`/`Object` 为 truthy、
//! 空容器为 falsy；`Bool(true)`、非 `0` 数字、非 `""` 且非 `"0"` 的字符串为
//! truthy；`Bool(false)`、`Null`、`0` 数字、`"0"`/`""` 字符串为 falsy。

use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::Result;
use crate::packer::Packer;

/// Query 序列化器
///
/// 按 PHP `http_build_query`（`PHP_QUERY_RFC1738`）/ `parse_str` 语义实现
/// [`Packer`] trait，处理 `application/x-www-form-urlencoded` 表单数据。
/// 详见模块级文档。
#[derive(Debug, Clone, Copy, Default)]
pub struct QueryPacker;

impl Packer for QueryPacker {
    /// 将 HashMap 按 RFC1738 编码为表单字符串
    ///
    /// Query 序列化器忽略 params（pack 无附加开关）。
    ///
    /// # Errors
    ///
    /// 本实现不会产生错误，恒返回 `Ok`。
    fn pack(
        &self,
        data: &HashMap<String, Value>,
        _params: &HashMap<String, Value>,
    ) -> Result<String> {
        let mut parts: Vec<String> = Vec::new();
        for (key, value) in data {
            pack_entry(key, value, &mut parts);
        }

        Ok(parts.join("&"))
    }

    /// 将表单字符串解析为 [`Value::Object`]
    ///
    /// `params` 中 `_unpack_raw` 为 truthy 时走 raw 模式（不解码），
    /// 否则按 PHP `parse_str` 语义解析（含 `.`/空格 → `_` quirk 与 `[...]` 嵌套）。
    ///
    /// # Errors
    ///
    /// 本实现不会产生错误，恒返回 `Ok`。
    fn unpack(&self, data: &str, params: &HashMap<String, Value>) -> Result<Value> {
        let raw = params.get("_unpack_raw").is_some_and(is_truthy);

        if raw {
            Ok(unpack_raw(data))
        } else {
            Ok(unpack_parse_str(data))
        }
    }

    /// 表单请求体的 Content-Type
    fn content_type(&self) -> Option<&'static str> {
        Some("application/x-www-form-urlencoded")
    }
}

/// 递归编码一个键值对（对齐 PHP `http_build_query` 展开逻辑）
///
/// `prefix` 为未编码的原始键路径（顶层为键名，嵌套为 `a[b]` / `a[0]` 形式）；
/// 空 `Array`/`Object` 不产出任何键值对（对齐 PHP 跳过空容器）。
fn pack_entry(prefix: &str, value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                pack_entry(&format!("{prefix}[{index}]"), item, parts);
            }
        }
        Value::Object(obj) => {
            for (key, item) in obj {
                pack_entry(&format!("{prefix}[{key}]"), item, parts);
            }
        }
        scalar => parts.push(format!(
            "{}={}",
            percent_encode(prefix),
            percent_encode(&scalar_to_string(scalar))
        )),
    }
}

/// 标量值的 PHP `(string)` 强转语义
///
/// `true` → `"1"`、`false` → `""`、`Null` → `""`、`Number` → 数字字符串；
/// 容器不会到达此处（`pack_entry` 中已递归展开，空容器不产出键值对）。
fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "1".to_string(),
        Value::Bool(false) | Value::Null => String::new(),
        // 不可达：非空容器已在 pack_entry 递归展开，空容器不产出键值对
        Value::Array(_) | Value::Object(_) => String::new(),
    }
}

/// RFC1738（application/x-www-form-urlencoded）百分号编码
///
/// 保留字符仅 `A-Za-z0-9-_.`；空格转 `+`；其余字节转 `%XX`（大写十六进制）。
fn percent_encode(s: &str) -> String {
    const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

    let mut out = String::with_capacity(s.len());
    for &byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(HEX_UPPER[usize::from(byte >> 4)] as char);
                out.push(HEX_UPPER[usize::from(byte & 0x0F)] as char);
            }
        }
    }
    out
}

/// RFC1738 百分号解码：`+` 转空格、`%XX` 转对应字节；非法 `%` 序列按原样保留字节
///
/// 返回字节序列，由调用方经 `String::from_utf8_lossy` 转为字符串（对齐 PHP 字节串语义）。
fn percent_decode(s: &str) -> Vec<u8> {
    fn hex_val(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push((hi << 4) | lo);
                    i += 3;
                }
                // 非法 % 序列：`%` 原样保留，从下一字节继续
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}

/// 默认模式：按 PHP `parse_str` 语义解析
///
/// - 按 `&` 切段，空段跳过（对齐 PHP 分段循环跳过空段）；每段取首个 `=` 分出
///   key/value，无 `=` 段 value 为 `""`
/// - key/value 先 URL 解码（`+`→空格、`%XX`→字节）；key 解码后 `.` 与空格替换为
///   `_`（PHP parse_str quirk；作用于整个键，含 `[]` 内内容——PHP 仅替换顶层名，
///   此处按一层近似）
/// - 解析 `[...]` 后缀：`k[sub]` → 嵌套对象；`k[]` / `k[0]` 等纯数值下标 →
///   数组追加（忽略实际下标）；嵌套仅支持一层，更深层级按一层语义近似
fn unpack_parse_str(data: &str) -> Value {
    let mut root = Map::new();

    for segment in data.split('&') {
        if segment.is_empty() {
            continue;
        }

        let (raw_key, raw_value) = match segment.find('=') {
            Some(pos) => (&segment[..pos], &segment[pos + 1..]),
            None => (segment, ""),
        };

        let key = String::from_utf8_lossy(&percent_decode(raw_key)).replace(['.', ' '], "_");
        let decoded_value = percent_decode(raw_value);
        let value = String::from_utf8_lossy(&decoded_value);

        insert_bracketed(&mut root, &key, Value::String(value.into_owned()));
    }

    Value::Object(root)
}

/// raw 模式：不做任何解码，逐段保留原文（对齐 PHP `Arr::wrapQuery($query, true)`）
///
/// - 整串为空或不含 `=` 时返回空对象
/// - 按 `&` 切段，每段取首个 `=`：有 `=` → key/value 原样保留；无 `=` 段 →
///   推断：key 为 `""`、value 为该段去掉第一个字符（基于 PHP
///   `substr($item, 0, false)` / `substr($item, false + 1)` 的强转语义推导，
///   本机无 PHP 未经实测；Rust 按字符边界切除首字符，PHP 按字节，
///   多字节首字符场景存在差异）
fn unpack_raw(data: &str) -> Value {
    let mut root = Map::new();

    if data.is_empty() || !data.contains('=') {
        return Value::Object(root);
    }

    for segment in data.split('&') {
        match segment.find('=') {
            Some(pos) => {
                root.insert(
                    segment[..pos].to_string(),
                    Value::String(segment[pos + 1..].to_string()),
                );
            }
            None => {
                // 推断：见函数文档；无 `=` 段 key 为 `""`、value 去掉第一个字符
                let value = match segment.chars().next() {
                    Some(first) => &segment[first.len_utf8()..],
                    None => "",
                };
                root.insert(String::new(), Value::String(value.to_string()));
            }
        }
    }

    Value::Object(root)
}

/// 解析 `[...]` 后缀并写入容器（一层近似）
///
/// `k[sub]` → 嵌套对象键 `sub`；`k[]` / `k[0]` 等纯数值下标 → 数组追加
/// （忽略实际下标，近似 PHP 数组追加语义）；无 `[...]` 后缀 → 顶层平键。
fn insert_bracketed(root: &mut Map<String, Value>, key: &str, value: Value) {
    let Some((base, inner)) = split_bracket(key) else {
        root.insert(key.to_string(), value);
        return;
    };

    if inner.is_empty() || inner.bytes().all(|b| b.is_ascii_digit()) {
        append_to_array(root, base, value);
    } else {
        insert_into_object(root, base, inner, value);
    }
}

/// 拆分 `k[sub]` 形式的键为 `(base, inner)`
///
/// - 无 `[`、或 `[` 后无闭合 `]`（如 `k[sub`）→ 返回 `None`，由调用方按平键处理
/// - 首个 `]` 之后仍有内容（如 `k[a][b]`）→ 视为更深嵌套，`inner` 取首个 `[`
///   之后的全部原文（即 `a][b`），按一层语义近似
fn split_bracket(key: &str) -> Option<(&str, &str)> {
    let open = key.find('[')?;
    let close = key[open..].find(']')? + open;

    if close + 1 == key.len() {
        Some((&key[..open], &key[open + 1..close]))
    } else {
        // 更深嵌套（如 k[a][b]）：inner 含 `]` 及其后原文，按一层语义近似
        Some((&key[..open], &key[open + 1..]))
    }
}

/// 数组追加：base 既有 `Value::Array` 时 push，否则重建为仅含该值的数组
/// （既有容器类型冲突时按目标类型重建，属一层近似）
fn append_to_array(root: &mut Map<String, Value>, base: &str, value: Value) {
    match root.get_mut(base) {
        Some(Value::Array(items)) => items.push(value),
        _ => {
            root.insert(base.to_string(), Value::Array(vec![value]));
        }
    }
}

/// 嵌套对象写入：base 既有 `Value::Object` 时插入，否则重建为仅含该键的对象
/// （既有容器类型冲突时按目标类型重建，属一层近似）
fn insert_into_object(root: &mut Map<String, Value>, base: &str, inner: &str, value: Value) {
    match root.get_mut(base) {
        Some(Value::Object(obj)) => {
            obj.insert(inner.to_string(), value);
        }
        _ => {
            let mut obj = Map::new();
            obj.insert(inner.to_string(), value);
            root.insert(base.to_string(), Value::Object(obj));
        }
    }
}

/// PHP truthy / `!empty()` 语义判定（含容器类型）
///
/// truthy：`Bool(true)`、非 `0` 数字、非 `""` 且非 `"0"` 的字符串、非空
/// `Array`/`Object`；falsy：`Bool(false)`、`Null`、`0` 数字、`"0"`/`""`
/// 字符串、空容器。
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty() && s != "0",
        Value::Array(items) => !items.is_empty(),
        Value::Object(obj) => !obj.is_empty(),
        Value::Null => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 多键 pack 输出键序不保证（HashMap 输入），统一按 `&` 切段排序后比对
    fn sorted_parts(packed: &str) -> Vec<&str> {
        let mut parts: Vec<&str> = packed.split('&').collect();
        parts.sort_unstable();
        parts
    }

    /// 银联 testUnpackRaw 报文（自 PHP QueryPackerTest::testUnpackRaw 原样复制，含 \r\n、+、/）
    const UNPACK_RAW_FIXTURE: &str = concat!(
        "accessType=0&bizType=000000&encoding=utf-8&merId=777290058167151&orderId=refundpay20240105165842&origQryId=052401051658427862748&queryId=052401051658427863998&respCode=00&respMsg=成功[0000000]&signMethod=01&txnAmt=1&txnSubType=00&txnTime=20240105165842&txnType=04&version=5.1.0&signPubKeyCert=-----BEGIN CERTIFICATE-----\r\n",
        "MIIEYzCCA0ugAwIBAgIFEDkwhTQwDQYJKoZIhvcNAQEFBQAwWDELMAkGA1UEBhMC\r\n",
        "Q04xMDAuBgNVBAoTJ0NoaW5hIEZpbmFuY2lhbCBDZXJ0aWZpY2F0aW9uIEF1dGhv\r\n",
        "cml0eTEXMBUGA1UEAxMOQ0ZDQSBURVNUIE9DQTEwHhcNMjAwNzMxMDExOTE2WhcN\r\n",
        "MjUwNzMxMDExOTE2WjCBljELMAkGA1UEBhMCY24xEjAQBgNVBAoTCUNGQ0EgT0NB\r\n",
        "MTEWMBQGA1UECxMNTG9jYWwgUkEgT0NBMTEUMBIGA1UECxMLRW50ZXJwcmlzZXMx\r\n",
        "RTBDBgNVBAMMPDA0MUA4MzEwMDAwMDAwMDgzMDQwQOS4reWbvemTtuiBlOiCoeS7\r\n",
        "veaciemZkOWFrOWPuEAwMDAxNjQ5NTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCC\r\n",
        "AQoCggEBAMHNa81t44KBfUWUgZhb1YTx3nO9DeagzBO5ZEE9UZkdK5+2IpuYi48w\r\n",
        "eYisCaLpLuhrwTced19w2UR5hVrc29aa2TxMvQH9s74bsAy7mqUJX+mPd6KThmCr\r\n",
        "t5LriSQ7rDlD0MALq3yimLvkEdwYJnvyzA6CpHntP728HIGTXZH6zOL0OAvTnP8u\r\n",
        "RCHZ8sXJPFUkZcbG3oVpdXQTJVlISZUUUhsfSsNdvRDrcKYY+bDWTMEcG8ZuMZzL\r\n",
        "g0N+/spSwB8eWz+4P87nGFVlBMviBmJJX8u05oOXPyIcZu+CWybFQVcS2sMWDVZy\r\n",
        "sPeT3tPuBDbFWmKQYuu+gT83PM3G6zMCAwEAAaOB9DCB8TAfBgNVHSMEGDAWgBTP\r\n",
        "cJ1h6518Lrj3ywJA9wmd/jN0gDBIBgNVHSAEQTA/MD0GCGCBHIbvKgEBMDEwLwYI\r\n",
        "KwYBBQUHAgEWI2h0dHA6Ly93d3cuY2ZjYS5jb20uY24vdXMvdXMtMTQuaHRtMDkG\r\n",
        "A1UdHwQyMDAwLqAsoCqGKGh0dHA6Ly91Y3JsLmNmY2EuY29tLmNuL1JTQS9jcmw3\r\n",
        "NTAwMy5jcmwwCwYDVR0PBAQDAgPoMB0GA1UdDgQWBBTmzk7XEM/J/sd+wPrMils3\r\n",
        "9rJ2/DAdBgNVHSUEFjAUBggrBgEFBQcDAgYIKwYBBQUHAwQwDQYJKoZIhvcNAQEF\r\n",
        "BQADggEBAJLbXxbJaFngROADdNmNUyVxPtbAvK32Ia0EjgDh/vjn1hpRNgvL4flH\r\n",
        "NsGNttCy8afLJcH8UnFJyGLas8v/P3UKXTJtgrOj1mtothv7CQa4LUYhzrVw3UhL\r\n",
        "4L1CTtmE6D1Kf3+c2Fj6TneK+MoK9AuckySjK5at6a2GQi18Y27gVF88Nk8bp1lJ\r\n",
        "vzOwPKd8R7iGFotuF4/8GGhBKR4k46EYnKCodyIhNpPdQfpaN5AKeS7xeLSbFvPJ\r\n",
        "HYrtBsI48jUK/WKtWBJWhFH+Gty+GWX0e5n2QHXHW6qH62M0lDo7OYeyBvG1mh9u\r\n",
        "Q0C300Eo+XOoO4M1WvsRBAF13g9RPSw=\r\n",
        "-----END CERTIFICATE-----&signature=c++EAuubwRkvr2MVyM9zyjbdH3RMRK/L1ttftpJ4fkl4ZSY1BjyRbTj5fx/2+Z/eH4dqPNfFEQt8egVVWhF/k7PaD8tLTaueeUIPwyjnEIWmqNtVbJtzKexCouGc8wtYDHZYxTJTgo6BW7GEgO5xD6Qpxq801Bb9Zto8uhn4BUP4HI7UsxHHIzP9JYhL2cqz2B8gb3AJHpLMEBpYv+Kb3mwq8ZFgpGaieCAFFGGWImUx1+MgCzLFoe3SKlTF13nbr39Cd3AHuDJnbN+uG1N6AwUtLu12Zzq/6SM+/dqiE0v5SpvB/PeRj9KQeiGDRg/ho9larqB+D3y0FjU13EeHng==",
    );

    #[test]
    fn test_pack_basic() {
        // 对齐 PHP QueryPackerTest::testPack（HashMap 键序不保证，切段排序比对）
        let packer = QueryPacker;
        let data = HashMap::from([
            ("name".to_string(), json!("yansongda")),
            ("age".to_string(), json!("29")),
        ]);

        let packed = packer.pack(&data, &HashMap::new()).unwrap();
        assert_eq!(sorted_parts(&packed), vec!["age=29", "name=yansongda"]);
    }

    #[test]
    fn test_pack_empty() {
        let packer = QueryPacker;
        let data = HashMap::new();

        assert_eq!(packer.pack(&data, &HashMap::new()).unwrap(), "");
    }

    #[test]
    fn test_pack_rfc1738_encoding() {
        // 空格 → `+`；`%` → `%25`
        let packer = QueryPacker;
        let data = HashMap::from([
            ("s".to_string(), json!("x y")),
            ("c".to_string(), json!("a%b")),
        ]);

        let packed = packer.pack(&data, &HashMap::new()).unwrap();
        assert_eq!(sorted_parts(&packed), vec!["c=a%25b", "s=x+y"]);
    }

    #[test]
    fn test_pack_bool_null() {
        // PHP (string) 强转：true → "1"、false → ""、null → ""
        let packer = QueryPacker;
        let data = HashMap::from([
            ("t".to_string(), json!(true)),
            ("f".to_string(), json!(false)),
            ("n".to_string(), json!(null)),
        ]);

        let packed = packer.pack(&data, &HashMap::new()).unwrap();
        assert_eq!(sorted_parts(&packed), vec!["f=", "n=", "t=1"]);
    }

    #[test]
    fn test_pack_nested_object() {
        // 推断：本机无 PHP，未经实测（基于 PHP 手册 http_build_query 示例
        // user%5Bname%5D=Bob+Smith 推导：`[`/`]` 编码为 %5B/%5D）
        let packer = QueryPacker;
        let data = HashMap::from([("a".to_string(), json!({"b": 1}))]);

        assert_eq!(packer.pack(&data, &HashMap::new()).unwrap(), "a%5Bb%5D=1");
    }

    #[test]
    fn test_pack_nested_array_uses_index() {
        // 推断：本机无 PHP，未经实测（基于 PHP http_build_query 数组下标语义推导）
        let packer = QueryPacker;
        let data = HashMap::from([("a".to_string(), json!({"l": [2, 3]}))]);

        let packed = packer.pack(&data, &HashMap::new()).unwrap();
        assert_eq!(
            sorted_parts(&packed),
            vec!["a%5Bl%5D%5B0%5D=2", "a%5Bl%5D%5B1%5D=3"]
        );
    }

    #[test]
    fn test_pack_skips_empty_containers() {
        // PHP http_build_query 跳过空数组/空对象（不产出任何键值对）
        let packer = QueryPacker;
        let data = HashMap::from([("a".to_string(), json!({})), ("b".to_string(), json!([]))]);

        assert_eq!(packer.pack(&data, &HashMap::new()).unwrap(), "");
    }

    #[test]
    fn test_unpack_default() {
        // 对齐 PHP QueryPackerTest::testUnpack（值保持字符串）
        let packer = QueryPacker;

        let result = packer
            .unpack("name=yansongda&age=29", &HashMap::new())
            .unwrap();
        assert_eq!(result, json!({"name": "yansongda", "age": "29"}));
    }

    #[test]
    fn test_unpack_default_mangle_quirk() {
        // PHP parse_str quirk：键中 `.` 与空格 → `_`
        let packer = QueryPacker;

        let result = packer.unpack("a.b=1&x y=2", &HashMap::new()).unwrap();
        assert_eq!(result, json!({"a_b": "1", "x_y": "2"}));
    }

    #[test]
    fn test_unpack_default_decoding() {
        // `+` → 空格；%XX → 字节
        let packer = QueryPacker;

        let result = packer.unpack("s=x+y&h=50%25", &HashMap::new()).unwrap();
        assert_eq!(result, json!({"s": "x y", "h": "50%"}));
    }

    #[test]
    fn test_unpack_default_nested_bracket() {
        let packer = QueryPacker;

        let result = packer.unpack("k[sub]=1", &HashMap::new()).unwrap();
        assert_eq!(result, json!({"k": {"sub": "1"}}));
    }

    #[test]
    fn test_unpack_default_array_append() {
        // `k[]` / `k[0]` 风格 → 数组追加（忽略实际下标，一层近似）
        let packer = QueryPacker;

        let result = packer
            .unpack("k[]=1&k[]=2&n[0]=a&n[1]=b", &HashMap::new())
            .unwrap();
        assert_eq!(result, json!({"k": ["1", "2"], "n": ["a", "b"]}));
    }

    #[test]
    fn test_unpack_default_no_equals_segment() {
        // 无 `=` 段：key 为整段、value 为 ""
        let packer = QueryPacker;

        let result = packer.unpack("a=1&noseq", &HashMap::new()).unwrap();
        assert_eq!(result, json!({"a": "1", "noseq": ""}));
    }

    #[test]
    fn test_unpack_raw_keeps_plus() {
        // 对齐 PHP QueryPackerTest::testUnpackBlank（_unpack_raw=1，+ 不解码）
        let packer = QueryPacker;
        let params = HashMap::from([("_unpack_raw".to_string(), json!(true))]);

        let result = packer.unpack("name=yan+song+da&age=29", &params).unwrap();
        assert_eq!(result, json!({"name": "yan+song+da", "age": "29"}));
    }

    #[test]
    fn test_unpack_raw_cert_unmodified() {
        // 对齐 PHP QueryPackerTest::testUnpackRaw：raw 解析后证书等字段逐字符无损
        let packer = QueryPacker;
        let params = HashMap::from([("_unpack_raw".to_string(), json!(true))]);

        let result = packer.unpack(UNPACK_RAW_FIXTURE, &params).unwrap();
        let Value::Object(map) = &result else {
            panic!("raw 解析应返回对象");
        };

        // 17 个字段全部解析
        assert_eq!(map.len(), 17);

        // signPubKeyCert：与输入切片逐字符相等（含 \r\n、+、/）
        let cert = map["signPubKeyCert"].as_str().unwrap();
        let cert_start =
            UNPACK_RAW_FIXTURE.find("signPubKeyCert=").unwrap() + "signPubKeyCert=".len();
        let cert_end = UNPACK_RAW_FIXTURE.find("&signature=").unwrap();
        assert!(cert.contains("\r\n") && cert.contains('+') && cert.contains('/'));
        assert_eq!(cert, &UNPACK_RAW_FIXTURE[cert_start..cert_end]);

        // respMsg：中文与 [] 不被破坏
        assert_eq!(map["respMsg"], "成功[0000000]");

        // signature：+ 与 / 不被破坏
        let signature = map["signature"].as_str().unwrap();
        let sig_start = UNPACK_RAW_FIXTURE.find("&signature=").unwrap() + "&signature=".len();
        assert_eq!(signature, &UNPACK_RAW_FIXTURE[sig_start..]);
    }

    #[test]
    fn test_unpack_raw_empty_and_no_equals() {
        // 整串为空或不含 `=` → 空 Object
        let packer = QueryPacker;
        let params = HashMap::from([("_unpack_raw".to_string(), json!(true))]);

        assert_eq!(packer.unpack("", &params).unwrap(), json!({}));
        assert_eq!(packer.unpack("noseq", &params).unwrap(), json!({}));
    }

    #[test]
    fn test_unpack_raw_segment_without_equals() {
        // 推断：本机无 PHP，未经实测（基于 PHP substr($item, 0, false) → "" /
        // substr($item, false + 1) → 去掉第一个字符 的强转语义推导）
        let packer = QueryPacker;
        let params = HashMap::from([("_unpack_raw".to_string(), json!(true))]);

        let result = packer.unpack("a=1&noseq", &params).unwrap();
        assert_eq!(result, json!({"a": "1", "": "oseq"}));
    }

    #[test]
    fn test_unpack_raw_truthy_matrix() {
        // 对齐 PHP truthy / !empty()（含容器类型）：truthy 参数走 raw 模式
        //（+ 保留），falsy 参数走默认模式（+ 转空格）
        let packer = QueryPacker;
        let cases = vec![
            (json!(true), true),
            (json!(false), false),
            (json!(null), false),
            (json!(1), true),
            (json!(0), false),
            (json!(-1), true),
            (json!(0.0), false),
            (json!(1.5), true),
            (json!("1"), true),
            (json!("0"), false),
            (json!(""), false),
            (json!("raw"), true),
            (json!([1]), true),
            (json!([]), false),
            (json!({"k": 1}), true),
            (json!({}), false),
        ];

        for (raw_param, expected) in cases {
            let params = HashMap::from([("_unpack_raw".to_string(), raw_param.clone())]);
            let result = packer.unpack("name=yan+song+da", &params).unwrap();
            let plus_kept = result["name"] == "yan+song+da";
            assert_eq!(plus_kept, expected, "参数 {raw_param} truthy 判定不符");
        }
    }

    #[test]
    fn test_content_type() {
        assert_eq!(
            QueryPacker.content_type(),
            Some("application/x-www-form-urlencoded")
        );
    }
}
