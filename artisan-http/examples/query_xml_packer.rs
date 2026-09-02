//! QueryPacker / XmlPacker 使用示例
//!
//! 演示在链上替换 `rocket.packer`（0.17.0 起链尾 `ParserPlugin` 经它 pack
//! 请求体、unpack 响应体）的三种组合：
//!
//! 1. `QueryPacker` + Response 方向：payload 按 RFC1738（`http_build_query`
//!    语义）编码为 `application/x-www-form-urlencoded` 表单体。因 httpbin.org/post
//!    的响应是 JSON（与 QueryPacker 的解析语义不匹配），解析方向用 Response
//!    取原始响应，并从服务端回显中打印收到的表单体。真实场景（如银联证书
//!    网关）响应即 query 串，可配合 payload 中的 `_unpack_raw` 参数逐字符
//!    无损解析证书，详见 `tests/parser_test.rs` 的 raw 模式用例。
//! 2. `XmlPacker` + Response 方向：payload 打包为 `<xml><k><![CDATA[v]]></k></xml>`
//!    请求体（`application/xml`），同样从 httpbin.org/post 回显中确认请求体。
//! 3. `XmlPacker` + 默认 Json 方向：httpbin.org/xml 返回 XML，链尾 `ParserPlugin`
//!    经 rocket.packer 解析为 `Destination::Json`（根元素值即结果、叶子文本为
//!    字符串、同名兄弟元素转数组、混合内容丢弃直接文本）。
//!
//! 目标端点沿用 basic.rs 的做法：向公共示例服务 httpbin.org 发起真实请求
//! （examples 不新增依赖、不引入 wiremock 以保持自包含）；外部网络不可用时
//! 打印警告后继续后续演示。

use artisan_http::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{
    Artful, Packer, Plugin, QueryPacker, Rocket, XmlPacker, direction::DirectionKind,
    flow_ctrl::Next,
};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

/// 设置 HTTP 方法和 URL 的插件
struct MethodUrlPlugin {
    method: reqwest::Method,
    url: String,
}

#[async_trait]
impl Plugin for MethodUrlPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.method = self.method.clone();
        rocket.config.url = self.url.clone();
        next.call(rocket).await
    }
}

/// 设置响应解析策略的插件
struct SetDirectionPlugin {
    direction: DirectionKind,
}

#[async_trait]
impl Plugin for SetDirectionPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.config.direction = self.direction.clone();
        next.call(rocket).await
    }
}

/// 链上替换 rocket.packer 的插件（ParserPlugin 按它 pack/unpack）
struct ReplacePackerPlugin(Arc<dyn Packer>);

#[async_trait]
impl Plugin for ReplacePackerPlugin {
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> artisan_http::Result<()> {
        rocket.packer = self.0.clone();
        next.call(rocket).await
    }
}

#[tokio::main]
async fn main() -> artisan_http::Result<()> {
    let artful = Artful::new()?;

    // ---- 演示 1：QueryPacker - payload 编码为 query 表单体 ----
    let mut params = HashMap::new();
    params.insert("biz_type".to_string(), json!("purchase"));
    params.insert("order_no".to_string(), json!("202609010001"));

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: "https://httpbin.org/post".to_string(),
        }),
        Arc::new(ReplacePackerPlugin(Arc::new(QueryPacker))),
        Arc::new(SetDirectionPlugin {
            direction: DirectionKind::Response,
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        // 链尾挂载 ParserPlugin：0.17.0 起负责按 direction 分发解析
        Arc::new(ParserPlugin),
    ];

    match artful.artful(params, plugins).await {
        Ok(artisan_http::Destination::Response(response)) => {
            let echo: serde_json::Value = response.json().await?;
            println!("QueryPacker 表单体（服务端回显 form）: {}", echo["form"]);
        }
        Ok(other) => println!("Destination: {other:?}"),
        // 外部网络不可用时走到这里：打印警告后继续后续演示
        Err(err) => eprintln!("[warn] request failed (network unavailable?): {err}"),
    }

    // ---- 演示 2：XmlPacker - payload 打包为 XML 请求体 ----
    let mut params = HashMap::new();
    params.insert("out_trade_no".to_string(), json!("202609010002"));
    params.insert("total_amount".to_string(), json!("10.00"));

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: "https://httpbin.org/post".to_string(),
        }),
        Arc::new(ReplacePackerPlugin(Arc::new(XmlPacker))),
        Arc::new(SetDirectionPlugin {
            direction: DirectionKind::Response,
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    match artful.artful(params, plugins).await {
        Ok(artisan_http::Destination::Response(response)) => {
            let echo: serde_json::Value = response.json().await?;
            println!("XmlPacker 请求体（服务端回显 data）: {}", echo["data"]);
        }
        Ok(other) => println!("Destination: {other:?}"),
        Err(err) => eprintln!("[warn] request failed (network unavailable?): {err}"),
    }

    // ---- 演示 3：XmlPacker - 响应 XML 解析（packer=XmlPacker + 默认 Json 方向） ----
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "https://httpbin.org/xml".to_string(),
        }),
        Arc::new(ReplacePackerPlugin(Arc::new(XmlPacker))),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    match artful.artful(HashMap::new(), plugins).await {
        Ok(artisan_http::Destination::Json(json)) => println!("XmlPacker 解析结果: {}", json),
        Ok(other) => println!("Destination: {other:?}"),
        Err(err) => eprintln!("[warn] request failed (network unavailable?): {err}"),
    }

    Ok(())
}
