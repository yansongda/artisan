[English](./README.md) | 简体中文

# Artisan

## Workspace 结构

```
artisan/
├── Cargo.toml              # Workspace 配置
├── src/lib.rs              # Facade（Feature 控制的 re-export）
└── artisan-http/           # HTTP 实现
    ├── src/                # 核心实现
    ├── tests/              # 测试
    ├── examples/           # 示例
    └── docs/               # 架构文档
```

## Crate 说明

| Crate | 职责 | 文档 |
|-------|------|------|
| [`artisan`](.) | Facade，Feature 控制的 re-export | [docs.rs/artisan](https://docs.rs/artisan) |
| [`artisan-http`](./artisan-http) | HTTP 客户端、洋葱模型、插件系统 | [README](./artisan-http/README.zh-CN.md#快速开始) |

## 安装

```bash
# 推荐：通过 facade（默认包含 HTTP 功能）
cargo add artisan

# 直接依赖实现层
cargo add artisan-http

# 纯 facade（禁用 HTTP 功能）
cargo add artisan --no-default-features
```

```toml
# Cargo.toml
[dependencies]
artisan = "0.17.0"

# 直接依赖实现层
[dependencies]
artisan-http = "0.17.0"

# 纯 facade（禁用 HTTP 功能）
[dependencies]
artisan = { version = "0.17.0", default-features = false }
```

## 快速入口

### artisan-http

- **快速开始**: [README](./artisan-http/README.zh-CN.md#快速开始)
- **架构设计**: [docs/ARCHITECTURE.md](./artisan-http/docs/ARCHITECTURE.md)
- **示例代码**: [examples/](./artisan-http/examples/)

### 事件系统（artisan-http）

每个 `Artful` 实例内置事件分发器：经 builder 注册监听器，即可观测请求生命周期，无需编写完整插件（未注册监听器时零开销）。

```rust
use artisan_http::{Artful, Event, EventListener};
use std::sync::Arc;

struct LoggingListener;

impl EventListener for LoggingListener {
    fn name(&self) -> &'static str {
        "LoggingListener"
    }

    fn on_event(&self, event: &mut Event<'_>) -> artisan_http::Result<()> {
        match event {
            Event::ArtfulStart { params, plugins } => {
                eprintln!("ArtfulStart: {} params, {} plugins", params.len(), plugins.len())
            }
            Event::HttpStart { rocket } => {
                eprintln!("HttpStart: {} {}", rocket.config.method, rocket.config.url)
            }
            Event::HttpEnd { rocket } => {
                eprintln!("HttpEnd: {:?}", rocket.destination_origin.as_ref().map(|r| r.status()))
            }
            Event::HttpError { error, .. } => eprintln!("HttpError: {error}"),
            Event::ArtfulEnd { .. } => eprintln!("ArtfulEnd"),
        }
        Ok(()) // 旁路监听器：内部消化错误，永不返回 Err
    }
}

let artful = Artful::builder()
    .event_listener(Arc::new(LoggingListener))
    .build()?;
```

| 事件 | 触发时机 | 可变性 |
|------|---------|--------|
| `ArtfulStart` | 插件链启动前 | 只读 |
| `HttpStart` | HTTP 请求即将发出、到达链尾核心动作执行点（`IgniteCore`，由框架自动挂载；正常链中 radar 已构建；链中缺 `AddRadarPlugin` 时为 `None`，事件仍触发；须经 `rocket.radar` 的 `*_mut` 访问器修改请求） | 可变 |
| `HttpEnd` | 请求成功返回、解析前（响应体不可读：body 消费权属于 direction 解析，仅可读 status / headers） | 只读 |
| `HttpError` | HTTP 请求执行失败（错误照常向上传播） | 只读 |
| `ArtfulEnd` | 链成功后、返回 destination 前（可改写 `rocket.destination`） | 可变 |

> - 监听器是**同步**回调，必须非阻塞——耗时任务请自行 `tokio::spawn`。
> - 监听器返回 `Err` 会中止主流程（以 `EventListenerError` 传播）。旁路监听器应内部消化错误、恒返回 `Ok(())`。
> - `HttpEnd` 无法读取响应体（消费权属于 direction 解析），该事件下仅可读 status / headers。

详见 [artisan-http README 事件系统小节](./artisan-http/README.zh-CN.md#事件系统)，或试一试 `cargo run -p artisan-http --example event`。

## 示例

### artisan-http

```bash
cargo run -p artisan-http --example basic
cargo run -p artisan-http --example config
cargo run -p artisan-http --example shortcut
cargo run -p artisan-http --example custom_plugin
cargo run -p artisan-http --example direction
```

## 许可证

MIT License
