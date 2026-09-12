//! 基础使用示例

use artisan_http::plugins::{AddPayloadBodyPlugin, AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{Artful, Plugin, Rocket, flow_ctrl::Next};
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

#[tokio::main]
async fn main() -> artisan_http::Result<()> {
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
        // 链尾挂载 ParserPlugin：0.17.0 起响应解析由它承担，忘挂时请求发出
        // 但 destination 保持 None，下方 Json 分支不会命中
        Arc::new(ParserPlugin),
    ];

    let artful = Artful::new()?;
    let result = artful.artful(params, plugins).await?;

    if let artisan_http::Destination::Json(json) = result {
        println!("Response: {}", json);
    }

    Ok(())
}
