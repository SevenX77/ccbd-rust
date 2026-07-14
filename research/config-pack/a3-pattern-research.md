# 编程场景最佳实践与工程规范设计调研 (a3-pattern-research.md)

本文件由 a3 worker (explore/深度调研角色) 独立撰写。基于业界主流的前端状态管理、服务器状态缓存、事件驱动架构规范，对用户提出的两大类典型工程问题进行深度调研，并对“业界最佳实践是否存在”这一核心元问题进行判定。**拒绝空泛表述，本报告直接给出可被机器检查的铁律与门禁点。**

---

## 问题一：服务端状态访问 + 事件驱动缓存失效 + UI 副作用边界

### 1. 业界成名标准解法 (Named Patterns / Libraries)
* **服务器状态管理 (Server-State Management)**：
  业界公认「前端本地 UI 状态（如弹窗是否打开）」与「异步获取的服务器状态（如数据库中的 Skill 列表）」是两类本质不同的数据。像 **SWR** 或 **TanStack Query (React Query)** 便是专门为此设计的库。它们充当了前端的数据缓存代理，接管了所有数据的读取、缓存、过期和同步工作。
* **过时而重新验证机制 (Stale-While-Revalidate - SWR)**：
  这是一种网络缓存策略。当组件请求数据时，SWR 优先返回缓存中的“旧数据（Stale）”以秒开界面，同时在后台静默发送请求获取“新数据（Revalidate）”，并在新数据到达后自动更新 UI，彻底省去了 Loading 动画。
* **按 Key 粒度失效 (Hierarchical Cache Keys / Fine-grained Invalidation)**：
  SWR 将每个接口请求的 URL + 参数组合成一个缓存 Key（例如 `['/api/skills', skillId]`）。当执行写操作（如更新 Skill `123`）后，仅通过 Key 对 `['/api/skills', 123]` 执行失效（Invalidate）或突变（Mutate），其他未修改的数据（如 Skill `456`）的缓存继续有效。
* **乐观更新 (Optimistic Update)**：
  在发送写请求前，前端直接根据用户输入「假装请求已成功」并更新 UI。若服务器返回成功则确认，返回失败则回滚到上一状态，实现极致的流畅交互。

### 2. “一刀切全量失效” vs “按 Key 粒度失效”的真实取舍

| 维度 | 一刀切全量失效 (Global Purge) | 按 Key 粒度失效 (Fine-grained Invalidation) |
| :--- | :--- | :--- |
| **同步安全性** | **极高**。由于所有缓存都被擦除重新获取，绝无数据漂移或旧缓存残留的可能。 | **中等**。如果 Cache Key 设计不匹配，或者依赖关系未更新，容易出现局部 UI 缓存漂移。 |
| **性能开销** | **极差**。每次微小的修改（如改个名字）都会触发全局所有组件的 cascading refetch，导致 CPU 暴涨、网络风暴（即使是 localhost 也会卡顿 FastAPI 服务端）。 | **极好**。只拉取发生改变的数据，未改变的数据保持缓存，无多余 I/O。 |
| **代码复杂度** | **极低**。直接调用全局 `mutate(() => true)`，无需考虑数据依赖和 Key 设计。 | **中高**。需要统一规范 API 路径与 Cache Key 的层级命名，并维护写操作与读 Key 的联动关系。 |

* **取舍判定**：对于运行在 localhost 上的桌面端应用（Tauri），虽然网络带宽不是瓶颈，但**频繁的全量渲染和串行 I/O 依然会造成严重的进程 OOM 或 CPU 拥堵**。因此，在初期可以为安全采用“一刀切”，但**项目一旦涉及复杂 DAG 画布或多文件关联，必须立刻 cutover 到“按 Key 粒度失效”**，否则性能会呈指数级崩溃。

### 3. SWR 在本技术栈（Tauri + FastAPI）下的正确用法
* **Cache Key 设计规范**：
  设计具有**层级关系**的 Key 命名体系。
  * 列表：`/api/skills`
  * 单个详情：`/api/skills/{id}`
  * 关联子资源：`/api/skills/{id}/nodes`
* **Mutate/Invalidate 粒度控制**：
  利用 SWR 提供的 `mutate` 匹配器函数进行局部失效。例如，当更新了某个 Skill 的节点信息，应只失效与该 Skill 关联的 key，而不是清空 `/api/skills` 列表本身：
  ```typescript
  // 仅失效该 skill 相关的缓存
  mutate(key => typeof key === 'string' && key.startsWith(`/api/skills/${skillId}`));
  ```
* **Revalidate 触发时机**：
  在 Tauri 桌面应用中，强烈建议**关闭**默认的 `revalidateOnFocus`（聚焦时重新验证）与 `revalidateOnReconnect`（重连时重新验证）。因为 localhost 不存在断网，且用户频繁切换窗口（Focus）会导致系统后台无端触发大量重复编译/获取动作。**仅在显式写操作（Post/Put/Delete/Mutate）发生后，手动触发 targeted revalidation**。

### 4. 写入配置包的可执行铁律与机器门禁点
* **SWR 消费铁律**：
  > **「所有组件均不允许直接发起 Raw Fetch 或 tauri.invoke 数据获取请求；所有服务器状态必须通过经过封装的 SWR React Hook 进行读取，并在 Hook 内部声明唯一的 Cache Key。」**
* **机器可检门禁点 (Static CI Gates)**：
  * **ESLint 规则禁止裸 Fetch**：配置 `no-restricted-imports` 或编写 ESLint 规则，禁止在 `/src/components/` 目录下直接 `import { invoke } from '@tauri-apps/api/core'` 或直接调用全局 `fetch`。
  * **API 消费出口检测**：通过 AST 扫描，验证所有数据接口的消费点必须包裹在名为 `use*Query` 或 `use*Data` 的自定义 hook 中。

---

## 问题二：单一真相源 + 多组件投影 (Single-Source compile result + event-driven projection)

### 1. 业界成名模式 (Named Patterns)
* **派生状态选择器模式 (Derived-State Selector / Projection)**：
  在前端只维护一份唯一的原始编译结果（Single Source of Truth，例如 `/api/compile` 的全量 JSON 数据）。每个组件（如错误小红点、大纲树、文件编辑器）**绝不保存独立的状态副本**，而是通过“选择器（Selector）”从这份全量 JSON 中实时计算并投影出自己需要的切片数据。例如：
  ```typescript
  const errorCount = useCompileResult(data => data.errors.length);
  ```
* **事件驱动缓存突变 (Event-driven Mutation)**：
  后端（FastAPI）在文件发生变化时异步执行编译，编译完成后通过 WebSocket 或 SSE（Server-Sent Events）向前端广播 `COMPILE_SUCCESS` 事件。前端的中央事件监听器捕获该事件后，**主动触发该 API 对应的 SWR Key (`/api/compile`) 进行原地 Mutate**。

### 2. 性能与全局触发的业界解答
* **需要触发全局编译（Compile）吗？**
  **需要，但不能在每次按键（Keystroke）时同步触发，且不能同步阻塞 UI。**
  * **防抖/防并发 (Debounce/Coalesce)**：前端应在用户停止输入 500ms 后（或显式 Ctrl+S 后）再发送编译请求；或者后端文件监视器在收到变更事件时，通过队列做 Coalescing（把 100ms 内的多个文件变动合并为一次编译）。
  * **异步非阻塞**：编译是耗时副作用，必须在后台线程（或 sidecar 中）跑。前端 UI 组件只订阅编译结果状态，在编译中状态显示 `compiling` 动画，编译完自动更新。
* **怎么更新数据不影响性能？**
  * **切片订阅 (Shallow Equality Check)**：如果使用派生状态选择器，React 组件应该只在它所订阅的“那个切片”发生改变时才重新渲染。例如，即使编译结果中其他代码行改变了，只要“错误数量”依然是 0，订阅 `errorCount` 的红点组件就不会触发重新渲染。这在 `Zustand` 或封装好的 `useSWR` 中可以通过传入比较器函数（如 `shallow` 比较）轻松实现。

### 3. 如何从结构上杜绝“组件自搓编译/自发请求”的反模式
* **架构层隔离（单向数据流与适配器模式）**：
  将“触发编译的副作用”和“展示编译诊断”在组件层级彻底剥离。
  * 组件是**纯渲染视图（Presentational View）**，它们只通过只读 Hook 读取编译结果。
  * 组件**没有触发编译的权限**。触发编译的行为由底层的 `FileSystemWatcher`（前端或 Tauri Rust 层）在后台独立运行。
* **门禁策略**：
  限制编译触发 API（如 `/api/compile` 的 POST 接口，或 `runCompile` 函数）的物理调用范围。

### 4. 写入配置包的可执行铁律与机器门禁点
* **投影消费铁律**：
  > **「禁止任何 UI 组件自主发起编译触发动作或手动过滤/解析编译报错；所有组件必须使用共享 Hook 提供的只读 Selectors（如 `useCompileDiagnostics`）消费编译投影数据。」**
* **机器可检门禁点 (Static CI Gates)**：
  * **编译端点写操作阻断**：在 ESLint/CI 中设置规则，禁止 `/src/components/` 目录下的文件调用任何名为 `triggerCompile` 或向 `/api/compile` 发送 `POST/PUT` 的 API 函数。
  * **文件一致性检查**：检测所有引用 `/api/compile` 返回值的组件，确认其解构赋值中不含有“对全量编译数据的本地深拷贝或 state 保存”代码，必须保证直接消费派生值。

---

## 五、 核心元问题解答：标准解法的覆盖比例与配置包架构定位

对于用户“业界一定有最佳实践”这一绝对判断，我的结论如下：

> **该判断在「应用支撑层与通用工程模式」上 100% 成立，但在「领域核心模型与物理编排」上 0% 成立。整体而言，一个优秀的配置包有 70% 可以通过“标准模式知识库”直接查表覆盖，剩余 30% 必须依靠针对性的定制架构设计。**

### 1. 70% 的通用工程问题（可直接查表应用）
这部分属于**应用开发的基础设施与交互边界**，业界有数十年沉淀的稳定解法：
* **数据获取与状态同步**：SWR、React Query、Zustand、Redux Selectors。
* **网络与并发控制**：防抖（Debounce）、节流（Throttle）、乐观更新。
* **测试与安全防护**：TDD 红绿矩阵、环境隔离、UDS 套接字权限。
* **设计呈现**：Design Tokens、语义化 CSS、Radix/shadcn 组件库。
* *结论*：这些问题的答案是死的，配置包应当把它们写成**硬性铁律**（如前文一④、二④），并通过 **ESLint 规则、预写好的 Hooks 模板、以及 CI 静态检测**进行物理守门，阻止 AI 乱写代码。

### 2. 30% 的领域专有架构问题（必须真设计）
这部分属于**智能体编排系统（SCS）特有的、与物理世界或 AI 本质特性相关的痛点**，业界没有任何开箱即用的库或教科书，必须基于第一性原理自己设计：
* **智能体越权与人机信任边界**：Worker 自派单、僵尸进程防范、物理验证（Recapture/物理 ls 校验）。
* **多智能体共享工作树并发冲突**：在共享同一个 Git 仓库 CWD 且缺乏隔离的状况下，如何避免并发 cargo 构建死锁（如 flock 锁继承死锁问题）。
* **AI 记忆的持久化与精准召回**：面对超长上下文或中断复活场景，如何保障 OAuth Token 透传与 Memory 蒸馏而不发生漂移。
* *结论*：对于这 30% 的核心架构问题，配置包不可能提供一个简单的“引入某个 npm 库”的解法，必须通过**设计底座架构（如 `ah` 内核的状态机跳转、UDS 协议设计、SCS 规范文件）**来进行硬性承载。

### 3. 配置包的形态定位
由此，我们的编程场景配置包，其定位不应仅仅是一个“Prompt 提示词模板”，而应当是一个 **“最佳实践模式库（ESLint/SWR标准Hooks预设）” + “智能体边界防护网（Daemon Hooks/静态CI检测）”的软硬结合体**。用标准知识库直接掐死 70% 的低级架构漂移，把宝贵的系统设计精力集中在解决那 30% 的智能体特异性硬骨头上。
