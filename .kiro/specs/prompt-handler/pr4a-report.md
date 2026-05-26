# PR4a Report: Agent 生命周期重画

PR4a 的核心目标不是“多加几个 prompt regex”，而是把 Agent 生命周期里的就绪判定从“看起来像 prompt”改成“物理上能接收输入”。这次改动把启动期、prompt 处理、READY 判定、ACK fallback 测试边界重新拉直，避免旧方案里 detect 扫屏、init probe、PR5 ACK 双状态机同时存在造成语义污染。

## 1. PR4a 改了什么

### 生命周期主干改为 outcome gate

旧路径里，`SPAWNING -> IDLE` 很大程度依赖 `InitGateProbe::detect()` 的文字扫描：看到某些 prompt/footer 字符就认为 provider ready。这在 TUI 里不可靠，因为屏幕上可能有历史 prompt、状态栏残留、footer 噪点，或者一个已经出现的 ready prompt 其实还不能接收输入。

PR4a 后，prompt-handling provider 的最终 READY 条件变成：

```text
屏幕看似 ready
  -> 必须执行 can-input probe
  -> probe 字符必须出现在输入框行
  -> BSpace 清理后 probe 必须从输入框行消失
  -> 才允许 mark IDLE
```

也就是说，`detect()` 现在只提供 ready candidate / prefilter 信号，不再单独决定 DB 状态。

### SPAWNING 期可以处理 prompt

旧路径里，prompt-handler 更像是 IDLE 后的保护逻辑；启动期遇到 trust/update/auth 类对话框时，系统容易等 init probe 超时。

PR4a 把 prompt-handler 接入启动循环：

```text
SPAWNING capture
  -> 已知 prompt: 执行动作清障
  -> 重新 capture
  -> ready marker: 做 can-input probe
  -> confirmed: IDLE
  -> unknown / not candidate: PROMPT_PENDING 或继续等待超时路径
```

结果是启动期已知框可以自动点掉，但点掉框本身不代表 ready。只有 can-input 确认后的 `ReadyConfirmed` 才能推进到 `IDLE`。

### Gemini / Claude probe 判据结构化

Codex 原本已经把 probe 锚到 `› x` 输入行；Gemini / Claude 之前还存在全屏 `contains("x")` 风险。状态栏、路径、模型名、上下文文案里经常天然包含 `x`，这会让 cleanup 后仍被误判为“probe 没清掉”，进而把 ready provider 错误打成 `PROMPT_PENDING`。

PR4a 收尾修复后：

| Provider | input candidate | probe echo |
| --- | --- | --- |
| Codex | `›` 输入行 | `› x` |
| Gemini | `* Type your message or @path/to/file` / `> Type...` 输入行 | 同一输入行 payload 恰好是 `x` |
| Claude | Sonnet / Opus / Haiku model marker + `❯` 输入行 | `❯ x` |

这把 probe 判断从“全屏任意字符命中”改成“输入行结构化命中”。

### PR5 双状态机断言清理

PR5 的旧关注点是 `WAITING_FOR_ACK / ACK -> BUSY` 链条。当前实现里这条 ACK 生产路径仍然存在：`WAITING_FOR_ACK -> BUSY` 仍可由 `ACK_STABILITY_WINDOW` 或 `ACK_VISUAL_DIFF` 推进。PR4a 没有彻底替换或统一 ACK 双状态机；本轮实际做的是在 ACK visual diff 的首次 meaningful diff 处插入 prompt scan，让 prompt-handling provider 遇到弹窗时先走 prompt-handler，并恢复 ACK 失败降级覆盖。

因此，本轮清理的是旧 R2/PR5 测试里对 ACK 内部链条细节的硬断言，避免测试继续把旧实现细节当成 PR4a 主契约；同时迁移保留仍有效的行为：

- dispatch atomicity 仍覆盖 job completion 和 `dispatched_at_seq_id`。
- mvp12 dispatcher lifecycle 仍覆盖完成后通知 waiter。
- 通用 state transition 单测不再拿 `ACK_VISUAL_DIFF` 作为样例原因。

### ACK fallback 安全路径补回

清理 PR5 断言时，`WAITING_FOR_ACK` 的一条仍有效安全路径曾被误删测试覆盖：ACK 期如果 pane 掉线或 tmux 抓屏失败，必须降级到 `CRASHED` 或 `STUCK`，不能永久卡在 `WAITING_FOR_ACK`。

现在新增 `ack_fallback_lifecycle` 覆盖当前语义：

- `WAITING_FOR_ACK + tmux_capture_failed_during_ack -> STUCK`
- `WAITING_FOR_ACK + pane_unregistered_during_ack -> CRASHED`
- 非 `WAITING_FOR_ACK` 状态下 fallback 不应误改状态

这不是复活旧 ACK 双状态机，而是保留生产路径上的安全降级契约。

## 2. 关键接口契约变化

### `InitGateProbe::detect()`

旧契约：

```text
detect(capture) == true  => 可以累计启动 ready / 最终 IDLE
```

新契约：

```text
detect(capture) == true  => 只是 ready candidate / diagnostic
prompt-handling provider 最终 IDLE 必须由 can-input Confirmed 决定
```

非 prompt-handling provider 仍可保留原 detect 语义；Codex / Gemini / Claude 不再允许 detect 独立决定 `IDLE`。

### `confirm_can_input`

`confirm_can_input` 现在是 READY 的硬门槛：

- `Confirmed`：输入框可接收 probe，且 probe 被 `BSpace` 清理干净。
- `NotCandidate`：不能当作 ready；对 Codex / Gemini / Claude 会走 `Pending`，而不是旧的 `NoActionNeeded`。
- `Failed`：执行失败，向上报 executor failure。

这个变化是 PR4a 的关键：看似 idle marker 命中但 probe 不确认，不再被吞成“无事发生”。

### `mark_idle_after_probe`

启动任务只在 `StartupPromptScan::ReadyConfirmed` 后调用 `mark_idle_after_probe`。如果刚处理完一个 prompt，只能返回 `HandledOrClear` 并重置 steady count；如果 probe 不确认，则不能推进到 `IDLE`。

### probe settle delay

probe 发键和抓屏之间、`BSpace` 清理和二次抓屏之间都加入 settle delay。原因是 TUI 刷新不是同步 API：刚发出 key 时立即 capture，可能看到旧屏或半刷新屏。settle delay 是为了让“回显”和“清理”这两个观察点具备工程确定性。

## 3. 测试签收

最终绿色测试套：

```text
pr4a_lifecycle_contract: 9 passed
prompt_handler_e2e: 10 passed
ack_fallback_lifecycle: 3 passed
mvp7_acceptance: 6 passed, 2 ignored
mvp11_acceptance: 6 passed
mvp12_init_probe: 6 passed
marker::matcher: 12 passed
prompt_handler::runner: 6 passed
prompt_handler::integration: 4 passed
```

这些测试分别覆盖：

- can-input probe + BSpace 契约
- dialog 期间不发送裸 probe
- ready marker 但 probe 不确认时不进 IDLE
- Gemini / Claude 状态栏噪点 `x` 不打穿 probe
- SPAWNING 期已知 prompt 自动处理
- SPAWNING 期未知 prompt 进入 `PROMPT_PENDING`
- ACK fallback 的 `STUCK/CRASHED` 安全降级
- 旧 init probe 测试作为 ready candidate / diagnostic 保留

## 4. 诚实签收：全量套件已知非 PR4a fail

全量 `cargo test` 还有两个已验证的非 PR4a regression：

1. `mvp6_acceptance::test_pidfd_kill_cleans_tmux_pane`

   这是确定性 pre-existing 的 kill -> tmux pane 清理 bug。clean HEAD 隔离也挂，归 PR3 / pidfd-tmux cleanup 方向处理。

2. `sandbox::bwrap::tests::test_build_args_binds_materialized_home_for_home_aware_manifest`

   这是测试并行改全局 `HOME` 的竞态。隔离单跑可过，归 sandbox-test-hygiene triage，不属于 PR4a 生命周期改动。

PR4a 本轮相关回归套件均已绿色。

## 5. 不在 PR4a：留给 PR4b

PR4a 只落确定性、本地可测的生命周期主干。以下内容明确不在本 PR：

- `prompt_experience` DB 自学习手册
- LLM / Haiku 慢路径判定
- 新增 HTTP client 或外部 LLM 调用链
- prompt 成功率自学习闭环

这些属于 PR4b / Phase 2-3。PR4a 的边界是先把状态机和 READY 判据拉稳，再在稳定主干上接自学习和 LLM fallback。

## 6. PM 视角总结

PR4a 把“Agent 是否 ready”的定义从视觉猜测改成了可验证结果：能不能实际输入、输入是否回显、回显能否清理。这个变化解决了启动期弹窗、TUI 状态栏噪点、旧 ACK 双状态机残留三类问题。

合并后的预期收益：

- provider 启动卡框时，系统能在 SPAWNING 期自动处理已知 prompt。
- 看似 ready 但不能输入时，不会误进 IDLE。
- Gemini / Claude 不会因为状态栏含 `x` 被误打成 `PROMPT_PENDING`。
- 旧 PR5 双状态机断言被清掉，但 ACK 失败安全降级仍有覆盖。
- PR4b 可以在更稳定的生命周期主干上继续做自学习和 LLM 慢路径。
