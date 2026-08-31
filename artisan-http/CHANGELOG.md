# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.16.0] - 2026-08-31

### Added

- 事件系统核心类型：`Event`（5 个生命周期事件的借用视图）、`EventListener`（对象安全同步监听器 trait）、`EventDispatcher`（`Clone` 共享监听器 `Arc`，实现 `Debug`/`Default`）；随 `Artful` 实例持有，无任何全局状态，未注册监听器时零开销
- 5 个生命周期事件与触发时机：`ArtfulStart`（插件链启动前，只读观测 params/plugins）/ `HttpStart`（请求即将发出、radar 已构建，可经 `rocket.radar` 的 `*_mut` 访问器修改请求；此时改 `rocket.config` 不影响本次请求）/ `HttpEnd`（请求成功返回、响应解析前，只读）/ `HttpError`（仅 HTTP execute 失败触发，错误照常向上传播，只读；PHP 版无对应物）/ `ArtfulEnd`（插件链成功后、返回 destination 前，可改写 `rocket.destination`）；`NoRequest` 方向不触发 HTTP 事件，`MissingRequest` 不触发 `HttpError`
- `ArtfulBuilder::event_listener(Arc<dyn EventListener>)`：追加式注册（注册顺序即执行顺序，区别于 config/customize/client 的覆盖语义）；监听器返回 `Err` 即中止后续监听器，错误包装为新错误变体 `EventListenerError { listener_name, message, source, original }` 向主流程传播；`original` 仅在 HttpError 分发中监听器失败时填充（保留被顶替的 `RequestFailed`，错误链不丢失）
- `Rocket` 新增 `events` 字段（`Option<Arc<EventDispatcher>>`，默认 `None`，由 `Artful::artful()` 注入传载进插件链；手动构造的 `Rocket` 不分发事件）

## [0.15.0] - 2026-08-30

### Added

- `Artful::builder()` 链式构建器：`ArtfulBuilder` 提供 `config()` / `customize()` / `client()` 可选 setter（后写覆盖先写）与 `build() -> Result<Artful>`；构建优先级为注入 client > config+customize；`ArtfulBuilder` 实现 `Default` 与 `Debug`，满足 `Send`（非 `Sync`）
  ([5bd0e97](https://github.com/yansongda/artisan/commit/5bd0e97))

### Changed

- **BREAKING**: `Artful::with_builder` 更名 `with_client_builder`（原 `with_builder` 物理删除，无兼容层；语义不变：以 `config.http` 为基座、回调叠加 `reqwest::ClientBuilder` 能力后构建）
  ([5bd0e97](https://github.com/yansongda/artisan/commit/5bd0e97))

## [0.14.0] - 2026-08-29

> **破坏性变更版本**：本版本集中清理历史 API（不做兼容层），升级前请完整阅读以下内容。

### ⚠️ 行为变更（升级必读）

- **JSON 请求默认携带 `Content-Type: application/json`**：默认插件链（`AddPayloadBodyPlugin` 打包分支与 `AddRadarPlugin` fallback 打包分支）在请求头缺失 `Content-Type` 时按 `Packer::content_type()` 自动补头；用户显式设置的任何 `Content-Type` 永不覆盖。判重按头名不区分大小写（RFC 9110）：无论以 `Content-Type` 还是小写 `content-type` 显式设置，均不会补出重复头。注意：头存储（`config.headers`）区分大小写——若自行同时设置不同大小写的同名键（如 `Content-Type` 与 `content-type`），两者都会发送，请勿重复设置。
- **此前无效的全局配置开始生效**：`Config.http.timeout` / `connect_timeout` 原为死字段（构建 client 时被忽略），本版本起真正接线。升级后请复核已配置的超时取值，原依赖"配置无效"行为的请求可能出现超时。

### Added

- `Artful` 实例类型：`new()` / `with_config(Config)`（构造时构建 client，fail-fast 返回 `ClientBuildError`）/ `config()` / `client()` / `artful()` / `shortcut()` / `raw()`；配合 `std::sync::LazyLock` 可构建全局单例或多实例
- `Artful::with_builder(config, customize)`：以 `config.http` 为基座（框架默认值兜底）、经回调叠加 `reqwest::ClientBuilder` 能力（代理、TLS 证书、cookie 会话、重定向策略等）后构建；回调内后写的 setter 覆盖先写值
- `Artful::with_client(config, client)`：注入外部构建的 `reqwest::Client`（跨实例共享连接池、client 在别处构建的场景）；注入的 client 原样生效，`config.http` 不作用于它，仅作为配置记录。需要基于 `config.http` 的定制构建请用 `with_builder`
- `Packer::content_type()` 默认方法（默认返回 `None`），`JsonPacker` 返回 `Some("application/json")`，自定义 Packer 声明自己的 MIME 即自动生效
- 错误变体 `ClientBuildError { source: reqwest::Error }`（与 `RequestBuildError` 命名对齐）

### Changed

- **BREAKING**: `HttpOptions` 拆分为 `ClientOptions`（client 级：`timeout` / `connect_timeout` / `pool_idle_timeout` / `pool_max_idle_per_host` / `user_agent`，`user_agent` 由 `Option<&'static str>` 放宽为 `Option<String>`）与 `RequestOptions`（请求级，仅 `timeout`）；`Config.http` 为 `ClientOptions`，`RocketConfig.http` 为 `RequestOptions`，per-request 误设 pool 字段改为编译错误
- **BREAKING**: `Artful` 由静态类 + `OnceLock` 全局配置改为实例类型，删除全部静态 API（`Artful::config()` / `Artful::get_config()` / `Artful::has_config()`）与 `GLOBAL_CONFIG`；原单元结构体上的 `Default`/`Copy` derive 随之移除，`Artful::default()`、unit 字面量 `Artful` 与按值 `Copy` 传递不再可用
- **BREAKING**: 错误变体 `InvalidUrl` 更名 `RequestBuildError`，语义覆盖 `request_builder.build()` 全部失败而非仅 URL
- **BREAKING**: `ArtfulError` 全部 Display 消息英文化（变体名不变，程序化匹配不受影响）

### Removed

- **BREAKING**: 删除公共 `artisan_http::get_client()`（HTTP client 由 `Artful` 实例持有，`Rocket.client` 注入）
- **BREAKING**: 删除 `FlowCtrl::cease()`（与 `skip_rest()` 行为相同），统一使用 `skip_rest()`
- **BREAKING**: 删除 `Rocket::merge_payload(HashMap)`，由 `Rocket::merge_params_to_payload()` 替代（省一次中间 HashMap 分配）
- **BREAKING**: 删除请求级 `connect_timeout` 字段（reqwest 0.13 的 `RequestBuilder` 不提供该方法，连接超时收敛为 client 级专属）

### Fixed

- 默认插件链发出的 JSON body 缺失 `Content-Type` 头（对接严格校验 Content-Type 的服务端会失败）
- `Config.http.timeout` / `connect_timeout` 死字段：client 级全部字段接线生效（请求级 `RequestBuilder::timeout` 自动覆盖 client 级默认）
- 文档与元数据：README/AGENTS/ARCHITECTURE 同步实例 API；`Cargo.toml` 补 `readme` / `keywords` / `categories`

## [0.13.1] - 2026-05-06

### Changed

- 移除冗余的 docs.rs 配置和 `doc(cfg)` 属性
  ([5bee973](https://github.com/yansongda/artisan/commit/5bee973))
- 完善文档注释，清理依赖
  ([f622f7c](https://github.com/yansongda/artisan/commit/f622f7c))

## [0.13.0] - 2026-05-06

### Changed

- 重命名 `artisan.rs` 为 `artful.rs`，提高命名一致性
  ([65085f4](https://github.com/yansongda/artisan/commit/65085f4))

## [0.12.0] - 2026-05-06

### Changed

- 重构为 workspace 结构，引入 `artisan-http` crate
  ([9261167](https://github.com/yansongda/artisan/commit/9261167))
- 更新 tokio 依赖至 ~1.52.0
  ([ed5f65a](https://github.com/yansongda/artisan/commit/ed5f65a))

### Documentation

- 补充 v0.11.0 CHANGELOG
  ([e1d1ace](https://github.com/yansongda/artisan/commit/e1d1ace))

## [0.11.0] - 2026-04-15

### Changed

- 移除 `Shortcut` trait 的 `Default` bound，使其 dyn compatible
  ([c4c1b9f](https://github.com/yansongda/artful-rs/commit/c4c1b9f))
- 代码优化 - 错误处理、架构、性能
  ([b5f3e5d](https://github.com/yansongda/artful-rs/commit/b5f3e5d))

### Style

- 修复所有 clippy pedantic 警告
  ([4651893](https://github.com/yansongda/artful-rs/commit/4651893))

### Documentation

- 添加 CHANGELOG.md 记录版本变更
  ([07f60e3](https://github.com/yansongda/artful-rs/commit/07f60e3))
- CHANGELOG 补充 commit 链接
  ([7dd4dd3](https://github.com/yansongda/artful-rs/commit/7dd4dd3))

## [0.10.0] - 2026-04-14

### Changed

- 重命名包名 `artful` → `artisan`，避免 crates.io 冲突
  ([0e7f91a](https://github.com/yansongda/artful-rs/commit/0e7f91a))
- 简化 `DirectionKind` 枚举命名
  ([42a37d9](https://github.com/yansongda/artful-rs/commit/42a37d9))
- 改进 API 设计和错误处理
  ([9eb78e6](https://github.com/yansongda/artful-rs/commit/9eb78e6))

### Fixed

- 修复 `JsonDirection` 错误类型映射，为零大小插件添加 `Clone + Copy` trait
  ([4e1ab9d](https://github.com/yansongda/artful-rs/commit/4e1ab9d))

### Added

- 添加完整测试覆盖 (59 tests)
  ([325641e](https://github.com/yansongda/artful-rs/commit/325641e))
- 添加 `AGENTS.md` 指导文件
  ([28f7afb](https://github.com/yansongda/artful-rs/commit/28f7afb))

### Documentation

- 更新 `AGENTS.md` 强调提交前验证流程
  ([7cb0db4](https://github.com/yansongda/artful-rs/commit/7cb0db4))

### Style

- `cargo fmt` 格式化代码
  ([7cb0db4](https://github.com/yansongda/artful-rs/commit/7cb0db4))

## [0.9.0] - 2025-XX-XX

Initial release with core onion model architecture.

### Added

- 洋葱模型 HTTP 客户端框架
- `Plugin` trait 中间件系统
- `Rocket` 请求载体
- `Direction` 响应解析策略
- `Packer` 序列化接口
- `Shortcut` 插件预设
- 内置插件: `StartPlugin`, `AddPayloadBodyPlugin`, `AddRadarPlugin`, `ParserPlugin`
- 全局 HTTP Client 单例 (OnceLock)
- `JsonDirection`, `JsonPacker` 默认实现
