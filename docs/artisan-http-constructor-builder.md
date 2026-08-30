# 技术设计：Artful 构造器改造——with_client_builder 重命名 + Builder 模式

> **时间**：2026-08-30
> **作者**：GLM-5.3-Flash + yansongda
> **状态**：经过人工审核确认

## 1. 背景与问题

**现状**：`Artful`（`artisan-http/src/artful.rs`）现有四个构造器：`new()`、`with_config(config)`、`with_builder(config, customize)`、`with_client(config, client)`。0.14.0（2026-08-29）刚完成实例化改造，确立 fail-fast 构建约定。

**困境**：

1. `with_builder` 命名歧义：其中 "builder" 指向 reqwest 的 `ClientBuilder`，与本类型自己的 builder 模式术语撞车，初次接触者会误解为"Artful 的 builder"（评审已确认）。
2. 无统一构建入口：四条构造路径相互独立，缺少一个可逐步累积选项（config → 回调 → client 注入）的链式入口；调用方只能在四个静态方法间"一次选对"。

**目标**：

- **命名消歧**：`with_builder` 更名为 `with_client_builder`，明示其操作对象是 client 构建器。
- **统一入口**：新增 `Artful::builder()` 链式构建器，覆盖现有全部构造场景，且不增加第三条漂移的构建路径。
- **约束**：不引入任何新依赖（依赖极简是本 crate 定位）；MSRV 不设硬上限，必要时可升级，但本方案无需升级（手写 builder 仅用 Rust 1.35 即稳定的能力）；0.x minor bump 承载 breaking。

## 2. 整体方案

**核心思路：手写 `ArtfulBuilder` 作为唯一构建事实来源，旧构造器全部保留为薄包装；`with_builder` 直接删除（跟随 0.14.0 "不做兼容层"惯例，不加 `#[deprecated]`）。**

选型依据（research 已验证，来源见附录）：

| 方案 | 结论 | 理由 |
|------|------|------|
| bon 3.10 derive | ❌ 可行但不推荐 | MSRV 解除后技术上可行（`Box<dyn FnOnce>` 字段 bon 原生支持），但：① 本 builder 仅 3 个字段，derive 收益趋近于零，手写约 100 行是一次性成本；② 引入 proc-macro 依赖树（bon + syn 全家桶），所有下游 crate 编译时都要付出构建时间与供应链审计代价，而 `ArtfulBuilder` 类型签名中不会出现任何 bon 类型，等于白扛依赖；③ bon 生成的 setter 文档是机械模板，本项目需要在 doc 注释里写清 `.client()` 注入后 `config.http` 与 `customize` 均不参与构建这类优先级语义，手写才可控；④ reqwest/hyper/sqlx/ureq/rustls 级别的基础库无一例外手写 builder，这是基础设施 crate 的一致惯例 |
| typed-builder / derive_builder | ❌ | derive 无法自然承载 `FnOnce` 装箱 setter；derive_builder 近两年未发版 |
| **手写 builder（reqwest/hyper/sqlx/ureq 同款惯例）** | ✅ | 零新依赖；doc 注释与优先级语义完全可控；约 100 行；MSRV 1.85 维持不动，`Cargo.toml` 零改动 |

bon 解决的是"字段多、setter 逻辑复杂"的规模问题，本场景不在这个规模上。若未来 `Config`/`ClientOptions` 需要自己的 builder 且字段膨胀到两位数，届时再引入 bon（MSRV 升 1.88）。

**架构关系**：

```
便捷入口                     定制入口                        统一入口
────────────                ─────────────                   ─────────────
Artful::new()               Artful::with_config(c)          Artful::builder()
  │                            │                               │ .config(c)
  │                            ▼                               │ .customize(f)
  │                    with_client_builder(c, |b| b)            │ .client(cl)
  │                            │                               ▼
  │                            │                         ArtfulBuilder::build()
  │                            │                               │
  └──────────┬─────────────────┴───────────────────────────────┘
             ▼
   构建 reqwest::Client（fail-fast，client 注入路径除外）
             ▼
   Artful { config, client }
```

**文件结构**（变更后）：

```
artisan-http/
├── src/artful.rs            # ✏️ 重命名 with_builder → with_client_builder；新增 ArtfulBuilder；文档与单测同步
├── src/lib.rs               # ✏️ re-export ArtfulBuilder；Send 断言加一行
├── tests/artful_test.rs     # ✏️ 3 个 with_builder 测试更名；新增 builder 集成测试
├── README.md                # ✏️ 构造器示例段（与 zh-CN 成对）
├── README.zh-CN.md          # ✏️ 同上
├── AGENTS.md                # ✏️ 37、92 行构造器描述
├── docs/ARCHITECTURE.md     # ✏️ 444、515-517、800 行（1061 为 v0.14.0 历史 checklist，豁免不改）
└── CHANGELOG.md             # ✏️ 新增 [Unreleased] 条目（Breaking + Added）
不动：examples/（未引用旧名）、docs/artisan-http-0.14.0*（历史文档）、CHANGELOG 历史条目（不追溯）、
     根包 artisan（根 src/lib.rs，整包 re-export 无需改）、两处 Cargo.toml 版本号（留给发布流程）
```

## 3. 详细设计

### 3.1 `ArtfulBuilder` 数据结构（新增于 `artful.rs`，与 `Artful` 同文件内聚）

| 字段 | 类型 | 可选 | 语义 |
|------|------|------|------|
| `config` | `Config` | ✅ | 默认 `Config::default()`；覆盖式 setter（后写覆盖先写） |
| `customize` | `Option<Box<dyn FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder + Send + 'static>>` | ✅ | 内部装箱；对外 setter 保持泛型，但较 `with_client_builder` 参数**多 `Send + 'static` 约束**（装箱所需），捕获非 `Send`/非 `'static` 值（如 `Rc`）的闭包只能走 `with_client_builder`，能力不对等须在 doc 中披露 |
| `client` | `Option<reqwest::Client>` | ✅ | 外部注入现成 client；最高优先级 |

类型契约说明：

- `Box<dyn FnOnce>` 的 `Send + 'static` 约束：builder 可能跨线程传递到 `build()` 调用点；`Box<dyn FnOnce>` 直接调用自 Rust 1.35 稳定，MSRV 1.85 无碍（已验证，Rust 语言特性）。
- **线程安全只断言 `Send`，不断言 `Sync`**：`Box<dyn FnOnce + Send>` 本身非 `Sync`，`ArtfulBuilder` 因此是 `Send` 而非 `Sync`。builder 是一次性消费对象，不存在多线程共享同一 builder 的场景，`Sync` 无需求。`src/lib.rs` 既有 `assert_send_sync` 辅助不适用，需新增 `assert_send` 辅助（或等价手段）单独断言。
- `#[derive(Debug)]` 不可用（`Box<dyn FnOnce>` 无 `Debug`），手写 `impl Debug`：打印 `config` 与 `client` 是否注入，`customize` 打印 `Some(_)`/`None`。

### 3.2 API 设计（setter 命名跟随 reqwest 惯例：consume-self + 裸字段名）

| 方法 | 签名要点 | 说明 |
|------|----------|------|
| `Artful::builder()` | `-> ArtfulBuilder` | 统一入口；`impl Default for ArtfulBuilder` 同步提供 |
| `.config(Config)` | `(self, Config) -> Self` | 不设置则 `Config::default()`；覆盖式（后写覆盖先写） |
| `.customize(F)` | `(self, F) -> Self where F: FnOnce(ClientBuilder) -> ClientBuilder + Send + 'static` | 对应 `with_client_builder` 的回调参数；覆盖式；较 `with_client_builder` 多 `Send + 'static` 约束 |
| `.client(reqwest::Client)` | `(self, Client) -> Self` | 对应 `with_client`；设置后 `config.http` 与 `customize` 均不参与构建（与 `with_client` 语义一致） |
| `.build()` | `(self) -> Result<Artful>` | 构建优先级：`client` 注入 > `config + customize` 构建 |

`build()` 构建优先级（伪代码）：

```
build():
    if client 已注入:   return Artful { config, client }              # 不构建、不校验
    b = build_builder(config.http)                                    # 框架默认值兜底
    b = customize.unwrap_or(identity)(b)                              # 叠加回调
    return Artful { config, client: b.build()? }                      # fail-fast
```

**防漂移设计**：builder 不复制 `Config` 的任何字段——`config` 整体持有、`build()` 时整体交给现有构建函数（与 ureq 3 "builder 只是 options 的填表器"、rustls "builder 只负责构造起点"的惯例一致），`Config` 仍保持公开字段 + `Default` 的 serde 友好形态，两条路径零重叠。

### 3.3 与现有构造器的关系（全部保留，职责重述）

| 现有构造器 | 改造后 | 实现 |
|------------|--------|------|
| `new()` | 保留 | 委托 `with_config`（现状不变） |
| `with_config(config)` | 保留 | 委托 `with_client_builder(config, \|b\| b)` |
| `with_builder` | **删除**，新增 `with_client_builder` 同签名 | 逻辑原样，仅更名 |
| `with_client(config, client)` | 保留 | 与 builder 的 `.client()` 路径共享语义 |
| `Artful::builder()` | **新增** | `build()` 委托上述路径，不新造构建逻辑 |

### 3.4 兼容性设计

- **0.x semver**：minor bump（0.14.0 → 0.15.0，发布时执行）允许破坏性变更；重命名直接删除旧名，**不加 `#[deprecated]`**——跟随本仓库 0.14.0 CHANGELOG 已确立的"不做兼容层"惯例，且项目尚处早期、无存量用户负担。
- **facade**：仓库根包 `artisan`（源码在根 `src/lib.rs`，不存在 `artisan/` 子目录）为 `pub use artisan_http as http` 整包 re-export，`ArtfulBuilder` 随之自动可用，零改动。
- **导出位置**：`pub use artful::{Artful, ArtfulBuilder}`（`src/lib.rs` 既有平铺 re-export 风格）。

### 3.5 文档同步范围（逐一列出，防漏改）

| 文件 | 位置 | 改动 |
|------|------|------|
| `src/artful.rs` | 模块头方法列表（第 9 行）、doc 注释 | 更名 + 新增 builder 说明 |
| `README.md` / `README.zh-CN.md` | 149-164 行构造器示例段 | 更名 + 补 builder 示例（成对同步） |
| `AGENTS.md` | 37、92 行 | 更名 + 补 builder |
| `docs/ARCHITECTURE.md` | 444、515-517、800 行（1061 为 v0.14.0 历史 checklist 豁免） | 更名 + 补 builder |
| `CHANGELOG.md` | 新增 `[Unreleased]` 段 | `**BREAKING**` 重命名条目 + `Added` builder 条目；历史条目不追溯 |

## 4. 推进策略

**阶段划分**（单 PR，三个阶段串行）：

- **Phase A 重命名**：`artful.rs` + 全部引用点（含文档引用处）机械替换 → 验证点：`cargo check --workspace --all-features` 通过、`grep -rn "with_builder"` 全仓仅剩豁免清单（CHANGELOG 0.14.0 条目、docs/artisan-http-0.14.0*、docs/evidence/、docs/implementation/、本设计文档 docs/artisan-http-constructor-builder.md、docs/ARCHITECTURE.md:1061）
- **Phase B builder**：新增 `ArtfulBuilder` + 单测 + 集成测试 → 验证点：`cargo test --workspace --all-features` 通过
- **Phase C 文档**：README 双语 / AGENTS / ARCHITECTURE / CHANGELOG → 验证点：README 中英文段落一一对应

**回滚**：单 commit 或按阶段分 commit，`git revert <commit>` 即回滚；无配置、无数据、无线上环境，无紧急回滚通道需求。

## 5. 风险与对策

| 风险 | 严重度 | 对策 |
|------|--------|------|
| ARCHITECTURE.md / README 漏改旧名，文档与代码漂移 | 中 | 验收含 `grep -rn "with_builder"` 断言（Phase A 验证点）；执行 todo 的 References 附全部行号清单（行号为 0.14.0 基线，执行时以内容定位为准） |
| README 双语漏改一侧 | 中 | 双语成对政策已写入 AGENTS.md；同一 todo 内成对修改 + grep 断言 |
| `with_client` 与 `with_client_builder` 名字接近导致误用 | 低 | 两者 doc 注释交叉引用（"需定制构建 → `with_client_builder`；注入现成 client → `with_client`"） |
| `.client()` 注入后误以为 `config.http` 生效（既有语义坑） | 低 | doc 注释沿用 `with_client` 既有警示文案，builder 上同样标注 |
| `Box<dyn FnOnce>` 每次 build 一次堆分配 | 低 | 构造一次性路径，开销可忽略；不优化 |
| CHANGELOG 历史条目仍写旧名造成困惑 | 低 | 不追溯修改历史条目；`[Unreleased]` 条目中注明"原 `with_builder`" |

## 6. 监控与可观测性

不适用（库代码，无线上运行时）。以 CI 三件套为验收门禁：`cargo fmt --all -- --check`、`cargo clippy --workspace -- -D warnings`、`cargo test --workspace --all-features`。

## 附录：选型来源（research agent 已验证，2026-08-30）

- bon 3.10 MSRV 1.88 变更：github.com/elastio/bon/releases；bon reference：bon-rs.com/reference/builder
- 手写 builder 惯例：reqwest `ClientBuilder`、sqlx `PgPoolOptions`、ureq 3 `ConfigBuilder`、rustls `ClientConfig`（consume-self + 裸字段名 + `build() -> Result`）
- `Box<dyn FnOnce>` 装箱先例：tauri `Builder::setup`（对外泛型 setter，内部装箱）
- 双路径防漂移：ureq 3（builder 产物即 Config）、sqlx（options 与 builder 字段零重叠）、rustls（builder 只负责构造起点，后续走公开字段）
- Cargo semver（0.x minor 允许破坏）：doc.rust-lang.org/cargo/reference/semver.html
