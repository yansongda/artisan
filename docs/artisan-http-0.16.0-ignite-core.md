# artisan-http 0.16.0：链尾核心动作（IgniteCore）设计

> **时间**：2026-09-01
> **作者**：GLM-5.3 + yansongda
> **状态**：经过人工审核确认

## 1. 背景与问题

**现状**：artisan-http 的 HTTP 请求发起、direction 解析、`HttpStart`/`HttpEnd`/`HttpError` 事件分发全部封装在 `ParserPlugin`（`src/plugins/parser.rs`）中，作为普通插件由用户手动挂载到链尾。PHP 版 yansongda/artful 则将请求发起放在 Laravel Pipeline 的终点回调 `then(ignite)` 中（`src/Artful.php` 的 `artful()`/`ignite()` 方法，`then(ignite)` 约位于 L268），框架必然执行。

**困境**：

1. 用户忘挂 `ParserPlugin` 时请求**静默不发起**（无报错、无事件、destination 为 None），错误延迟暴露且难以定位；
2. `HttpStart`/`HttpEnd`/`HttpError` 生命周期事件是否触发取决于用户链构造正确性，监听器无法依赖"必然触发"契约；
3. `ParserPlugin` 名为"解析"实为"执行+解析"，职责名不副实，与 artful PHP 的 `ignite()` 语义不对齐。

**目标**：

- **必达**：经 `Artful::artful` / `Artful::shortcut` 入口的请求**必然发起**，HTTP 生命周期事件**必然触发**（`NoRequest` 方向除外，语义不变），与 PHP 版行为对齐；
- **编译期强制迁移**：老用法（链中手挂 ParserPlugin）在编译期暴露，而非运行期双请求或静默变化；
- **机制对框架内部收敛**：用户可见 API 面缩小而非扩大。

## 2. 整体方案

**核心思路**：将 `FlowCtrl` 升级为"洋葱链 + 固定终点"模型（对齐 Laravel Pipeline `then(ignite)` 与 reqwest-middleware "Client 恒为链尾终点"的既有实践），请求发起从"可选插件"变为"框架内置链尾核心动作"，由 `Artful::artful` 自动挂载。

**外部调研佐证**（已验证，来源为各框架源码/官方文档）：

| 框架 | 终点保证方式 | 启示 |
|------|------------|------|
| Laravel Pipeline | `then()` 必填终点，空链时终点立即执行 | 本方案直接对标 |
| reqwest-middleware | `ClientWithMiddleware` 内固定持有 reqwest::Client 为终点，用户只能追加中间件 | 与本方案最接近的 Rust 先例 |
| axios | 核心 `dispatchRequest` 为私有函数，不可作为拦截器挂载 | 核心/扩展角色分离，杜绝双执行 |
| tower / Go net/http | 类型层面终点不可省略 | 不适用（本库为运行期组装），仅佐证"终点必存在"惯例 |

**反例警示**：主流框架均**不**将核心动作同时暴露为可挂中间件--双入口是双重执行事故的常见根源（Express/Next.js/Django 均有真实案例）。故本方案**彻底删除** `ParserPlugin` 公开导出，不留 deprecated 双入口。

**架构图**：

```
改造前：
  Artful::artful(params, plugins)
    └─ FlowCtrl::new(plugins)                     // 链尾 = Ok(())，静默结束
         Start -> AddPayloadBody -> AddRadar -> [ParserPlugin?]   // 用户可漏挂
                                                   ↑ 漏挂 => 无请求、无事件

改造后：
  Artful::artful(params, plugins)
    └─ FlowCtrl::new(plugins) + set_core(IgniteCore)   // 框架自动挂载
         Start -> AddPayloadBody -> AddRadar -> ═ IgniteCore ═    // 终点必然执行
                                                │ NoRequest? -> 直接返回（不请求）
                                                │ dispatch HttpStart
                                                │ client.execute(radar)
                                                │   ├─ Ok  -> dispatch HttpEnd -> direction 解析
                                                │   └─ Err -> dispatch HttpError
                                                ▼
         后向阶段：各插件依次回退（洋葱剥层，与改造前一致）
```

**文件结构**：

```
artisan-http/src/
├── flow_ctrl.rs        # 修改：+ CoreAction trait（pub(crate)）、+ core 字段、链尾执行
├── ignite.rs           # 新增：IgniteCore（由 plugins/parser.rs 迁移改名）
├── plugins/
│   ├── parser.rs       # 删除
│   └── mod.rs          # 修改：移除 parser 模块与导出
├── artful.rs           # 修改：artful() 挂载 IgniteCore；doc 更新
├── event.rs            # 修改：doc 措辞（"ParserPlugin 执行点"->"链尾核心动作"）
├── shortcut.rs         # 修改：单元测试链调整
└── lib.rs              # 修改：移除 ParserPlugin 导出、crate doc 更新
+ tests/examples/docs/双语 README/CHANGELOG×2/AGENTS.md 同步（见 3.5）
```

## 3. 详细设计

### 3.1 数据结构设计

`FlowCtrl` 新增内部字段与 trait（均 `pub(crate)`，不进公开 API）：

```rust
// flow_ctrl.rs（伪代码）
#[async_trait]
pub(crate) trait CoreAction: Send + Sync {
    async fn run(&self, rocket: &mut Rocket) -> crate::Result<()>;  // 无 Next：终点无下一层
}

pub struct FlowCtrl {
    cursor: usize,
    plugins: Vec<Arc<dyn Plugin>>,
    core: Option<Arc<dyn CoreAction>>,   // 新增
    is_ceased: bool,
}

pub(crate) fn set_core(&mut self, core: Arc<dyn CoreAction>) { ... }

pub async fn call_next(&mut self, rocket: &mut Rocket) -> Result<()> {
    if self.is_ceased { return Ok(()); }
    if !self.has_next() {
        // 链尾：执行核心动作，返回值沿洋葱后向阶段回退
        if let Some(core) = self.core.take() { return core.run(rocket).await; }
        return Ok(());   // 未挂 core：行为与现状一致（纯插件链直用场景）
    }
    // ...原有插件调度逻辑不变
}
```

| 设计点 | 决策 | 理由 |
|--------|------|------|
| core 是否复用 `Plugin` trait | 否，独立 `CoreAction` trait | 终点无 `next`；Next 模式下分离更诚实；防误挂回插件链 |
| `set_core` 可见性 | `pub(crate)` | 唯一挂载点为 `Artful::artful`（artful.rs:179，生产代码唯一 FlowCtrl 构造点--已验证读过源码）；防高级用户自造双入口 |
| `core.take()` | 一次性消费 | 终点只执行一次；插件双重调用 `next` 时第二次回落 `Ok(())`，与现状一致 |
| `skip_rest` 与 core | 跳过 core | 语义"主动中止流程"，与现状一致 |
| 空链 + 有 core | core 直接执行 | 对齐 Laravel 空 pipes 时终点立即执行 |

### 3.2 核心动作 IgniteCore

`ParserPlugin::assembly` 主体原样迁移为 `IgniteCore::run`，**仅两处变化**：

1. 删除末尾 `next.call(rocket).await`（终点无下一层）；
2. `NoRequest` 分支从"调用 next"改为"直接 `Ok(())` 返回"。

保持不变（已验证读过源码，parser.rs L52-L121）：

- `HttpStart` 分发时序（`radar.take()` 之前，监听器可经 `radar` 的 `*_mut` 修改请求）；
- `execute` 失败时 `HttpError` 分发 + `EventListenerError` 回填 `original` 字段的错误链处理；
- `HttpEnd` 于"execute 成功后、direction 解析前"分发；
- `MissingRequest`（radar 为 None）属前置失败不触发 `HttpError`；
- direction 分支解析（Json/Response/Custom/NoRequest）与 `destination_origin` 消费语义。

### 3.3 挂载与执行时序

```
Artful::artful()
  ① dispatch ArtfulStart（不变）
  ② FlowCtrl::new(plugins); ctrl.set_core(Arc::new(IgniteCore))     // 新增一行
  ③ ctrl.call_next(rocket)：前向穿透 -> 链尾 IgniteCore::run -> 后向回退
  ④ dispatch ArtfulEnd（不变）
```

用户链构造从 `[Start, ..., AddRadar, Parser]` 变为 `[Start, ..., AddRadar]`，与 artful PHP 的 Shortcut 插件组合完全同构（PHP 侧 Shortcut 也从不包含 ignite--ignite 由 pipeline 的 `then()` 保证，已验证读过 Artful.php）。

### 3.4 事件语义与兼容性

| 契约 | 改造前 | 改造后 |
|------|--------|--------|
| `HttpStart` 触发条件 | 到达 ParserPlugin 执行点（用户挂载才触发） | **必然触发**（`NoRequest` 除外） |
| `HttpEnd`/`HttpError` | 同上 | 同上 |
| 触发时机 | 链尾插件执行点 | 链尾核心动作点（正常链中时序等价：均在最后插件后、execute 前/后） |
| 空插件链 | 静默返回 `Destination::None` | 默认 direction（`Json`，rocket.rs:49 已验证）下报 `MissingRequest`（fail-fast，对齐 Laravel/Rack 惯例）；`NoRequest` 下行为不变 |
| 直用 `FlowCtrl::new`（未挂 core） | 链尾 `Ok(())` | **完全不变**（core 为 None 分支） |
| `Artful::raw` | 不经链 | 完全不变 |

**破坏性变更**（0.16.0 未 release，直接并入其 CHANGELOG 条目，对 0.15.x 构成 Breaking）：`ParserPlugin` 从 `artisan_http::plugins::ParserPlugin` 与顶层再导出中**删除**。老代码 `vec![..., Arc::new(ParserPlugin)]` 因类型不再存在而**编译失败**--这是特性而非缺陷：编译期强制迁移，杜绝"旧链 + 新 core"的双请求运行期事故。

迁移方式（写入 README/CHANGELOG）：从插件链中删除 `ParserPlugin` 一项即可；唯一例外：链中位于原 ParserPlugin **之后**的插件，其 `next.call` 之前的逻辑（前向阶段）现运行于请求执行之前（destination/destination_origin 尚为 None、radar 未消费；后向阶段不受影响），此类链型需复核。

### 3.5 影响面分析（explore 调研全清单，~80 处）

证据等级：lib.rs / plugins/mod.rs / parser.rs / artful.rs / flow_ctrl.rs / event.rs / 全部 examples / event_test.rs / shortcut_test.rs / 两份 Cargo.toml / AGENTS.md / rocket.rs Default 已完整阅读（主会话 + 调研双重验证）；artful_test.rs / direction_test.rs / integration_test.rs / ARCHITECTURE.md / README 为片段 + grep 定位（行号执行时以内容复核）；docs/event-system.md、0.14.0-optimization.md 仅 grep 命中，未通读。

| 类别 | 处置 |
|------|------|
| src 代码（修改 6 + 新增 1 + 删除 1，共触碰 8 文件：flow_ctrl/plugins-mod/artful/event/lib/shortcut 修改，ignite 新增，parser 删除） | 按 3.1-3.3 改造 |
| `parser.rs` 自身 5 个单元测试函数（其一含两场景断言） | 迁移至 `ignite.rs`，`FlowCtrl::new` 后补 `set_core`（同 crate 内可调用 pub(crate)） |
| `tests/` 5 个文件（~30 处链构造） | 从链中删除 `ParserPlugin`；4 处手建 FlowCtrl 且链中含 ParserPlugin 的测试迁移至 `Artful::artful` 入口（`set_core` 为 `pub(crate)`，集成测试不可见；迁移时以插件设置 `rocket.config`。注：artful_test.rs 的 `test_plugin_chain_stops_on_error` 亦手建 FlowCtrl 但链不含 ParserPlugin，无需迁移） |
| `examples/` 5 个文件 | 从链中删除 ParserPlugin；`event.rs` 示例可新增演示"不挂 parser 事件仍触发" |
| 双语 README ×2 + 根 README ×2 | 事件表、插件表、示例代码同步（仓库双语同步规范） |
| ARCHITECTURE.md §2.5/§3.1/§3.4/§4.4/§5/§6/§8 | 更新为终点动作模型（§4.4 重写为 IgniteCore；顺带修正该节代码为旧版的漂移） |
| CHANGELOG ×2 | **并入未 release 的 0.16.0 条目**（Changed/Removed，标注 BREAKING，相对 0.15.x） |
| artisan-http/AGENTS.md 模块树 | parser.rs 行改 ignite.rs（从 plugins/ 子树移至 src/ 顶层） |
| docs/event-system.md | 正文更新为终点动作模型（该文档描述的是 0.16.0 未发布版本的最终设计，0.16.0 事件条目中"不改插件链结构"约束已被本变更推翻） |
| docs/0.14.0-optimization.md | **历史文档保持原样**（记录当时决策，不追改） |

**测试语义核查结论**（主会话精读，已验证）：

- `direction_test.rs::no_request_skips_http_and_keeps_chain` 的 `MarkAfterParserPlugin` 断言（`destination.is_none()`）在新模型下**依然成立**：core 的 `NoRequest` 分支立即返回，前向/后向阶段 destination 均为 None；仅需更新注释措辞；
- `response_direction_consumes_origin` 的 `AssertOriginTakenPlugin` 断言位于 `next.call` 之后（后向阶段），新模型下 next 触发 core，断言依然成立；测试本身需迁移至 `artful()` 入口；
- `event_test.rs` 7 个事件序列断言**零变化**（HttpStart/HttpEnd/HttpError 触发点语义等价）；
- `artful.rs::artful_dispatches_start_and_end` 单元测试需改写：空链 + 默认方向从 `[ArtfulStart, ArtfulEnd] + Destination::None` 变为 `[ArtfulStart, HttpStart] + Err(MissingRequest)`，另补 NoRequest 空链变体。

## 4. 推进策略

```
阶段一：机制落地（FlowCtrl CoreAction + ignite.rs 迁移，ParserPlugin 暂保留）
  验证点：cargo test --workspace --all-features 全绿（行为零变化）
阶段二：切换与删除（artful 挂载 + ParserPlugin 删除 + tests/examples 适配，原子提交）
  验证点：三件套全绿；event_test 事件序列断言不变
阶段三：新契约回归测试 + 文档同步（并行）
  验证点：grep -r "ParserPlugin" 仅命中 CHANGELOG 历史条目（0.9.0）与 0.16.0 新增 BREAKING 条目、历史设计文档、本次两份设计/plan 文档及 evidence 产物（口径与 plan Task 4 一致）
最终验证：fmt / clippy -D warnings / test 三件套 + grep 零残留
```

**回滚方案**：逐 todo commit，任一阶段可 `git revert` 单独回退；0.16.0 发布前回滚零成本；发布后回滚需 bump 0.16.1 并在 CHANGELOG 标注。

## 5. 风险与对策

| 风险 | 严重度 | 对策 |
|------|--------|------|
| 下游用户（如 pay 调用方）老代码编译失败 | 中（0.x 生态预期内） | CHANGELOG/README 双语迁移指引（"删除链中 ParserPlugin 一项"）；编译错误信息指向明确（类型不存在） |
| 空链行为从静默变 fail-fast，个别依赖旧行为的调用方受影响 | 低 | 生产代码与集成测试均无 `artful(params, vec![])` 空链用法（调研已扫；src 内单测空链用法已列入改写清单）；CHANGELOG 显式标注行为变化 |
| 直用 `FlowCtrl` 的高级用户仍无 core（漏请求面未 100% 消除） | 低 | FlowCtrl 直用本属测试/高级场景，文档明示"生产请走 Artful 入口"；后续迭代再评估公开 `with_core` |
| 事件时序语义漂移（HttpStart 可改 radar 的窗口变化） | 低 | 触发点仍为 execute 前最后环节，正常链中时序等价；event_test 7 个时序断言全程护航 |
| ~80 处文档/测试机械替换引入漂移 | 低 | plan 按 todo 拆分 + 每阶段 grep 验证零残留 |

## 6. 监控与可观测性

本变更直接强化可观测性契约--事件触发矩阵由"条件触发"变为"恒触发"（`NoRequest` 除外），用户监听器（日志/metrics/tracing）可无条件依赖：

```
触发矩阵（改造后）：
| 场景                | ArtfulStart | HttpStart | HttpEnd | HttpError | ArtfulEnd |
|---------------------|:-:|:-:|:-:|:-:|:-:|
| 正常请求            | ✅ | ✅ | ✅ | -  | ✅ |
| HTTP 执行失败       | ✅ | ✅ | -  | ✅ | -  |
| NoRequest           | ✅ | -  | -  | -  | ✅ |
| 插件失败（前向阶段）| ✅ | -  | -  | -  | -  |  ← 变化：不再可能"到达链尾却无请求"
| 插件不调 next 返 Ok | ✅ | -  | -  | -  | ✅ |  ← core 不执行，链视为成功，ArtfulEnd 照常分发
| 链尾缺 radar        | ✅ | ✅ | -  | -  | -  |  ← MissingRequest fail-fast
| 解析阶段失败        | ✅ | ✅ | ✅ | -  | -  |
```

新增回归测试断言此矩阵（含"空链 + 有监听器 -> HttpStart 触发 + MissingRequest"），矩阵本身写入 ARCHITECTURE.md §3.4 作为契约。

## 附录：已批准的决策

1. **版本策略**：0.16.0 尚未 release，本变更**并入 0.16.0 CHANGELOG 条目**（对 0.15.x 构成 BREAKING），不新增 0.17.0，Cargo.toml 版本号不动；
2. **接受两处行为变化**：删除 `ParserPlugin` 公开导出（编译期强制迁移）+ 空链 fail-fast（默认方向报 `MissingRequest`）；
3. **文件改名 `parser.rs` -> `ignite.rs`**，类型 `ParserPlugin` -> `IgniteCore`（`pub(crate)`），对齐 artful PHP `ignite()` 命名；
4. `CoreAction`/`set_core` 保持 crate 内部，公开 API 面净缩小（导出少一项，无新增）。
