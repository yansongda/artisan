//! Direction 响应解析策略示例

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
    // 默认 JsonDirection - 解析为 JSON
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

    // ResponseDirection - 返回原始 Response
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

    Ok(())
}
