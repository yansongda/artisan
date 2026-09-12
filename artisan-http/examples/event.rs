//! 事件系统使用示例
//!
//! 运行：`cargo run -p artisan-http --example event`
//!
//! 演示：注册旁路日志监听器，观测一次完整 HTTP 请求的生命周期事件
//! （ArtfulStart → HttpStart → HttpEnd → ArtfulEnd）。
//!
//! 要点：
//! - 监听器是**同步**回调，必须非阻塞——耗时任务（IO/重计算）请自行 `tokio::spawn`；
//! - 本示例的监听器只记录日志、**恒返回 `Ok(())`**（旁路观察的标准写法）：
//!   监听器一旦返回 `Err`，会以 `EventListenerError` 中断主流程；
//! - 示例向 httpbin.org 发起真实请求，外部网络不可用时打印警告后正常退出。

use artisan_http::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{
    Artful, Destination, Event, EventListener, Plugin, Result, Rocket, flow_ctrl::Next,
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
    async fn assembly(&self, rocket: &mut Rocket, next: Next<'_>) -> Result<()> {
        rocket.config.method = self.method.clone();
        rocket.config.url = self.url.clone();
        next.call(rocket).await
    }
}

/// 旁路日志监听器：记录每个生命周期事件，内部消化错误、恒返回 Ok
struct LoggingListener;

impl EventListener for LoggingListener {
    fn name(&self) -> &'static str {
        "LoggingListener"
    }

    fn on_event(&self, event: &mut Event<'_>) -> Result<()> {
        match event {
            Event::ArtfulStart { params, plugins } => {
                eprintln!(
                    "[event] ArtfulStart: {} params, {} plugins",
                    params.len(),
                    plugins.len()
                );
            }
            Event::HttpStart { rocket } => {
                eprintln!(
                    "[event] HttpStart: {} {}",
                    rocket.config.method, rocket.config.url
                );
            }
            Event::HttpEnd { rocket } => {
                let status = rocket
                    .destination_origin
                    .as_ref()
                    .map_or_else(|| "unknown".to_string(), |r| r.status().to_string());
                eprintln!("[event] HttpEnd: status {status}");
            }
            Event::HttpError { error, .. } => {
                eprintln!("[event] HttpError: {error}");
            }
            Event::ArtfulEnd { rocket } => {
                eprintln!("[event] ArtfulEnd: destination = {:?}", rocket.destination);
            }
        }

        // 旁路观察：不向上传播错误，避免日志故障影响主流程
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let artful = Artful::builder()
        .event_listener(Arc::new(LoggingListener))
        .build()?;

    let mut params = HashMap::new();
    params.insert("order_id".to_string(), json!("123"));
    params.insert("amount".to_string(), json!(100));

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::POST,
            url: "https://httpbin.org/post".to_string(),
        }),
        Arc::new(AddPayloadBodyPlugin),
        Arc::new(AddRadarPlugin),
        // 链尾挂载 ParserPlugin：0.17.0 起响应解析由它承担
        Arc::new(ParserPlugin),
    ];

    match artful.artful(params, plugins).await {
        Ok(Destination::Json(json)) => println!("Response: {json}"),
        Ok(other) => println!("Destination: {other:?}"),
        // 外部网络不可用时走到这里：打印警告后正常退出
        Err(err) => eprintln!("[warn] request failed (network unavailable?): {err}"),
    }

    // ---- 演示：插件链不含任何解析插件，HTTP 事件依然触发 ----
    // HttpStart/HttpEnd 由框架自动挂载的链尾核心动作（IgniteCore）分发：
    // 插件链只需准备请求（method/url/radar），无需挂载任何"执行请求"的插件。
    // 注意链不可为空：空链会 fail-fast 报 MissingRequest 且不触发 HttpEnd。
    // 0.17.0 起响应解析由 ParserPlugin 承担：本段刻意不挂它——请求照常发出、
    // 事件照常触发，但 destination 保持 None，说明事件系统与响应解析完全解耦。
    eprintln!("\n[event] ---- 插件链不含任何解析插件，HTTP 事件依然触发 ----");

    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "https://httpbin.org/get".to_string(),
        }),
        Arc::new(AddRadarPlugin),
        // 刻意不挂解析插件：见上方说明
    ];

    match artful.artful(HashMap::new(), plugins).await {
        // 未挂解析插件：请求发出但 destination 为 None
        Ok(Destination::None) => {
            println!("未挂 ParserPlugin，destination 为 None（0.17.0 起解析由 ParserPlugin 承担）")
        }
        Ok(other) => println!("Destination: {other:?}"),
        // 外部网络不可用时走到这里：打印警告后正常退出
        Err(err) => eprintln!("[warn] request failed (network unavailable?): {err}"),
    }

    Ok(())
}
