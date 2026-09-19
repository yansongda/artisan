# 技术设计：artisan-http 0.18.0 —— Packer 数据域迁移 `serde_json::Map` + typed 便利层

> **时间**：2026-09-12
> **作者**：DeepSeek-V4-Flash + yansongda
> **状态**：已经用户批准（2026-09-12 对话确认），待 plan-reviewer 独立审查
> **关联文档**：`docs/artisan-http-0.17.0-align-artful.md`（0.17.0 为 `Packer::pack/unpack` 新增 `params` 形参，本方案在其上继续演进）；GitHub issue #15（Packer API 演进讨论，方案 B + D 的源头）
> **修订记录**：2026-09-12 初版经对话呈现与用户确认；同日应用户要求以 subagent 调研 yansongda/pay（32 处 `setPacker` 调用）与 yansongda/artful（`Rocket::setPacker` 存类名字符串、容器解析）源码，确认「每请求选择 packer」为真实核心场景、「链中途二次替换」无真实用例，据此新增 3.5 节（能力语义收窄）并维持 object-safe 约束；决策点定稿：删除 `From<HashMap> for Rocket`、`Config.extra` 不动、B+D 全量 0.18.0、新增 `DestinationMismatch` 变体。同日 plan-reviewer 初审（结论：拒绝执行，1 BLOCKER + 2 MAJOR + 9 MINOR），逐条源码复核全部属实（零驳回）后修订，用户已确认：B1 补 serde 依赖（dependencies 无 derive 仅 bound + dev-dependencies derive；serde 已在依赖树中为 serde_json 传递依赖）；M1 改 Task 1 验收为 `cargo test -p artisan-http --lib`（覆盖 cfg(test) 模块）；M2 修 `artisan-http/docs/ARCHITECTURE.md` 路径并收窄 F1 grep 范围；MINOR 逐条采纳（serde_json 版本标注 1.0.149、行号修正、`DestinationMismatch` Display 文案统一、xml `btree_map::Entry` 迁移、README 逐处处置、根 Cargo 依赖写法、JsonPacker 键序测试改字符串断言、CI 四步表述修正、依赖矩阵补 Task 5→Task 3）。同日复审（结论：修改后执行，0 BLOCKER + 5 MAJOR + 6 MINOR）逐条采纳：M-A Task 1 验收 grep 豁免 `config.rs` 的 `extra`、M-B F4 grep 收窄至 src/、M-C ARCHITECTURE 验收白名单化（headers/extra/描述文字保留）、M-D Task 2 删失实表述、M-E Scope Display 文案统一；MINOR 全部采纳（raw string 转义、基线数字统一、行号修正、验收条款验证力、`test_rocket_from_hashmap` 随删）。同日第 2 轮复审（结论：修改后执行，0 BLOCKER + 1 MAJOR + 5 MINOR），用户裁决 MAJOR-1 按 b) 方案：examples/typed.rs 遵循既有惯例（不引入 wiremock，query_xml_packer.rs L20 明示，7 个 examples 全部 httpbin.org 真实服务）演示 artful_as 成功路径；MINOR 全部采纳（基线口径 214 = 151 lib + 58 integration + 5 doctest、双语粗检改 artful_as 锚点、CI doctest 已覆盖的表述修正、query.rs 注释行号 L13-14/L56/L69 + 验收锚点、错误示范标注删除）。

---

## 1. 背景与问题

### 现状

artisan-http 0.17.0（2026-09-01）的 `Packer` trait 签名（`packer.rs` L25-38）：

```rust
fn pack(&self, data: &HashMap<String, Value>, params: &HashMap<String, Value>) -> Result<String>;
fn unpack(&self, data: &str, params: &HashMap<String, Value>) -> Result<Value>;
```

`HashMap<String, Value>` 遍布数据域全链：`Rocket.params/payload`、`Artful::artful/shortcut` 参数、`Shortcut::get_plugins`、公开函数 `filter_params`（`lib.rs` L101）、`Event::ArtfulStart.params`（`event.rs` L74）；src 内约 40 处 + 7 个 examples + 6 个测试文件。

### 困境

1. **类型不对称**：`unpack` 返回 `Value`（其 `Object` 变体内部就是 `serde_json::Map`），而 `pack` 收 `HashMap`——`value.as_object()` 拿到的 `&Map` 必须 clone 转换才能回传 pack，请求/响应往返各一次无谓转换。
2. **键序确定性靠手动排序**：`QueryPacker`/`XmlPacker` pack 各自 `keys.sort_unstable()`（注释明写"HashMap 无序，排序保证确定性"）；`JsonPacker` 对 `HashMap` 序列化输出键序随机。确定性输出是网关签名场景的硬需求，却只能靠实现自觉。
3. **嵌套与顶层行为分裂**：嵌套 `Value::Object` 内部已是 BTreeMap（天然有序），顶层却是无序 HashMap。
4. **强类型 DX 缺失**：响应 `Value` → 业务结构体需手写 `match Destination::Json` + `serde_json::from_value` + 错误转换三段样板。

### 调研约束（2026-09-12，subagent 已 clone 并读 yansongda/pay 与 yansongda/artful 源码）

- **「每请求选择 packer」是真实核心场景**：pay 中 32 处 `setPacker` 调用，全部在请求链渠道插件中按请求设定（Unipay→`XmlPacker`、Open→`QueryPacker`、Allinpay→`JsonPacker` 等），后续 `AddPayloadBodyPlugin`/`ParserPlugin` 经 `get_packer()` 从容器解析消费。
- **「链中途二次替换 packer」无真实用例**：pay 全库每文件恰 1 次 `setPacker`、无二次覆盖；链结构证明「先定死、后只读」（`Shortcut/Unipay/CancelShortcut.php` L94-101：`setPacker → AddPayloadSignature → AddPayloadBody(pack) → AddRadar → VerifySignature → Parser(unpack)`）。
- **结论**：`Arc<dyn Packer>` 保留、object-safe 约束成立、trait 泛型化（`pack<T: Serialize>`）维持「不采纳」——「每请求动态选择」必须有间接层，泛型方法杀 object-safe，枚举亦无法承载自定义 packer（泛型方法无法经间接层表达）。

### 目标

- **两侧同构**：`pack`/`unpack` 数据域统一 `serde_json::Map<String, Value>`
- **键序由类型保证**：删除全部手动排序，顶层与嵌套行为一致
- **typed 便利层**：`pack_typed`/`unpack_typed`/`Destination::into_json`/`artful_as`，非破坏（Added）
- **BREAKING 单窗口**：0.18.0 一次性收齐，不残留两代类型
- **`unpack` 返回值保持 `Value`**（可表达数组根/标量根/XML 空对象语义，收窄为 `Map` 属语义变化，不采纳）
- **能力语义收窄**：packer 定位为「请求级配置，链早期设定」，不承诺链中途替换

## 2. 整体方案

### 核心思路

**数据域整体从 `HashMap<String, Value>` 迁移到 `serde_json::Map<String, Value>`（无 `preserve_order` 特性时 = BTreeMap 后端），`Packer` trait 签名同步；trait 外新增 typed 便利层，把强类型痛点收敛在入口/出口。**

### 架构图

```
调用方: Map<String, Value>
   │
   ▼
Artful::artful(params: Map) ──► Rocket.params/payload: Map
   │                                  │
   │                     StartPlugin  │ merge
   │                                  ▼
   │                        filter_params(Map)  [剔除 _ 前缀 + null]
   │                                  │
   │                    AddPayloadBodyPlugin: packer.pack(&Map)──► 请求体 String
   │                                  ▲
   │                                  │ unpack(data, params) ──► Value(内含 Map)
   │                    JsonDirection ◄┘
   │                                  │
   ▼                                  ▼
Destination ── into_json() ──► Value ──► artful_as::<T> / unpack_typed::<T> ──► 业务结构体
```

### 文件结构

| 类型 | 文件 | 变更 |
|---|---|---|
| 修改 | `packer.rs` | trait 签名 `HashMap`→`Map`；**新增** `pack_typed`/`unpack_typed` 自由函数 |
| 修改 | `rocket.rs` | `params`/`payload`/`new`/`get_params` → `Map`；**删除** `From<HashMap> for Rocket`（用户已确认） |
| 修改 | `artful.rs` | `artful`/`shortcut` 参数 → `Map`；**新增** `artful_as::<T>` |
| 修改 | `shortcut.rs` / `lib.rs` / `event.rs` | `get_plugins`/`filter_params`/`ArtfulStart.params` → `Map` |
| 修改 | `packers/{json,query,xml}.rs` | 签名迁移；Query/Xml 删 `sort_unstable`；Xml 内部 `BTreeMap`→`Map` |
| 修改 | `plugins/*`、`directions/*`、`ignite.rs`、`flow_ctrl.rs` | 调用点适配（无逻辑变化） |
| 修改 | `direction.rs` | `Destination` **新增** `into_json()` 方法 |
| 修改 | `error.rs` | **新增** `DestinationMismatch` 变体 |
| 修改 | `config.rs` | **不动**（`Config.extra` 保留 `HashMap<String, Value>`） |
| 修改 | `Cargo.toml`×2 | 0.18.0 版本；artisan-http 依赖新增 `serde`（正依赖无 derive 仅 bound + dev-dependencies derive，见 §3.3） |
| 修改 | `CHANGELOG.md`×2、`README.md`×2（artisan-http）、`artisan-http/docs/ARCHITECTURE.md` | 0.18.0 版本与文档同步（含 ARCHITECTURE 数据域代码示例迁移） |
| 修改 | tests×6（integration/artful/event/parser/shortcut/direction） | 迁移 + typed 新用例；`ReplacePackerPlugin` 语义重构为「链早期设定」 |
| 修改 | examples×7 + 新增 `examples/typed.rs` | 迁移 + typed 演示 |

## 3. 详细设计

### 3.1 Packer trait 新签名（已验证：当前签名 `packer.rs:25-38`）

```rust
pub trait Packer: Send + Sync + std::fmt::Debug {
    fn pack(&self, data: &Map<String, Value>, params: &Map<String, Value>) -> Result<String>;
    fn unpack(&self, data: &str, params: &Map<String, Value>) -> Result<Value>;
    fn content_type(&self) -> Option<&'static str> { None }
}
```

object-safe 不变（无泛型方法），`Arc<dyn Packer>` 与「每请求选择 packer」场景（pay 32 处 `setPacker`）保持兼容。

**已验证的 serde_json API 契约**（读过 registry 源码 serde_json-1.0.149/src/map.rs，Cargo.lock 锁定版本；1.0.149 与 1.0.151 的 map.rs 逐字节一致）：
- ✅ `Map::insert(String, Value)`（L127）、`Map::entry<S: Into<String>>`（L274，可直接 `map.entry("k")` 传 `&str`）
- ✅ `FromIterator<(String, Value)>`（L549）
- ❌ **无 `From<[(K,V); N]>`**——迁移不能写 `Map::from([...])`，必须 `Map::from_iter([...])` 或 `.into_iter().collect()`，**这是执行期最大的机械踩坑点**

### 3.2 公开 API 面（BREAKING 全清单，均已验证源码位置）

| API | 位置 | 变更 |
|---|---|---|
| `Packer::pack/unpack` | `packer.rs:25,38` | 两形参 `&HashMap`→`&Map` |
| `Rocket::new` / `payload` / `get_params` | `rocket.rs:141,156` | → `Map` |
| `Artful::artful` / `shortcut` | `artful.rs:163,210` | `params` → `Map` |
| `Shortcut::get_plugins` | `shortcut.rs:23`（trait 定义，L19 为 doc 注释） | `&HashMap`→`&Map` |
| `filter_params` | `lib.rs:101` | → `Map` |
| `Event::ArtfulStart.params` | `event.rs:74` | → `&Map` |
| `From<HashMap> for Rocket` | `rocket.rs` | **删除**（用户已确认） |

**明确不动**：`RocketConfig.headers`（`HashMap<String,String>`，头顺序无意义）、`Config.extra`（与 payload 键序语义无关，扩大 BREAKING 面无收益）、`Destination` 枚举结构、`DirectionKind`、`Direction` trait、`Plugin` trait、`async_trait` 用法。

### 3.3 typed 便利层（Added，非破坏）

```rust
// packer.rs —— packer 层（dyn 兼容）
pub fn pack_typed<T: Serialize>(packer: &dyn Packer, data: &T, params: &Map<String, Value>) -> Result<String>;
// 流程：to_value(data) → 校验 Object → packer.pack(obj, params)

pub fn unpack_typed<T: DeserializeOwned>(packer: &dyn Packer, data: &str, params: &Map<String, Value>) -> Result<T>;
// 流程：packer.unpack(data, params) → from_value::<T>

// direction.rs —— Destination 视图方法
impl Destination {
    pub fn into_json(self) -> Result<Value>;  // Json→Ok；Response/None→DestinationMismatch
}

// artful.rs —— 入口层
impl Artful {
    pub async fn artful_as<T: DeserializeOwned>(
        &self,
        params: Map<String, Value>,
        plugins: Vec<Arc<dyn Plugin>>,
    ) -> Result<T>;
    // 流程：artful() → into_json() → from_value::<T>
}
```

**依赖变化（用户已确认 2026-09-12）**：`T: Serialize` / `T: DeserializeOwned` 的 trait bound 引用 `serde::Serialize`/`serde::de::DeserializeOwned`，serde crate 必须成为正依赖——`dependencies` 加 `serde = { version = "1.0", default-features = false, features = ["std"] }`（无 derive，下游不继承 derive 特性；serde 已在依赖树中作为 serde_json 的传递依赖，直接声明不新增第三方代码）；`dev-dependencies` 加 `serde = { version = "1.0", features = ["derive"] }` 供测试/示例的结构体 derive。

> 注意：这是对 0.17.0「不引入 serde」约束（PR #14 不变项）的有意修正——typed 层的公开 API bound 需要 serde crate 直接可见，已获用户确认。

### 3.4 错误变体（共新增 1 个）

| 场景 | 变体 | 理由 |
|---|---|---|
| `pack_typed` 的 `to_value` 失败 | 复用 `JsonSerializeError`（`#[from]` 已有） | 语义吻合 |
| `pack_typed` 输入序列化为非 Object | 复用 `InvalidParameter { param: "data" }` | "调用方传参非法"语义吻合 |
| `unpack_typed` 的 `from_value` 失败 | 复用 `JsonDeserializeError { message }`（message 注明目标类型） | 用户视角同为"反序列化失败" |
| `into_json`/`artful_as` 遇 `Response`/`None` | **新增 `DestinationMismatch`** | destination 是链路结果而非调用方传参，`InvalidParameter` 语义不贴切 |

`DestinationMismatch` 定稿形态（error.rs 全 Display 风格）：字段 `expected: &'static str`、`actual: String`；`#[error("destination mismatch: expected {expected}, got {actual}")]`。`actual` 取变体名（`"Response"`/`"None"`），不用 Display 的 `"<HTTP Response>"` 文案。

### 3.5 能力语义收窄（本次调研的架构产出）

- packer 定位从「运行时可替换的机制」收窄为「**请求级配置：请求链早期设定一次，框架不承诺链中途替换语义**」——与 PHP 事实形态（pay 32 处 `setPacker` 均在链启动阶段、全库无二次覆盖）忠实对齐。
- 机制保留：`rocket.packer` 仍为 `pub Arc<dyn Packer>` 可变字段（「链早期设定」本身需要它）。
- 测试语义对齐：`tests/parser_test.rs:64`、`tests/integration_test.rs:205` 的 `ReplacePackerPlugin` 重构为「链早期设定 packer」语义（置于链首设定，注释更新），保留作为「每请求选择 packer」真实场景的回归保护。
- 文档声明：ARCHITECTURE.md / CHANGELOG 明示上述定位。

### 3.6 行为变化（非 BREAKING 但需声明）

- `JsonPacker` 输出键序：随机 → 字典序（JSON 对象键序无语义，纯收益：日志/签名确定性收敛）。
- `QueryPacker`/`XmlPacker` pack：删手动排序，行为不变（字典序相同）。

## 4. 推进策略

### 阶段划分

```
Wave 0（串行）Task 1: src 全量类型迁移（trait + rocket + lib + packers + plugins + directions
                      + artful + shortcut + event + flow_ctrl + ignite + error.rs 加变体 + 删 From）
   │  验收: cargo test -p artisan-http --lib 全绿（覆盖 cfg(test) 模块）
Wave 1（并行）Task 2: packers 内部简化（删排序、Xml BTreeMap→Map、文档注释）
            Task 3: typed 便利层（pack_typed/unpack_typed/into_json/artful_as + 单元测试）
   │  验收: cargo check --workspace --all-features 通过
Wave 2（并行）Task 4: tests×6 迁移 + ReplacePackerPlugin 语义重构 + artful_as 端到端用例
            Task 5: examples×7 迁移 + 新增 examples/typed.rs
   │  验收: cargo test --workspace --all-features 通过（Task 4 后）
Wave 3（串行）Task 6: 版本号三处 + CHANGELOG×2 + README×2 + ARCHITECTURE.md
   │  验收: 五连门禁全绿
最终验证     F1-F4（对照 plan 文档）
```

### 回滚

单 PR 内聚（一个 commit 序列、一次合并），合并前 `git revert` 即整体回滚；无灰度、无配置开关。

## 5. 风险与对策

| 风险 | 严重度 | 对策 |
|---|---|---|
| `preserve_order` 全局特性：下游一旦启用，`Map` 变 IndexMap（插入序），框架「pack 键序确定性」承诺被破坏 | 中 | 无法阻止（Cargo 特性合并全局生效）。CHANGELOG/ARCHITECTURE 明示「键序保证基于 serde_json 默认 BTreeMap 后端」 |
| 迁移量大（约 40+ 处 + 测试示例），机械重复中出错 | 中 | 编译器错误即迁移地图；`Map::from_iter` recipe 写入 plan 与 CHANGELOG（`Map::from([...])` 不存在，是头号踩坑点）；逐 Wave 编译验证 |
| 新增公开枚举变体 `DestinationMismatch` 破坏下游 exhaustive match | 低 | 0.x 阶段可接受；CHANGELOG 置顶标注 |
| `JsonPacker` 键序从随机变字典序 | 低 | 语义无害（JSON 对象键序无意义），CHANGELOG 声明 |
| 测试断言依赖旧 `HashMap` 构造形态 | 低 | 机械替换 + `cargo test` 全绿为验收 |
| `Map::new()` 无类型标注时无法推断 | 低 | 测试/示例中 `let mut params: Map<String, Value> = Map::new();` 显式标注，写入 plan |

## 6. 监控与可观测性

库项目无运行时监控；以 CI 门禁替代：

- **本地五连门禁**（plan F3 执行）：`cargo fmt --all -- --check` / `cargo clippy --workspace --all-features -- -D warnings` / `cargo test --workspace --all-features` / `cargo test --doc` / `cargo build --workspace --all-features --examples`。
- **CI 实际门禁**（`.github/workflows/coding-linter.yml`，四步）：`cargo check --workspace --all-features` / `cargo fmt --all -- --check` / `cargo clippy --workspace -- -D warnings`（**无 `--all-features`**）/ `cargo test --workspace --all-features`；**examples build 无 CI 兜底**（doctest 已含于 CI 的 `cargo test` 步骤，F3 的 `cargo test --doc` 为冗余强化），依赖本地 F3 执行。
- **键序回归锚点**：QueryPacker/XmlPacker 现有确定性测试保留；JsonPacker 新增「输出字符串直接断言」用例 `assert_eq!(packed, r#"{"a":1,"b":2,"c":3}"#)`（raw string 内无反斜杠转义；不做 Value 解析回比——后者恒真无验证力）。
- **文档同步检查**：README 双语成对（仓库硬性规定）、CHANGELOG 0.18.0 条目含迁移 recipe、`artisan-http/README` 双语各 9 处代码示例同步更新。

## 附录 A：迁移 recipe（写入 CHANGELOG）

```rust
// 旧（0.17.x）
use std::collections::HashMap;
let params = HashMap::from([("order_id".to_string(), json!("123"))]);
let rocket = Rocket::new(params);

// 新（0.18.0）——注意 serde_json::Map 无 From<[(K,V); N]>，必须 from_iter
use serde_json::Map;
let params: Map<String, Value> = Map::from_iter([("order_id".to_string(), json!("123"))]);
let rocket = Rocket::new(params);

// 自定义 Packer 迁移：仅形参类型替换
// fn pack(&self, data: &Map<String, Value>, _params: &Map<String, Value>) -> Result<String>

// 强类型入口（新增能力）
let order: OrderResp = artful.artful_as(params, plugins).await?;
```
