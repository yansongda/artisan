# 技术设计：artisan-http 0.17.0 对齐 artful —— 补齐 Direction / Packer / ParserPlugin

> **时间**：2026-09-01
> **作者**：GLM-5.3-Flash + yansongda
> **状态**：经过人工审核确认
> **关联文档**：`docs/artisan-http-0.16.0-ignite-core.md`（0.16.0 将 ParserPlugin 合并进 IgniteCore 的决策，本方案对其做有意修正）
> **修订记录**：2026-09-01 初版经用户批准；同日 plan-reviewer 初审（结论：拒绝执行，1 BLOCKER + 5 MAJOR）后逐条源码复核（全部属实、零驳回）并修订：补 Task 2 调用点清单（add_radar.rs/integration_test.rs）、XmlPacker pack 改 is_numeric 值判定、unpack 空元素→空 Object、quick-xml 钉 0.41（MSRV）、Task 3 spike 降级协议（本机无 PHP）、examples 补 custom_plugin/event、根 CHANGELOG 等详见 plan 文档同日修订记录。

---

## 1. 背景与问题

### 现状

artisan-http（0.16.0）是 yansongda/artful（PHP）的 Rust 移植。响应解析策略以 `DirectionKind` 枚举（`Json`/`Response`/`NoRequest`/`Custom`）表达，其分发逻辑**内联在链尾核心 `IgniteCore` 中**（`artisan-http/src/ignite.rs` L79-91）；`Packer` 仅有 `JsonPacker`；插件仅有 `StartPlugin`/`AddPayloadBodyPlugin`/`AddRadarPlugin`。

### 困境

1. **与 artful 的类清单不对齐**：artful 有独立的 `NoHttpRequestDirection`、`OriginResponseDirection`、`QueryPacker`、`XmlPacker`、`ParserPlugin`；Rust 版的对应逻辑要么内联在 `IgniteCore` 的 match 里，要么不存在。
2. **架构分工与 artful 不一致**：artful 中 `IgniteCore` 等价物（`Artful::ignite()`）只负责 HTTP 执行，"解析"由独立的后置插件 `ParserPlugin` 承担；Rust 0.16.0 把两者合并进了 `IgniteCore`。
3. **能力缺口**：无法以 query-string / XML 格式编码请求体（微信支付 V2 等场景）；`Packer::unpack` 签名缺 `params` 通道，导致 PHP 的 `_unpack_raw`（银联证书场景）无法复刻；`JsonDirection` 硬编码 JSON 解析、不经过 `packer`，使自定义 Packer 对响应方向无效。
4. **恢复 `ParserPlugin` 与 0.16.0 刚做出的 BREAKING 决策正面冲突**（0.16.0 刻意删除它并把 HTTP 执行+解析合并进 `IgniteCore`，设计文档明确"防双执行"）。

### 目标

- **类清单对齐 artful**：五个缺失能力全部以独立类型呈现，命名与 PHP 一致
- **职责分工对齐 artful**：`IgniteCore` = 纯 HTTP 执行；`ParserPlugin` = 纯响应解析
- **默认行为不回归**：`DirectionKind` 枚举 API、默认 Json 解析路径对存量用户行为不变（除显式声明的 BREAKING 项）
- **quirk 忠实复刻**：XML CDATA/空元素空对象、query RFC1738 编码、`_unpack_raw` 原始模式，以 artful 测试用例为验收基准（注意：PHPUnit assertEquals 为宽松比较，测试名义期望不可直接当类型语义，见附录）

---

## 2. 整体方案

### 核心思路

**把 0.16.0 "过激合并"的职责拆回 artful 的原始分工**：`IgniteCore` 瘦身为纯 HTTP 执行（对齐 PHP `Artful::ignite()`，含 `NoRequest` 短路判断——这在 PHP 里本来就是 ignite 的 `should_do_http_request` 职责），解析职责以 `ParserPlugin` 插件形式回归（对齐 PHP），新增的两个 Direction 与两个 Packer 均为独立实现类型。

> 注意：0.16.0 删除 `ParserPlugin` 的核心理由是"它同时承担 HTTP 执行，忘挂则连请求都不发"。本方案中 `IgniteCore` **保留 HTTP 执行**，忘挂 `ParserPlugin` 的后果降级为"请求发出但不解析"（destination 为 None、origin 有值），风险显著小于 0.15.x 形态。此为对 0.16.0 决策的**有意修正**，属 BREAKING 变更，已经用户确认（2026-09-01）。

### 架构图

```
用户插件链（洋葱模型）
┌────────────────────────────────────────────────────────────┐
│ StartPlugin → [业务插件] → AddPayloadBodyPlugin             │
│   → AddRadarPlugin → ParserPlugin(新增，链尾插件)           │
│                              │ next                         │
│                    IgniteCore（框架内置链尾，瘦身）          │
│                     ① NoRequest 短路（不发请求）            │
│                     ② HttpStart 事件                        │
│                     ③ client.execute(radar)                 │
│                     ④ destination_origin = response         │
│                     ⑤ HttpEnd / HttpError 事件              │
│                     ✗ 不再解析                              │
└────────────────────────────────────────────────────────────┘
ParserPlugin 后向逻辑（HTTP 完成后）：
  读 destination_origin / config.direction / packer / payload
  → direction.parse(rocket) 分发：
      DirectionKind::Json      → JsonDirection（改为经 packer.unpack 解包）
      DirectionKind::Response  → OriginResponseDirection(新增)
      DirectionKind::NoRequest → NoHttpRequestDirection(新增)
      DirectionKind::Custom    → 用户自定义
  → rocket.destination = Some(result)
```

### 文件结构

```
artisan-http/src/
├── packer.rs                 [修改] Packer trait: pack/unpack 增加 params 参数
├── packers/
│   ├── mod.rs                [修改] 模块声明 + 内置表格追加
│   ├── json.rs               [修改] 适配新签名；unpack 忽略 params
│   ├── query.rs              [新增] QueryPacker（手写 RFC1738 编解码，零新依赖）
│   └── xml.rs                [新增] XmlPacker（quick-xml 解析；pack 手写拼接）
├── direction.rs              [不改] DirectionKind 枚举 API 保持不变
├── directions/
│   ├── mod.rs                [修改] 模块声明 + 内置表格追加
│   ├── json.rs               [修改] parse 改为经 rocket.packer.unpack 解包
│   ├── no_http_request.rs    [新增] NoHttpRequestDirection（透传）
│   └── origin_response.rs    [新增] OriginResponseDirection（原始 Response，缺失报错）
├── plugins/
│   ├── mod.rs                [修改] 模块声明 + 表格追加
│   ├── parser.rs             [新增] ParserPlugin
│   ├── add_payload_body.rs   [修改] pack 调用点适配新签名
│   └── add_radar.rs          [修改] pack 调用点适配新签名（fallback 分支）
├── ignite.rs                 [修改] 删除解析分发块（match + destination 赋值）；NoRequest 短路保留
├── error.rs                  [修改] 新增 XmlSerializeError/XmlDeserializeError
├── lib.rs                    [修改] re-export 新类型 + doctest + assert_send_sync
└── rocket.rs                 [不改] params/payload 结构维持现状
artisan-http/Cargo.toml       [修改] 新增 quick-xml 0.41（MSRV 1.79 兼容 workspace 1.85）；版本 0.17.0
CHANGELOG.md / README*.md / AGENTS.md / docs/ARCHITECTURE.md  [修改] 全量同步
examples/direction.rs         [修改] 追加新 Direction 演示
examples/query_xml_packer.rs  [新增] Query/Xml Packer 演示
examples/custom_plugin.rs     [修改] 链尾补挂 ParserPlugin（L51/L67）
examples/basic.rs             [修改] 链尾补挂 ParserPlugin（L31/L44）
examples/shortcut.rs          [修改] 两处链尾补挂 ParserPlugin（L31/L49/L70/L77）
examples/event.rs             [修改] 第一处链补挂；第二处专项演示链改 None 口径（L91/L114/L102/L123/L112）
tests/                        [修改] direction/integration/artful/event/shortcut 更新 + 新增 parser_test.rs
根 Cargo.toml                 [修改] artisan-http 版本引用 0.17.0
根 CHANGELOG.md               [修改] 0.17.0 facade 透传条目（对齐 0.16.0 先例）
根 Cargo.lock                 [随构建更新]
```

### 受影响的存量调用点（Task 2 必须全量适配，grep 实测）

| 位置 | 现状 | 适配 |
|---|---|---|
| `plugins/add_payload_body.rs:30` | `packer.pack(&rocket.payload)?` | 补第二参空 map |
| `plugins/add_radar.rs:45`（fallback 分支） | `packer.pack(&rocket.payload)?` | 补第二参空 map + `use std::collections::HashMap` |
| `tests/integration_test.rs:179-185`（`impl Packer for FormPacker`） | 旧签名 trait impl | impl 签名补参（trait impl 不允许"暂存"，必须同步改） |

---

## 3. 详细设计

### 3.1 Direction 补齐（无破坏）

| 类型 | 语义（已验证：读过 artful 源码） | Rust 行为 |
|---|---|---|
| `NoHttpRequestDirection` | `guide` 原样返回 response（null → null）；真正作用是 ignite 阶段 `should_do_http_request` 豁免其及子类，**不发 HTTP** | struct 实现 `Direction`：`parse` 透传 `rocket.destination`（`Some` 原样克隆返回，`None` → `Destination::None`）。`NoRequest` 前置短路**保留在 IgniteCore**（对齐 PHP ignite 职责，match 中的兜底分支同样指向该 struct，正常不可达） |
| `OriginResponseDirection` | `guide` 不解包返回原始 Response；response 为 null 抛 9303；**会**发起 HTTP | struct 实现 `Direction`：`take destination_origin` → `Destination::Response`，`None` → `Err(MissingResponse)`。即把 IgniteCore 内联的 `Response` 变体逻辑原样抽出 |

- `DirectionKind` 枚举变体名（`NoRequest`/`Response`）**保持不变**，仅 match 分发目标从内联逻辑改为调用上述 struct（`Custom` 分支的 `direction.clone()` 借用规避手法沿用）。
- PHP 的 `ResponseDirection`（`NoHttpRequestDirection` 的纯语义子类）**本次不复刻**：Rust 无继承，`DirectionKind::NoRequest` 已承担该语义；若未来需要，可加同名零开销标记 trait。已验证：读过 PHP 源码确认其为空壳别名。

### 3.2 Packer 补齐（含 BREAKING：trait 签名）

**trait 签名变更**，对齐 PHP `PackerInterface`：

```rust
pub trait Packer: Send + Sync + std::fmt::Debug {
    fn pack(&self, data: &HashMap<String, Value>, params: &HashMap<String, Value>) -> Result<String>;
    fn unpack(&self, data: &str, params: &HashMap<String, Value>) -> Result<Value>;
    fn content_type(&self) -> Option<&'static str> { None }
}
```

- `params` 来源对齐 PHP：`unpack` 侧由 ParserPlugin/Direction 传 `rocket.payload` 全量（**不过滤** `_` 特殊参数，`_unpack_raw` 因此可达）；`pack` 侧 AddPayloadBodyPlugin 传空 map（PHP 内置链传 null，已验证：读过 PHP 源码确认内置无人传参）。
- `JsonPacker` 忽略 params，行为不变；自定义 Packer 用户需适配新签名 → **BREAKING**。

**QueryPacker**（已验证：读过 artful 源码；个别 edge 为推断，见标注）：

| 方法 | 行为 |
|---|---|
| `pack` | RFC1738：`k=v&k2=v2`，空格→`+`，手写 percent-encode（无新依赖）。`Bool(true)`→`"1"`、`Bool(false)`→`""`、`Null`→`""`（对齐 PHP `http_build_query` 标量强转）；Number 原样（已知差异：serde_json 整值浮点 `29.0` → `"29.0"`，PHP 为 `"29"`，进有意差异清单）；嵌套容器复刻 PHP `k[sub]` 递归语法（数组按下标 `a[0]=v`，空容器跳过）；输出键序不保证（HashMap 无序为既有 trait 设计） |
| `unpack` 默认 | URL 解码、`+`→空格；key 中 `.`/空格→`_`（PHP `parse_str` quirk，**复刻**）；`k[sub]=v` → 嵌套 Object、`k[]=v` → Array 追加；值一律 `Value::String` |
| `unpack` raw（`params["_unpack_raw"]` truthy 时） | 按 `&` 切段、首个 `=` 分割，**零解码**（银联 `signPubKeyCert` 含 `\r\n`/`+`/`/` 不被破坏）；空串或不含 `=` → 空对象；无 `=` 的混合段 key/value 行为以 spike 实测 PHP 为准（推断） |
| truthy 判定 | `Bool(true)` / `Number≠0` / 非`""`非`"0"` 的 `String` / 非空 `Array`·`Object`（对齐 PHP truthy：`"0"`、空容器为 falsy） |
| `content_type` | `Some("application/x-www-form-urlencoded")` |

**XmlPacker**（已验证：读过 artful 源码；PHP is_numeric 值判定语义经审查复核确认）：

| 方法 | 行为 |
|---|---|
| `pack` | `<xml><k><![CDATA[v]]></k><n>29</n></xml>`：**值判定**对齐 PHP `is_numeric($val)`（实测源码 `supports/Collection.php:288-301`）——`Number` 或符合 PHP `is_numeric` 语义的数值字符串（i64/u64/f64 解析成功，近似覆盖 `"29"`/`"1.5"`/`"1e5"`；边角差异：前导空白 PHP true/Rust false、`"inf"`/`"NaN"` Rust true/PHP false，入有意差异清单）→ 纯文本节点，其余 → CDATA；**CDATA 分支字符串化规则对齐 PHP 隐式转换**：`Bool(true)`→`"1"`、`Bool(false)`→`""`、`Null`→`""`（与 QueryPacker pack 规则一致）；**空 payload 输出 `<xml></xml>` 而非空串**（与 JsonPacker 不同，已验证）；仅支持一维标量（对齐 PHP 名义语义），嵌套容器 → `XmlSerializeError`（有意差异：PHP 产出 `<![CDATA[Array]]>` 垃圾值，Rust 显式报错）。已知边界差异：serde_json 整值浮点 `29.0` 输出 `"29.0"` 而 PHP `(float)29.0` 为 `"29"`（进有意差异清单）；键不做 XML 转义（对齐 PHP 现状） |
| `unpack` | quick-xml 事件流解析：忽略根元素名、CDATA 剥离、属性丢弃、重复标签→`Value::Array`、**叶子文本一律 `Value::String`**（保真复刻：PHP simplexml→json_encode→json_decode 全程无数字转换，叶子文本实为字符串；artful `XmlPackerTest::testUnpack` 的 `assertEquals(['age'=>29],...)` 通过系 PHPUnit 宽松比较 `'29' == 29`，不能证明数字语义——故不从 artful 测试名义期望推断类型）、**无文本元素（`<a></a>` 与自闭合 `<a/>`）→ 该 key 值为空 `Value::Object`**（对齐 PHP SimpleXML→json 怪癖）、`""` 与 `"0"` → 空 Object（对齐 PHP `empty()` 语义，`"0" 为 falsy）、仅空白输入 → `XmlDeserializeError`（PHP 侧 simplexml 失败→TypeError）、混合内容（元素同时含文本与子元素）丢弃直接文本（对齐 PHP）；解析失败 → `XmlDeserializeError`（PHP 此处抛 TypeError，Rust 以错误类型优雅表达，属**有意差异**）。已知差异：XML 注释/处理指令 quick-xml 默认忽略而 PHP 会产出 `"comment":{}` 等假节点（记录在案） |
| `content_type` | `Some("application/xml")` |

**依赖**：新增 `quick-xml = "0.41"`（**钉定**：0.42.0 的 rust_version=1.86 超出 workspace MSRV 1.85，0.41.x 为 1.79 兼容；不启用 serde/encoding feature——已验证 default features 为空集）；query 编解码手写、不引入 `percent-encoding`/`serde_urlencoded`。

### 3.3 ParserPlugin 回归（BREAKING 核心）

- **IgniteCore 瘦身**：删除 `ignite.rs` L79-91 解析分发块，保留 `NoRequest` 短路、`HttpStart`/`HttpEnd`/`HttpError` 事件、`execute`、`destination_origin` 写入。HTTP 执行不再是插件职责（0.16.0 的正当理由得以保留），只移交"解析"。
- **ParserPlugin**（`plugins/parser.rs`，unit struct + 全 derive，`name()` 返回 `"ParserPlugin"`）：
  - 前向：无逻辑，直接 `next.call(rocket).await`
  - 后向（HTTP 已完成后，对齐 PHP 后置插件语义，已验证：读过 PHP 源码确认 `$next($rocket)` 在最前）：
    1. 守卫：`rocket.destination` 为 `Some(Destination::Json(_))` → `InvalidParameter`（对齐 PHP 9208 "destination 只能 null 或 ResponseInterface"）；`None`/`Some(Destination::Response(_))`/`Some(Destination::None)` 放行
    2. 按 `config.direction` 分发到对应 Direction（含 `Custom`）
    3. `rocket.destination = Some(parse 结果)`；`parse` 消费 `destination_origin`（`take()` 惯例保持）
  - `params` 传递：`rocket.payload` 全量传给 `packer.unpack`（3.2）
  - **与 PHP 的响应来源差异（已声明）**：PHP `ParserPlugin` 读 `destination`（ignite 同时写 destination 与 destinationOrigin），Rust 读 `destination_origin`——正常链路两者持有同一 response，等价；用户插件在前向阶段预置 `Some(Destination::Response)` 时，PHP 解析预置值而 Rust 解析 origin，文档标注此差异。
- **JsonDirection 改造**：`parse` 由硬编码 `serde_json::from_str` 改为 `rocket.packer.unpack(&text, params)` → `Destination::Json`。默认路径（JsonPacker）行为等价（同样的 `from_str`、同样的错误包装），**默认用户无感**；但 `packer=XmlPacker + direction=Json` 时响应按 XML 解包——这正是 artful `CollectionDirection.guide($packer, ...)` 的架构语义（已验证：读过 PHP 源码）。`DirectionKind::Json` 名字沿用历史，文档标注其语义为"经 rocket.packer 解包（同 artful CollectionDirection）"。
- **`NoRequest` 场景语义**：挂 `ParserPlugin` 后 destination 变为 `Some(Destination::None)`（对齐 PHP guide 返回 null 语义）；0.16.0 为保持 `None`。两者经 `Artful::artful` 入口无差异（`rocket.destination.unwrap_or_default()` 归一为 `Destination::None`），仅 `ArtfulEnd` 监听器直接观测 `rocket.destination` 时有差异。
- **用户链默认形态变更** → 0.17.0，README/CHANGELOG 提供迁移示例：

```rust
// 0.16.0
let plugins: Vec<Arc<dyn Plugin>> = vec![start, method_url, add_payload_body, add_radar];
// 0.17.0 —— 链尾追加 ParserPlugin，忘挂则请求发出但不解析
let plugins: Vec<Arc<dyn Plugin>> = vec![start, method_url, add_payload_body, add_radar, parser];
```

### 3.4 配套

- **error.rs**：新增 `XmlSerializeError { message, source }` / `XmlDeserializeError { message, source }`（对齐 `Json*` 命名，不占用 `#[from]`）；模块 doc 清单与底部单测同步。`XmlPacker::pack` 遇嵌套容器等无法序列化场景走 `XmlSerializeError`（保证变体有构造点，对齐 `JsonSerializeError` 被 `JsonPacker::pack` 使用的先例）；query 编码类错误复用现有 `InvalidParameter`。
- **导出注册**：`packers/mod.rs`、`directions/mod.rs`、`plugins/mod.rs` 模块声明 + Markdown 表格；`lib.rs` 顶层 `pub use` 追加 5 个新类型 + `assert_send_sync` 契约测试追加。根 facade `pub use artisan_http as http` 整体导出，无需修改。
- **文档同步**：CHANGELOG（0.17.0 BREAKING 条目 + 迁移说明）、根与 artisan-http 的 README/README.zh-CN（双语同步策略；**注意反转 0.16.0 写入的规范性段落**——如 `artisan-http/README.md:324` "插件链无需且不可挂解析插件"，0.17.0 语义相反）、根 `CHANGELOG.md`（facade 透传条目，对齐 0.16.0 先例）、AGENTS.md（插件清单/测试文件表修正 drift：`Tests across 7 files` 实为 6（5 现存 + parser_test）、"Tests in tests/ not inline" 与现状不符、测试文件表列出不存在的文件且缺席 event_test.rs）、`docs/ARCHITECTURE.md`；`lib.rs` 顶层 doc（关键类型表/内置插件表/doctest）同步。
- **测试策略**：单测内联（query/xml 编解码边界、direction 透传/报错、守卫等价物）；集成测试 wiremock（全链路：query pack → xml 响应 unpack → raw 模式证书无损）；`examples/` 编译验证；与 artful `tests/Packer/*Test.php`、`tests/Direction/*Test.php`、`tests/Plugin/ParserPluginTest.php` 的断言值逐条对齐作为验收基准（已验证：读过 PHP 测试源码，断言值已固化进 plan 文档 References）。**PHP spike 降级**：本机无 PHP（2026-09-01 实测 `php: command not found`），PHP 侧 edge（raw 混合段、XML 数字/空元素 roundtrip）按已记录推断值实现并在测试/文档标注"推断，未经实测"。

---

## 4. 推进策略

```
Wave 1  Direction 抽离（3.1）                    —— 纯重构，行为零变化
   ├─ 验证点：cargo test 全绿（现有 direction_test 不改断言）
   └─ commit: refactor(direction): extract NoHttpRequest/OriginResponse
Wave 2  Packer 扩展（3.2，串行）                  —— trait 签名 BREAKING + 新依赖
   ├─ 2a trait params 化 → 2b QueryPacker → 2c XmlPacker + error
   ├─ 验证点：JsonPacker 行为回归；query/xml 单测过；对齐 PHP 测试断言
   └─ commit: feat(packer)!: add Query/XmlPacker, params in trait
Wave 3  ParserPlugin 拆分（3.3，串行）            —— 行为 BREAKING
   ├─ 3a JsonDirection 经 packer → 3b ParserPlugin 新增 → 3c IgniteCore 瘦身 + 测试全量切换
   ├─ 验证点：集成测试全链路（含"忘挂 ParserPlugin → destination None"负例断言）
   └─ commit: feat(plugin)!: move response parsing into ParserPlugin
Wave 4  文档与示例（3.4）                         —— 收尾
   └─ 验证点：cargo fmt/clippy/test --workspace --all-features 三连全绿
```

- 波次间串行（`ignite.rs`/`lib.rs`/`mod.rs` 被多波修改），Wave 2/3 内部也串行（`packers/mod.rs`、`tests/` 存在共享文件）。
- **回滚**：全程未合入前正常 `git revert`；若已发 0.17.0，下游锁定 `artisan-http = "0.16"` 即回旧行为（0.16.0 无 yanked 必要，两版本行为各自完整）。

---

## 5. 风险与对策

| 风险 | 严重度 | 对策 |
|---|---|---|
| 升级用户忘挂 `ParserPlugin` → 请求发出但响应不解析（静默） | **高** | CHANGELOG 置顶 BREAKING + README 迁移示例；集成测试写"忘挂 → destination None"负例固化文档口径；`IgniteCore` 保留 HTTP 执行使后果限于"不解析"（优于 0.15.x 连请求都不发） |
| 推翻 0.16.0 架构决策引发反复 | 中 | 本方案为**有意修正**而非回退：保留 0.16.0 拆出的 `IgniteCore`（HTTP 执行），仅把"解析"归还插件；设计文档留档理由 |
| XML/query quirk 与 PHP 不一致（部分 edge 无 PHP 环境实测） | 中 | 验收以 artful 测试断言逐条对齐；无 PHP 环境的 edge（raw 混合段、XML roundtrip 细节）按推断值实现并显式标注"推断，未经实测"；无法对齐处（如非法 XML 的 TypeError）在 rustdoc 显式标注"有意差异"清单 |
| `Packer` trait 加 params 破坏自定义实现 | 中 | CHANGELOG 迁移说明给出 before/after；0.x 阶段一次到位避免二次破坏 |
| HashMap 无序导致 pack 输出键序不稳定 | 低 | pack 类单测断言顺序无关（roundtrip / 切片排序比对）；文档标注"pack 输出键序不保证" |
| quick-xml 新依赖（供应链/体积） | 低 | 钉 `0.41`（rust_version 1.79 兼容 workspace MSRV 1.85，0.42 需 1.86 不采用）；default features 空集（无 serde），锁定版本 |

---

## 附录：契约对照与证据等级

| PHP（artful） | Rust 落点 | 证据 |
|---|---|---|
| `NoHttpRequestDirection.guide` 透传 + `should_do_http_request` 豁免 | struct + IgniteCore 前置短路 | 已验证（读过源码） |
| `OriginResponseDirection.guide` null → 9303 | struct + `MissingResponse` | 已验证 |
| `QueryPacker` RFC1738 / `_unpack_raw` 零解码 / parse_str 点号转下划线 | query.rs 三态 | 已验证（vendored supports 通读）；混合段无 `=` edge 为推断（本机无 PHP，无法 spike 实测） |
| `XmlPacker` CDATA/is_numeric 值判定/空 payload→`<xml></xml>`/叶子文本一律 String/空元素→空 Object | xml.rs | unpack 语义经 hakre 系列（SimpleXML+json_encode 权威分析）+ supports `Arr::wrapXml` 源码复核：PHP 全程无数字转换、叶子文本实为字符串（artful 测试名义期望数字系 PHPUnit 宽松比较）；`"010"`/`@attributes` 等 roundtrip 细节为推断（无 PHP 环境） |
| `ParserPlugin` 后置、params 未过滤、9208 守卫 | plugins/parser.rs | 已验证 |
| `CollectionDirection.guide` 经 `$packer->unpack` | JsonDirection 改造 | 已验证 |
| PHP 测试断言值（XmlPackerTest/QueryPackerTest/ParserPluginTest 等） | Rust 测试期望值 | 已验证（读过测试源码，值已固化进 plan） |
