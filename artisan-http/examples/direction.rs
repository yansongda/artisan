//! Direction 响应解析策略示例
//!
//! 0.17.0 起：响应解析由链尾挂载的 `ParserPlugin` 承担（IgniteCore 只执行 HTTP），
//! 忘挂时请求发出但 `destination` 保持 `None`。以下各演示链均在链尾挂载。

use artisan_http::plugins::{AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{Artful, Plugin, Rocket, direction::DirectionKind, flow_ctrl::Next};
use async_trait::async_trait;
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

#[tokio::main]
async fn main() -> artisan_http::Result<()> {
    // 默认 JsonDirection - 解析为 JSON（链尾 ParserPlugin 负责解析）
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "https://httpbin.org/get".to_string(),
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new()?;

    let result = artful.artful(HashMap::new(), plugins).await?;

    if let artisan_http::Destination::Json(json) = result {
        println!("JSON Response: {}", json);
    }

    // ResponseDirection - 返回原始 Response（0.17.0 起经链尾 ParserPlugin 分发到
    // OriginResponseDirection），可先读取 status/headers 再消费 body
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "https://httpbin.org/get".to_string(),
        }),
        Arc::new(SetDirectionPlugin {
            direction: DirectionKind::Response,
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(HashMap::new(), plugins).await?;

    if let artisan_http::Destination::Response(response) = result {
        println!("Response status: {}", response.status());
        println!("Response headers: {:?}", response.headers());
    }

    // NoRequestDirection - 不发起 HTTP 请求（DirectionKind::NoRequest 短路）：
    // 链尾 ParserPlugin 经 NoHttpRequestDirection 透传 destination，
    // 无响应可解析时返回 Destination::None
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "https://httpbin.org/get".to_string(),
        }),
        Arc::new(SetDirectionPlugin {
            direction: DirectionKind::NoRequest,
        }),
        Arc::new(ParserPlugin),
    ];

    let result = artful.artful(HashMap::new(), plugins).await?;

    if let artisan_http::Destination::None = result {
        println!("NoRequestDirection: 未发起 HTTP 请求，destination 透传为 None");
    }

    Ok(())
}
