# Artisan

## Workspace 结构

```
artisan/
├── Cargo.toml              # Workspace 配置
├── src/lib.rs              # Facade（Feature 控制的 re-export）
└── artisan-http/           # HTTP 实现
    ├── src/                # 核心实现
    ├── tests/              # 测试（68 个）
    ├── examples/           # 示例
    └── docs/               # 架构文档
```

## Crate 说明

| Crate | 职责 | 文档 |
|-------|------|------|
| [`artisan`](.) | Facade，Feature 控制的 re-export | [docs.rs/artisan](https://docs.rs/artisan) |
| [`artisan-http`](./artisan-http) | HTTP 客户端、洋葱模型、插件系统 | [README](./artisan-http/README.md) |

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
artisan = "~0.14.0"

# 直接依赖实现层
[dependencies]
artisan-http = "~0.14.0"

# 纯 facade（禁用 HTTP 功能）
[dependencies]
artisan = { version = "~0.14.0", default-features = false }
```

## 快速入口

### artisan-http

- **快速开始**: [README](./artisan-http/README.md#快速开始)
- **架构设计**: [docs/ARCHITECTURE.md](./artisan-http/docs/ARCHITECTURE.md)
- **示例代码**: [examples/](./artisan-http/examples/)

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
