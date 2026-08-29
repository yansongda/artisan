//! 配置初始化示例

use artisan_http::{Artful, ClientOptions, Config};
use serde_json::json;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> artisan_http::Result<()> {
    // 基础用法：默认配置创建实例（构造时即构建 client，fail-fast）
    let artful = Artful::new()?;

    // 带 HTTP 客户端选项的配置：client 级选项对该实例发出的所有请求生效
    let config_with_http = Config {
        http: ClientOptions {
            timeout: Some(10),
            connect_timeout: Some(5),
            pool_idle_timeout: Some(90),
            pool_max_idle_per_host: Some(20),
            user_agent: Some("my-app/1.0".to_string()),
        },
        extra: HashMap::new(),
    };
    let artful_with_http = Artful::with_config(config_with_http)?;

    // 经实例读取配置
    println!(
        "HTTP timeout: {:?}, connect_timeout: {:?}",
        artful_with_http.config().http.timeout,
        artful_with_http.config().http.connect_timeout
    );

    // 带扩展配置（如支付渠道配置）
    let mut extra = HashMap::new();
    extra.insert(
        "alipay".to_string(),
        json!({
            "app_id": "2016082000295641",
            "notify_url": "https://example.com/alipay/notify",
        }),
    );
    extra.insert(
        "wechat".to_string(),
        json!({
            "mch_id": "1234567890",
            "notify_url": "https://example.com/wechat/notify",
        }),
    );

    let config_with_extra = Config {
        extra,
        http: ClientOptions {
            timeout: Some(5),
            connect_timeout: Some(3),
            ..Default::default()
        },
    };
    let artful_with_extra = Artful::with_config(config_with_extra)?;

    // 读取扩展配置中的渠道信息
    if let Some(alipay) = artful_with_extra.config().extra.get("alipay") {
        println!("Alipay config: {alipay}");
    }

    // 实例持有的 HTTP 客户端可直接使用
    println!("client ready: {:?}", artful.client());

    println!("Config initialized successfully!");

    Ok(())
}
