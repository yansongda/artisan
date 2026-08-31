# 技术设计文档:artisan-http 事件系统(v0.16.0)

> **2026-09-01 演进注记**:0.16.0 发布前,事件分发点从用户手动挂载的旧解析插件(类型已删除,旧名见 artisan-http/CHANGELOG.md 0.16.0 条目)迁移至框架内置链尾核心动作 `IgniteCore`,本文插入点/触发矩阵描述已同步。

> **时间**:2026-08-30
> **作者**:GLM-5.3-Flash + yansongda
> **状态**:经过人工审核确认(2026-08-30,对话内批准;三项决策点均按建议确认,见文末决策记录)。2026-08-31 依审查意见修订:解析插件路径纠正、reqwest 访问器契约由"推断"升级为"已验证"、CHANGELOG 纳入范围;方案方向未变。同日 plan-reviewer 审查通过(0 BLOCKER / 0 MAJOR / 6 MINOR),依 MINOR 意见补充 §3.3 HttpError 分发中监听器失败的错误链语义说明。再次依 PR 审查意见修订:触发矩阵拆分插件/解析失败两行、HttpStart 触发条件精确为"到达链尾核心动作执行点"、`EventListenerError` 增加 `original` 字段保留被顶替的原始错误、HttpEnd 响应体不可读的限制入档

## 1. 背景与问题

**现状**:artisan-http v0.15.0 是洋葱模型 HTTP 客户端框架,扩展点唯一入口是 `Plugin` trait(`artisan-http/src/plugin.rs:33`),内置插件链为 `StartPlugin → AddPayloadBodyPlugin → AddRadarPlugin`(0.16.0 起 HTTP 执行由框架内置链尾核心动作 `IgniteCore` 承担,见文首演进注记)。其 PHP 姊妹项目 yansongda/artful 具有完整事件系统(4 个生命周期事件 + PSR-14 分发器),本仓库 `artisan-http/docs/ARCHITECTURE.md` 将事件列为后续规划,尚未实现。

**困境**:

1. **无可观测性挂点**:用户要做请求日志、metrics、审计,必须编写完整 Plugin 并理解洋葱摆放位置,门槛高;且插件返回 `Err` 会直接中断主流程,不适合"旁路观察"语义。
2. **与 PHP 生态不对齐**:yansongda/pay 等下游生态依赖 artful 事件做横切逻辑,Rust 版缺少对应能力,影响迁移与心智一致。
3. **HTTP 执行失败无通知**:当前 HTTP 执行失败(网络错误等)只有返回值错误一条通道,无法旁路感知(如上报失败率)。

**目标**(约束条件):

- **PHP 版语义对齐**:4 个事件的触发点、携带数据、成功/失败路径行为与 artful 一致,并补充 Rust 场景需要的 `HttpError`
- **实例级、零全局状态**:延续 v0.14.0 将 `Artful` 实例化的方向,不引入任何 `static` 分发器
- **零新增依赖、默认零开销**:不注册监听器时无运行成本;不引入 feature 门控(依赖集不变)
- **非破坏性**:纯增量 API,现有签名与行为不变
- **最小 API 面**:同步监听器、注册顺序即执行顺序(无优先级)、仅经 builder 注册

## 2. 整体方案

**核心思路**:**事件分发器作为 `Artful` 实例字段,5 个分发点内嵌于框架既有代码路径——`Artful::artful()` 入口/出口分发 Artful 事件,框架内置链尾核心动作 `IgniteCore` 请求执行点前后分发 HTTP 事件,分发器经 `Rocket` 传载进插件链**。不新增插件、不改插件链结构、不改任何既有函数签名。

```
Artful { config, client, events: EventDispatcher }
   │
   │ artful(params, plugins)                       [artful.rs:134]
   │   ① dispatch ArtfulStart { params, plugins }  ← 链启动前,观测
   ▼
FlowCtrl 插件链:  StartPlugin → AddPayloadBodyPlugin → AddRadarPlugin
                                                                        │
                                              Rocket.events = Some(Arc<EventDispatcher>)
                                                                        │
              ② 链尾核心动作 IgniteCore(框架自动挂载,洋葱链固定终点)
                              ③ dispatch HttpStart { &mut rocket }      ← execute 前
                              ④ rocket.client.execute(radar)
                                   ├─ Ok  → ⑤ dispatch HttpEnd { &rocket }   ← 解析前
                                   └─ Err → ⑥ dispatch HttpError { &rocket, &err } → 返回 Err
                                                                        │
   ▼                                                                    │
   ⑦ dispatch ArtfulEnd { &mut rocket }  ← 链成功后,可改写 destination    │
   │
   └─ return rocket.destination.unwrap_or_default()
```

**文件结构**(✚ 新增 / ✏ 修改,均在 `artisan-http/`):

```
src/
├── event.rs          ✚ Event / EventListener / EventDispatcher(+单元测试)
├── error.rs          ✏ +EventListenerError 变体
├── rocket.rs         ✏ Rocket +events 字段(默认 None)
├── ignite.rs          ✚ 链尾核心动作 IgniteCore(execute 前后插入 HttpStart/HttpEnd/HttpError 分发,由原解析插件文件迁移)
├── artful.rs         ✏ Artful +events 字段;artful() 注入与 ArtfulStart/ArtfulEnd;Builder 注册方法
└── lib.rs            ✏ 导出 + 模块文档 + Send/Sync 契约测试
tests/event_test.rs   ✚ 集成测试(wiremock)
examples/event.rs     ✚ 使用示例
artisan-http/docs/ARCHITECTURE.md ✏;README.md / README.zh-CN.md(根目录与 artisan-http 各一对) ✏;CHANGELOG.md(根与 artisan-http 各一份,新增 0.16.0 条目) ✏
```

## 3. 详细设计

### 3.1 数据结构设计

**事件枚举**(Rust 接口契约,非实现代码):

| 变体 | 携带数据 | 可变性 | 对应 PHP 事件 | 触发时机 |
|------|---------|--------|--------------|---------|
| `ArtfulStart` | `params: &HashMap`, `plugins: &[Arc<dyn Plugin>]` | 只读 | `Event\ArtfulStart($plugins, $params)` | 链启动前,观测 |
| `HttpStart` | `rocket: &mut Rocket` | 可变 | `Event\HttpStart($rocket)` | 到达链尾核心动作执行点(`IgniteCore`,框架自动挂载)、`execute` 前(正常链中 radar 已构建;缺 `AddRadarPlugin` 时为 `None`,事件仍触发) |
| `HttpEnd` | `rocket: &Rocket` | 只读 | `Event\HttpEnd($rocket)` | `execute` 成功、direction 解析前(响应体不可读:body 消费权属于 direction 解析,仅可读 status/headers) |
| `HttpError` | `rocket: &Rocket`, `error: &ArtfulError` | 只读 | —(Rust 新增) | `execute` 失败,错误照常传播 |
| `ArtfulEnd` | `rocket: &mut Rocket` | 可变 | `Event\ArtfulEnd($rocket)` | 链成功后、返回 destination 前 |

**监听器与分发器**(签名级契约):

```
trait EventListener: Send + Sync + 'static {
    fn name(&self) -> &'static str;                      // 默认 "UnknownEventListener"
    fn on_event(&self, event: &mut Event<'_>) -> Result<()>;
}

struct EventDispatcher { listeners: Vec<Arc<dyn EventListener>> }
    // Clone 派生(共享监听器 Arc);手写 Debug(仅打印数量);Default 空表
    // pub fn add_listener(); pub fn len(); pub fn is_empty(); pub(crate) fn dispatch(Event<'_>) -> Result<()>
```

设计要点与理由:

- **`Event<'_>` 是借用视图而非数据拷贝**:变体内是引用(如 `HttpStart` 的 `&mut Rocket` 就是插件链正在流转的 Rocket 本体),监听器对它的修改在主流程真实生效;可写权限由**变体内部引用类型**决定(`&mut Rocket` vs `&Rocket`),`&mut Event` 外层包装只是让同一 Event 能被多个监听器顺序重借用的机械手段;
- **`'_` 生命周期的安全保证**:借用仅在 `on_event` 调用内有效,监听器无法把内部引用存入自身状态、返回或 spawn 到其他任务,编译期即排除;
- **enum 而非 trait object 载荷**:事件集合封闭(5 个),enum + `&mut Event` 传递可让 `&mut Rocket` 变体被多个监听器顺序复用(规避 `&mut` 单一所有权限制),且完全对象安全;
- **同步监听器**:分发点全部位于 async fn 内的瞬时点,同步闭包即可覆盖日志/metrics/改参数场景,不引入 `#[async_trait]` 复杂度(reqwest-middleware 的 hook 同为同步,先例);
- **监听器返回 `Result`**:与 PHP 版"监听器异常中断主流程"对齐,错误可传播。

### 3.2 流程/时序设计

链尾核心动作 `IgniteCore`(`src/ignite.rs`,由原解析插件代码路径迁移,经 `Artful::artful()` 自动挂载,洋葱链固定终点;已验证读过源码)伪代码:

```
run(rocket):                                              # 链尾终点,无 next
    if direction == NoRequest: return Ok(())              # 不发起请求,不触发任何 HTTP 事件
    dispatch(HttpStart { rocket })                        # 在 radar.take 之前,监听器可见 radar
    match client.execute(radar.take().ok_or(MissingRequest)?):
        Ok(resp)  → rocket.destination_origin = Some(resp)
                    dispatch(HttpEnd { rocket })          # 先于解析
                    …解析… → rocket.destination = Some(结果)
        Err(e)    → dispatch(HttpError { rocket, &RequestFailed(e) })
                    return Err(e)                         # 错误照常传播
```

**触发矩阵**(行为契约):

| 场景 | ArtfulStart | HttpStart | HttpEnd | HttpError | ArtfulEnd |
|------|:-:|:-:|:-:|:-:|:-:|
| 正常请求 | ✅ | ✅ | ✅ | — | ✅ |
| HTTP 执行失败 | ✅ | ✅ | — | ✅ | — |
| `NoRequest` | ✅ | — | — | — | ✅ |
| 插件失败(前向阶段) | ✅ | — | — | — | — |
| 插件不调 next 返 Ok | ✅ | — | — | — | ✅ |
| 链尾缺 radar | ✅ | ✅ | — | — | — |
| 解析阶段失败(execute 成功后) | ✅ | ✅ | ✅ | — | — |
| `Artful::raw()` / `shortcut()` | — / ✅ | 同左 | 同左 | 同左 | 同左 |

与 PHP 版差异说明:PHP 版请求失败时 `HttpEnd` 不触发、无失败事件;本设计补充 `HttpError` 以满足 Rust 显式错误处理场景的可观测需求,其余行为逐点对齐(已验证:经 GitHub API 读取 artful `src/Artful.php`,4 个 `Event::dispatch` 调用点分别为 `artful()` 管线前后与 `ignite()` 请求前后)。`MissingRequest`(链中缺 `AddRadarPlugin`)属请求前置失败,不触发 `HttpError`,与"HttpError 仅限 execute 失败"的范围界定一致。

### 3.3 错误语义设计

- 分发按注册顺序同步执行;**任一监听器返回 Err → 立即停止后续监听器,错误包装为 `EventListenerError { listener_name, message, source }` 向上传播,中断主流程**(对齐 PHP/Symfony 语义);
- 监听器 panic:按 Rust 惯例直接传播,文档注明,不做 `catch_unwind`;
- 特例(HttpError 分发):分发 `HttpError` 时若监听器自身返回 Err,向上传播的是 `EventListenerError`,原始 `RequestFailed` 保留在其 `original` 字段(错误链不丢失,下游可诊断/分支处理)--PR 审查修订(2026-08-31),取代初版"原始错误从错误链消失"的取舍;
- 新增错误变体镜像既有 `PluginExecutionError`(`error.rs:45-51`)的结构与 `#[source]` 用法。

### 3.4 兼容性设计

| 变更点 | 兼容性论证 |
|--------|-----------|
| `Rocket` 新增 `pub events: Option<Arc<EventDispatcher>>` | 非破坏:`params` 字段私有,用户无法结构体字面量构造 `Rocket`,只能经 `Rocket::new()`(默认 `None`) |
| `Artful` 新增 `events` 字段 | 非破坏:字段私有;`#[derive(Debug, Clone)]` 保留(`EventDispatcher` 手写 Debug 打印数量;Clone 共享监听器 Arc,文档注明语义) |
| `ArtfulBuilder` 新增 `event_listener(Arc<dyn EventListener>)` | 纯增量;**追加语义**(与 config/customize/client 的覆盖语义不同,doc 注明);不新增构造函数,统一走 builder |
| semver | 0.15.0 → 0.16.0(0.x 下 minor 表新增功能) |

### 3.5 契约验证状态

| 契约 | 状态 |
|------|------|
| `artful()` / `Rocket` / `Plugin` / `FlowCtrl` / `ArtfulError` 现有结构与行号 | **已验证**(读过源码) |
| PHP artful 4 事件触发点与携带数据、PSR-14 静默降级行为 | **已验证**(GitHub API 读取 `src/Artful.php`、`src/Event.php` 原文) |
| `reqwest::Request` 提供可变访问器(`method_mut`/`url_mut`/`headers_mut`/`body_mut`/`timeout_mut`)供 HttpStart 修改 | **已验证**(2026-08-31 读本机 registry reqwest-0.13.3 `src/async_impl/request.rs`,5 个访问器齐全;注意 `body_mut` 返回 `&mut Option<Body>`);Task 0 按此复核并落盘契约快照 |
| dev 依赖 tokio/wiremock 已具备 | **已验证**(`artisan-http/Cargo.toml:19-22`) |

## 4. 推进策略

```
v0.16.0(单 PR,分支 feature/event-system)
├── 阶段 A:核心类型 —— event.rs + error.rs + rocket.rs
│     验证点:cargo check --workspace --all-features 通过;event.rs 单测(空表 no-op/顺序/首错中止)通过
├── 阶段 B:分发点接入 —— ignite.rs(原解析插件迁移) + artful.rs + lib.rs
│     验证点:既有全部测试不回归;clippy -D warnings 通过
├── 阶段 C:集成测试与示例 —— tests/event_test.rs(触发矩阵 5 场景)+ examples/event.rs
│     验证点:cargo test --workspace --all-features 全绿
└── 阶段 D:文档与版本 —— ARCHITECTURE.md、README 双语四件套、CHANGELOG×2、版本号 0.16.0
      验证点:doctest 通过;grep 确认 README 双语同步
```

**回滚方案**:纯库代码变更、无配置/数据迁移,回滚 = `git revert` 该 PR(或发布 0.16.1 撤销文档);因 API 纯增量,已依赖新 API 的下游在 revert 后仅表现为未使用新能力,无行为破坏。

## 5. 风险与对策

| 风险 | 严重度 | 对策 |
|------|--------|------|
| 同步监听器阻塞 tokio worker(用户在监听器内做 IO/长任务) | 中 | doc 注释显著警示"监听器必须非阻塞,耗时任务请自行 spawn";`examples/event.rs` 给出正确示范 |
| 用户误以为 `HttpStart` 中改 `rocket.config` 影响本次请求(实际 radar 已构建,需改 `rocket.radar` 访问器) | 中 | doc 注释明确写出两条修改路径的差异;集成测试覆盖"HttpStart 加 header → 服务端收到" |
| reqwest `Request` 可变访问器缺失(原"推断"级风险) | 已消除 | 2026-08-31 实测 reqwest-0.13.3 源码:5 个访问器齐全(§3.5 已验证);Task 0 复核并落盘快照,lock 版本变化致签名不符时按设计性偏差停报 |
| 监听器错误中断主流程超出用户预期(旁路观察变主流程故障) | 中 | doc 明确"监听器返回 Err = 主流程 Err"语义;文档给出"忽略错误的日志监听器"标准写法 |
| `Clone` 后多实例共享监听器内部状态(如计数器)造成困惑 | 低 | `Artful::clone` 文档注明共享语义;示例用 `Arc<Mutex<…>>` 展示 |
| 监听器持有 `&mut Rocket` 越界缓存引用 | 低 | `Event` 借用生命周期使其无法逃逸出分发调用,编译期即排除;doc 说明 |

## 6. 监控与可观测性

本仓库为库项目,无独立线上监控;本功能本身就是向下游提供的可观测性能力(事件即挂点),由使用方按需接入,不设计库内上报。

## 决策记录(2026-08-30 人工确认)

1. **`HttpError` 范围**:仅 HTTP `execute` 失败时触发;插件/解析阶段失败不触发(保持最小面,PHP 无对应物)。
2. **`ArtfulEnd` 可变性**:`&mut Rocket`,保留 PHP 版改写 destination 的能力。
3. **版本号**:0.16.0。
