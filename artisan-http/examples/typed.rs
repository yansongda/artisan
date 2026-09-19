//! 强类型反序列化示例（artful_as）
//!
//! 演示 `Artful::artful_as::<T>`：一次调用完成请求 + JSON 解析 + 反序列化为
//! 强类型结构体（等价于 `artful` + `Destination::into_json` + `from_value`）。
//!
//! - 成功路径：向 httpbin.org/json 发起真实请求，响应反序列化为 `OrderResp`
//!   （外部网络不可用时打印警告后继续，见 query_xml_packer.rs 惯例）；
//! - 错误路径：`DirectionKind::NoRequest` 短路不发起请求，destination 为 None，
//!   经 `into_json` 报 `DestinationMismatch { expected: "Json", actual: "None" }`
//!   （不依赖外部网络，恒可演示）。

use artisan_http::plugins::{AddRadarPlugin, ParserPlugin, StartPlugin};
use artisan_http::{Artful, DirectionKind, Plugin, Rocket, flow_ctrl::Next};
use async_trait::async_trait;
use serde_json::Map;
use std::sync::Arc;

/// 与 httpbin.org/json 响应结构对应的强类型（仅取演示所需字段）
#[derive(serde::Deserialize)]
struct OrderResp {
    slideshow: Slideshow,
}

#[derive(serde::Deserialize)]
struct Slideshow {
    author: String,
    title: String,
    slides: Vec<Slide>,
}

#[derive(serde::Deserialize)]
struct Slide {
    title: String,
    #[serde(rename = "type")]
    slide_type: String,
}

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
    let artful = Artful::new()?;

    // ---- 成功路径：artful_as 将 httpbin.org/json 响应反序列化为 OrderResp ----
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(MethodUrlPlugin {
            method: reqwest::Method::GET,
            url: "https://httpbin.org/json".to_string(),
        }),
        Arc::new(AddRadarPlugin),
        Arc::new(ParserPlugin),
    ];

    match artful.artful_as::<OrderResp>(Map::new(), plugins).await {
        Ok(resp) => {
            println!("OrderResp 反序列化成功:");
            println!("  slideshow.title: {}", resp.slideshow.title);
            println!("  slideshow.author: {}", resp.slideshow.author);
            println!("  slideshow.slides: {} 张", resp.slideshow.slides.len());
            for slide in &resp.slideshow.slides {
                println!("    - [{}] {}", slide.slide_type, slide.title);
            }
        }
        // 外部网络不可用时走到这里：打印警告后继续后续演示
        Err(err) => eprintln!("[warn] artful_as failed (network unavailable?): {err}"),
    }

    // ---- 错误路径：NoRequest 方向 destination 为 None → DestinationMismatch ----
    // NoRequest 短路不发起 HTTP 请求，此演示不依赖外部网络
    let plugins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(StartPlugin),
        Arc::new(SetDirectionPlugin {
            direction: DirectionKind::NoRequest,
        }),
        Arc::new(ParserPlugin),
    ];

    match artful.artful_as::<OrderResp>(Map::new(), plugins).await {
        Ok(_) => println!("unexpected: NoRequest 方向不应反序列化成功"),
        Err(artisan_http::ArtfulError::DestinationMismatch { expected, actual }) => {
            println!(
                "NoRequest 方向如预期报错: DestinationMismatch {{ expected: {expected}, actual: {actual} }}"
            );
        }
        Err(err) => println!("unexpected error: {err:?}"),
    }

    Ok(())
}
