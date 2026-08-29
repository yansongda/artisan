# artisan-http 0.14.0 优化改造 - 技术设计文档

> **时间**：2026-08-29
> **作者**：GLM-5.3-Flash + yansongda
> **状态**：经过人工审核确认（2026-08-29，对话内逐项确认）；2026-08-29 按 plan-reviewer 初审意见修订（B1：`RequestOptions` 去掉 `connect_timeout`；M1-M3：补 config.rs / merge_payload 测试迁移 / README 版本号）；2026-08-29 按执行前终审意见修订（补第 7 用例 `fallback_branch_sets_content_type`，测试计数修正为 59−1+7=65；修正 §2 add_radar 表项 connect_timeout 残留、§5 文件数、ARCHITECTURE 行数）；2026-08-29 按落盘后独立审查修订（facade 路径更正为仓库根 `src/lib.rs`；src 与测试合并为同一推进步、同一 commit 以符合仓库 AGENTS.md 三件套强制规则；7 个新用例固定落入既有 7 个测试文件）；2026-08-29 按独立审查意见修订（M1：§3.3 AddRadarPlugin fallback 补头落点明确为 request_builder；M2：附录 B 零 panic 变体调用点改为可编译写法；m1-m5：README L234 断链纳入修复、新用例前置条件注明、补头片段位置注明、§5 计数与措辞勘误）
> **决策记录**：不考虑向后兼容；`HttpOptions` 拆分为 `ClientOptions`/`RequestOptions`（选项 A，无废弃别名，类型放 rocket.rs；**`RequestOptions` 仅含 `timeout`**——reqwest 0.13.3 的 `RequestBuilder` 无 `connect_timeout` 方法，已验证本地 vendored 源码，`connect_timeout` 收敛为 client 级专属）；`Artful` 直接改为实例类型、删除静态版本；错误消息英文化、删 `cease()`、`InvalidUrl` 改名全部纳入；单 PR 交付。

## 1. 背景与问题

**现状**：artisan-http 0.13.1 是基于洋葱模型的 HTTP 客户端框架（artful PHP 框架的 Rust 移植），`Artful` 为纯静态类 + `OnceLock` 全局配置 + 全局 reqwest 单例 client，59 个测试全绿，clippy/fmt 干净。

**困境**（全部基于源码通读验证，非推断）：

1. **Content-Type 缺失**（bug 级）：默认插件链（StartPlugin → AddPayloadBodyPlugin → AddRadarPlugin → ParserPlugin）发出的 JSON body 不带 `Content-Type` 头。`AddRadarPlugin` 使用 `request_builder.body()`（`artisan-http/src/plugins/add_radar.rs:39-40`），reqwest 仅 `.json()` 才自动设置 Content-Type。对接严格校验头的服务端会失败；测试未暴露是因为无人断言该头。
2. **`connect_timeout` 死字段**：`HttpOptions::connect_timeout`（`rocket.rs:60`）定义并被模块文档与 `examples/config.rs` 演示，但全库无任何代码消费。
3. **全局 `Config.http.timeout/connect_timeout` 静默无效**：`build_client()`（`http.rs:34-46`）只消费 pool 与 user_agent；同时 `RocketConfig.http` 复用整个 `HttpOptions`，其 pool 字段在 per-request 层被静默忽略。一个类型混合 client/request 两种生命周期语义，两层各有死字段。
4. **静态全局配置的时序陷阱**：`GLOBAL_CONFIG: OnceLock<Config>`（`artful.rs:28`）+ `get_client()` 惰性初始化——若任何请求先于 `Artful::config()` 发生，client 按默认配置构建且后续配置静默失效；无法多配置共存；同一测试进程内配置不可隔离（现有测试套件因此没有任何配置生效断言）。
5. **API/错误卫生**：`ArtfulError` Display 消息为中文；`InvalidUrl` 吞掉 `request_builder.build()` 全部错误、语义过窄；`FlowCtrl::cease()` 与 `skip_rest()`（`flow_ctrl.rs:74-83`）行为逐行相同；`user_agent: Option<&'static str>` 限制动态 UA。
6. **工程残留**：facade `src/lib.rs:1` 的 `cfg_attr(docsrs, feature(doc_cfg))` 死代码（上个提交已移除全部 doc(cfg) 用法）；两个 Cargo.toml 缺 `readme/keywords/categories`；根 AGENTS.md 引用的 `docs/ARCHITECTURE.md` 路径错误（实际在 `artisan-http/docs/ARCHITECTURE.md`）且该文档多处过期（`artisan.rs`/`LoggerConfig`/`artisan_test.rs`、reqwest 0.12 依赖清单、全部静态 API 示例）；CI check job 缺 `--workspace`。

**目标约束**：

- **破坏性变更集中一次 minor 发版（0.14.0）**，不做任何兼容层。
- **`payload` 保持 `HashMap<String, Value>`**：深拷贝优化（Arc/Cow）收益/影响面不成比例，明确不做；仅做省一个中间 HashMap 的微优化。
- **每步三件套可验证**：`cargo fmt --all -- --check && cargo clippy --workspace --all-features -- -D warnings && cargo test --workspace --all-features`。
- **错误 Display 英文化，文档注释保持中文**。

## 2. 整体方案

**核心思路**：删除「隐式全局」，改为「实例优先」——配置与 client 在 `Artful` 构造时显式解析（fail-fast）；Content-Type 由 `Packer` 自描述、插件补头；`HttpOptions` 按 client/request 生命周期拆分，错误配置从运行时静默失效变为编译期暴露。

**架构图**：

```
应用层
  │  推荐：static ARTFUL: LazyLock<Artful>（std 1.80+，MSRV 1.85 满足）
  ▼
Artful::new() / Artful::with_config(config)      ← 唯一入口，构造时构建 client
  │  self.client 注入                                （失败返回 ArtfulError::ClientBuild）
  ▼
Rocket { params, payload, config, client, radar, destination_origin, destination, packer }
  │  client 由实例注入，插件不再直接依赖全局单例
  ▼
StartPlugin (merge_params_to_payload)
  → AddPayloadBodyPlugin (packer.pack + content_type 补头，仅缺失时)
  → AddRadarPlugin (rocket.client / headers / timeout / RequestBuildError)
  → ParserPlugin (rocket.client.execute + Direction 解析)
```

**文件结构变更**（A=新增，M=修改，共 36 个文件：src 14 + 文档/元数据 10 + tests 7 + examples 5）：

```
artisan-http/src/
├── rocket.rs         [M] HttpOptions → ClientOptions + RequestOptions（无别名）；
│                         RocketConfig.http: RequestOptions；Rocket 新增 pub client；
│                         merge_payload → merge_params_to_payload
├── http.rs           [M] 删公共 get_client；pub(crate) build_client（消费全部字段）+
│                         pub(crate) default_client（供直接构造 Rocket）
├── config.rs         [M] Config.http: HttpOptions → ClientOptions（import 同步替换）
├── artful.rs         [M] 整体重写：实例类型；删 GLOBAL_CONFIG 与全部静态方法
├── error.rs          [M] Display 全英文；InvalidUrl → RequestBuildError；新增 ClientBuild
├── flow_ctrl.rs      [M] 删 cease()
├── packer.rs         [M] 新增 content_type() 默认方法
├── packers/json.rs   [M] 覆写 content_type → "application/json"
├── plugins/start.rs  [M] 改用 merge_params_to_payload()
├── plugins/add_payload_body.rs [M] 打包后 or_insert 补 Content-Type
├── plugins/add_radar.rs        [M] rocket.client；fallback 分支补 CT；
│                                  RequestBuildError
├── plugins/parser.rs           [M] rocket.client.execute
├── lib.rs            [M] re-export：删 get_client/HttpOptions，加 ClientOptions/RequestOptions
src/lib.rs（仓库根 facade） [M] 删 cfg_attr(docsrs, ...) 死代码
Cargo.toml ×2        [M] 0.14.0；readme/keywords/categories
AGENTS.md ×2         [M] 修正过期项；按新 API 更新
README.md ×2         [M] 实例 API + LazyLock 推荐模式 + Content-Type 说明
CHANGELOG.md ×2      [M] 0.14.0 破坏性变更记录
artisan-http/docs/ARCHITECTURE.md [M] 成段重写过期章节（§2.2/2.3/2.5/3.1/3.2/4/5/6/7）
.github/workflows/coding-linter.yml [M] check job 加 --workspace
tests/ ×7            [M] 机械替换 + 新增 7 用例
examples/ ×5         [M] 实例 API 替换；config.rs 重写
```

不改动：`direction.rs`、`directions/`、`plugin.rs`、`shortcut.rs`、`packers/` 结构（json.rs 仅加 trait 方法覆写）、`.github/workflows/publish.yml`。

## 3. 详细设计

### 3.1 类型拆分（rocket.rs）

```rust
/// 客户端级选项：仅在构建 reqwest::Client 时生效（Artful::with_config）
#[derive(Debug, Clone, Default)]
pub struct ClientOptions {
    pub timeout: Option<u64>,            // client 级默认超时，请求级可覆盖（修复原全局死字段）
    pub connect_timeout: Option<u64>,
    pub pool_idle_timeout: Option<u64>,          // 默认 90
    pub pool_max_idle_per_host: Option<usize>,   // 默认 20
    pub user_agent: Option<String>,              // 由 &'static str 放宽
}

/// 请求级选项：仅单次请求生效（RocketConfig.http）
///
/// 仅含 timeout：reqwest 0.13.3 的 RequestBuilder 无 connect_timeout 方法
/// （connect_timeout 仅 ClientBuilder 提供），请求级连接超时无法表达。
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestOptions {
    pub timeout: Option<u64>,
}
```

- `Config.http: ClientOptions`（`config.rs`）；`RocketConfig.http: RequestOptions`；`HttpOptions` 直接删除（无废弃别名）。
- 字段名沿用（请求级仅保留 `timeout`；`connect_timeout` 为 client 级专属）。per-request 误设 pool 字段 → 编译错误（设计意图）。
- `build_client` 消费全部五个字段（`ClientBuilder::timeout` / `ClientBuilder::connect_timeout` 接线，均已验证存在于 reqwest 0.13.3）；请求级 `RequestBuilder::timeout` 自动覆盖 client 级 timeout，两层天然协作。
- 注意：`ClientOptions` 含 `String` 字段不再 `Copy`，`get_config().http` 按值传递处改为 `.clone()`。

### 3.2 Artful 实例化（artful.rs 整体重写）

```rust
#[derive(Debug, Clone)]
pub struct Artful { config: Config, client: reqwest::Client }

impl Artful {
    pub fn new() -> Result<Self>;                          // 默认配置
    pub fn with_config(config: Config) -> Result<Self>;    // 构造即构建 client，失败 ClientBuild
    pub fn config(&self) -> &Config;
    pub fn client(&self) -> &reqwest::Client;
    pub async fn artful(&self, params, plugins) -> Result<Destination>;
    pub async fn shortcut<S: Shortcut>(&self, s, params) -> Result<Destination>;
    pub async fn raw(&self, request) -> Result<reqwest::Response>;
}
```

- 删除 `GLOBAL_CONFIG`、`Artful::config()/get_config()/has_config()`。E0592 约束随之消失（静态方法不再存在，实例方法可自然使用这些名字）。
- `Rocket` 新增 `pub client: reqwest::Client`：`Rocket::new` 默认 `crate::http::default_client().clone()`（reqwest::Client 内部 Arc，clone 廉价且共享连接池）；`Artful::artful` 注入 `self.client.clone()`。
- `AddRadarPlugin`/`ParserPlugin` 从 `get_client()` 改为 `rocket.client`。
- `Rocket` 不可字面量构造（`params` 私有），新增 pub 字段无破坏面。
- `Config.extra` 语义不变；需要额外配置的插件在自身结构体字段持有（`Arc<dyn Plugin>` 为用户类型）。
- **应用层 LazyLock 模式（README 推荐）**：`Artful: Send + Sync + 'static` 成立（reqwest::Client 为 Arc 内核），`static ARTFUL: LazyLock<Artful> = LazyLock::new(|| Artful::new().expect(...))`；首访初始化支持读环境变量；panic 仅发生于 TLS 后端初始化失败等极端场景（与 reqwest `Client::new()` panic 语义一致）；零 panic 变体为 `LazyLock<Result<Artful, ArtfulError>>`。

### 3.3 Packer 自描述 Content-Type

```rust
// packer.rs —— 带默认实现，对现有实现者非破坏
fn content_type(&self) -> Option<&'static str> { None }
// packers/json.rs
fn content_type(&self) -> Option<&'static str> { Some("application/json") }
```

- `AddPayloadBodyPlugin` 打包后：`rocket.config.headers.entry("Content-Type".to_string()).or_insert_with(|| ct.to_string())`——仅缺失时补，用户手动设置永不覆盖。补头语句置于打包分支（`body.is_none() && !payload.is_empty()`）内部、body 赋值之后。
- `AddRadarPlugin` fallback pack 分支（body 为 None 且 payload 非空）：`!rocket.config.headers.contains_key("Content-Type")` 时补头；**补头落点为 `request_builder`**（`request_builder = request_builder.header("Content-Type", ct)`）——该分支位于 headers 遍历（add_radar.rs L35-37）之后，写回 `config.headers` 不会被应用到请求；不改变现有遍历顺序。
- 边界：手动 `set_body()` 不经 packer，不自动补；自定义 Packer（如 form）声明自己的 MIME 即自动生效。
- 已知简化：`config.headers` 为 `HashMap<String, String>`，键匹配区分大小写（现状如此，文档标注；不在本次统一）。

### 3.4 错误与 API 清理（error.rs / flow_ctrl.rs / plugins）

- `ArtfulError` Display 全英文（`"HTTP request failed: {0}"` 等 11 个现有变体；改名与新增后共 12 个），变体名除下述外不变，程序化匹配不受影响。
- `InvalidUrl` → `RequestBuildError { source: reqwest::Error }`，消息 `"failed to build HTTP request: {source}"`——涵盖 `build()` 全部失败而非仅 URL。
- 新增 `ClientBuild { source: reqwest::Error }`，消息 `"failed to build HTTP client: {source}"`。**不可写 `#[from]`**：`reqwest::Error` 的 `#[from]` 已被 `RequestFailed` 占用，thiserror 重复 `#[from]` 会编译报错；`Artful::with_config` 中显式构造。
- 删 `FlowCtrl::cease()`（与 `skip_rest()` 逐行相同）；`flow_ctrl_test.rs` 对应用例改用 `skip_rest`。
- `StartPlugin`：`rocket.merge_payload(rocket.get_params().clone())` → `rocket.merge_params_to_payload()`（rocket.rs 新方法，内部直接字段访问 `self.params.iter()` + `self.payload.insert`，借用检查器允许字段级 disjoint borrow；省一个中间 HashMap 分配）。原 `merge_payload(HashMap)` 删除（src 唯一调用方是 StartPlugin；`tests/rocket_test.rs::test_rocket_merge_payload` 的直接调用随测试迁移改写为 `merge_params_to_payload` 断言）。

### 3.5 测试计划

**改写**（机械替换，行号基于 0.13.1 源码）：

| 文件 | 改动 |
|------|------|
| `tests/artful_test.rs` | `Artful::artful` 9 处（artful_test.rs L49/175/254/286/332/353/370 + integration_test.rs L48/79）→ `Artful::new().unwrap()` 实例调用；`get_config/has_config` 2 用例（L77-90）重写为实例 `config()` 断言；`raw` 2 用例（L93-130）改实例 client |
| `tests/flow_ctrl_test.rs` | 5 处真实 `cease()` 调用点所在用例改 `skip_rest` 并全部保留（L135/150/217/245/277，维持 65 计数；`test_flow_ctrl_cease` L46 未实际调用 `cease()`，不动） |
| `tests/rocket_test.rs` | `HttpOptions` → `RequestOptions`/`ClientOptions` 类型名替换（实际位置 L1/30/37/185）；`test_rocket_merge_payload`（L56-64）改写为 `merge_params_to_payload` 断言 |
| `tests/integration_test.rs` | L48/L79 两处实例化替换 |
| `tests/packer_test.rs`、`direction_test.rs`、`shortcut_test.rs` | 核对是否受 trait 方法新增影响（`content_type` 带默认实现，预期零改动） |

**新增**（7 用例）：

| 用例 | 验证点 |
|------|--------|
| `default_chain_sets_content_type` | wiremock `header("content-type", "application/json")` matcher 命中；params 非空（空 payload 不打包、不补 CT） |
| `manual_content_type_not_overridden` | 用户先设 CT 后，mock 收到用户值；CT 须经前置插件（照抄既有 MethodUrlPlugin 模式）或 FlowCtrl 直控 Rocket 设置——`Artful::artful` 入口无法预置 Rocket 字段 |
| `custom_packer_content_type` | 自定义 Packer 声明的 MIME 生效；前置插件替换 packer 或 FlowCtrl 直控 Rocket 赋 `rocket.packer` |
| `client_timeout_takes_effect` | `Artful::with_config(timeout: 1s)` + mock 延迟 2s → `RequestFailed` |
| `artful_new_and_accessors` | `with_config` 正常/失败路径；`client()`/`config()` |
| `merge_params_to_payload` | params → payload 合且 params 不变 |
| `fallback_branch_sets_content_type` | 无 `AddPayloadBodyPlugin` 的链（StartPlugin + URL 插件 + AddRadarPlugin + ParserPlugin）：payload 非空且 body 为 None，走 AddRadarPlugin fallback 打包分支补 CT，matcher 断言 `application/json`（覆盖 §3.3 第二补头点，补头落点为 request_builder） |

用例归属固定，不新建测试文件（维持 tests ×7）：CT 四例与 `client_timeout_takes_effect` 入 `integration_test.rs`；`artful_new_and_accessors` 入 `artful_test.rs`；`merge_params_to_payload` 入 `rocket_test.rs`（替代原 `test_rocket_merge_payload`）。

### 3.6 文档与元数据同步

- **Cargo.toml**：`readme = "README.md"`、`keywords = ["http", "client", "middleware", "plugin"]`、`categories = ["network-programming", "web-programming::http-client"]`、版本 0.14.0。
- **README ×2**：快速开始改实例 API；新增「全局单例（LazyLock）」小节（含零 panic 变体）；Content-Type 自动补头说明；版本引用 `~0.13.1` → `~0.14.0`（根 README 3 处、artisan-http README 1 处）；测试计数 59 → 65（两份 README 各 1 处）；修复 artisan-http README L234 架构文档断链（`../docs/ARCHITECTURE.md` → `docs/ARCHITECTURE.md`，实际文件在 `artisan-http/docs/`）。
- **AGENTS.md（根）**：References 中 `docs/ARCHITECTURE.md` → `artisan-http/docs/ARCHITECTURE.md`；feature 用法示例同步。
- **artisan-http/AGENTS.md**：`src/artisan.rs` → `src/artful.rs`；Shortcut 描述去掉 `Default` bound；测试数量更新；按新 API 重写 Key Types/Patterns。
- **artisan-http/docs/ARCHITECTURE.md**（969 行，成段重写）：§2.2 类型拆分、§2.3 Config 实例化（含 LazyLock）、§2.5 删 cease、§3.1 Artful 主入口、§3.2 HTTP 客户端（build_client/default_client + 全字段接线）、§4 插件（merge_params_to_payload/CT/RequestBuildError/rocket.client）、§5 使用示例全量实例化、§6 模块结构（artful.rs/artful_test.rs/删 LoggerConfig）、§7 依赖清单对齐真实 Cargo.toml（reqwest 0.13、无 tracing）、§8 补记 0.14.0。
- **CHANGELOG ×2**：0.14.0 条目，破坏性变更逐项列出。
- **CI**：coding-linter.yml check job `cargo check --all-features` → `cargo check --workspace --all-features`。

## 4. 推进策略

单 PR，4 步串行（强耦合：类型层被所有层消费，无可并行波次），每步跑三件套，红灯不进下一步；第 1 步 src 与测试同批完成后才 commit——测试引用被删旧 API，src 单独提交必然测试编译红，违反仓库 AGENTS.md 三件套强制规则：

1. **src 演进与测试迁移**：13 个 src 文件 + 7 个测试文件改写 + 7 个新用例（三件套全绿后一次 commit）。
2. **示例迁移**：5 个 examples（config.rs 重写）。
3. **文档与元数据**：Cargo.toml ×2、README ×2、AGENTS ×2、ARCHITECTURE.md、CHANGELOG ×2、CI、facade lib.rs（仓库根 `src/lib.rs`）。
4. **发布就绪**：`cargo package --list`、`cargo doc --no-deps`、全量三件套。

发版：tag `artisan-http/v0.14.0` → publish artisan-http → tag `artisan/v0.14.0` → publish artisan（既有 workflow 自动化）。**打 tag 与 publish 为独立动作，实施前单独向用户确认。**

回滚：发版前 `git revert` 单 PR（或按步 commit 定位）；已发布版本由 CHANGELOG 引导升级。

## 5. 风险与对策

| 风险 | 严重度 | 对策 |
|------|--------|------|
| 默认链请求带上 Content-Type 后，个别依赖"无 CT"的服务端行为变化 | 中 | 仅缺失时补；README/CHANGELOG 显著标注；用户显式设置任意 CT 即可覆盖 |
| 已配置全局 timeout 的用户升级后请求开始超时（原为无效配置） | 中 | 本就是配置意图；CHANGELOG 置顶提示复核该字段 |
| 不考虑兼容性导致下游（若有）编译失败 | 中 | CHANGELOG 完整记录破坏项；0.x minor 发版符合 semver 预期 |
| 单 PR 变更面较大（36 文件），定位回归困难 | 低 | 4 步（另有基线验证步）各自跑三件套，按步 commit，出问题按 commit 定位/revert |
| `HashMap` 键大小写导致 CT 判重不严（用户写 `content-type` 小写时会补出第二个 CT 头，reqwest append 语义下重复发送） | 低 | 文档标注现状与后果；后续需要时再统一 |
| client 级 connect_timeout 网络行为在 CI 难稳定断言 | 低 | 以消除死字段 + 类型编译期暴露为准，不做脆弱的网络断言；请求级不提供该选项（reqwest 0.13.3 `RequestBuilder` 无此方法） |
| `ClientOptions` 失去 `Copy` 引发内部传递处编译错 | 低 | 仅 `Artful::with_config`（artful.rs）一处按值传递（`build_client(config.http.clone())`）；clippy 兜底 |

## 附录 A：契约验证声明

本方案全部契约标注为「已验证（读过源码）」，无「推断（未实测）」项。验证方式：通读 `artisan-http/src/` 全部 13 个源文件（含 config.rs）、7 个测试文件、5 个示例、2 个 Cargo.toml、2 组 workflow、README/AGENTS ×2、ARCHITECTURE.md，并运行 `cargo clippy`/`cargo test`（0.13.1 基线 59 个测试 + 5 个 doctest 全绿）、grep 确认 `connect_timeout`/`Content-Type`/`get_client`/`Artful::` 全部调用点。reqwest 0.13.3 API 已对照本地 vendored 源码（`~/.cargo/registry/.../reqwest-0.13.3`）验证：`RequestBuilder` 有 `timeout`、无 `connect_timeout`（后者仅 `ClientBuilder` 提供）；`.body()` 不设置 Content-Type；实现期如遇偏差按 Executor rules 分级处理。

## 附录 B：应用层 LazyLock 用法

```rust
use std::sync::LazyLock;

// 推荐：全局单例（首访初始化，可读环境变量）
static ARTFUL: LazyLock<Artful> = LazyLock::new(|| {
    Artful::with_config(load_config()).expect("failed to build Artful client")
});

// 零 panic 变体（ArtfulError 非 Clone，调用点需 map_err 转移错误所有权）
static ARTFUL: LazyLock<Result<Artful, ArtfulError>> = LazyLock::new(|| Artful::with_config(load_config()));
// 调用点：
let artful = ARTFUL
    .as_ref()
    .map_err(|e| ArtfulError::Other(format!("Artful init failed: {e}")))?;
artful.artful(params, plugins).await?;

// 多实例：不同渠道各一个 static，连接池独立
static ALIPAY: LazyLock<Artful> = ...;
static WECHAT: LazyLock<Artful> = ...;
```
