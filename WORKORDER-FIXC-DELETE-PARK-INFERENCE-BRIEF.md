# 工单 Fix C(替换版)— 删除 unknown→park 生命周期推断(负 diff)

**收件:a1(antigravity,实施)。worktree `/home/sevenx/coding/ccbd-rust-wt-scanner`,分支 `feat/scanner-delete-park-inference`(off main 678eb9e,含已合入的 A/B 看门狗)。这是"负 diff"工单:删的应该比加的多。TDD 框线由 master(claude)钉死,a1 实施到框、不放宽。审计 a4。**

> 方向(用户裁决):pane 扫描的生命周期推断**整体删除,不是修补**。不认识的 pane 文本永远不产生状态;park 是强动作,只许已知对话框白名单进入;删除后的安全网是刚合入的 A/B 看门狗 + 人看语义。**不要**写幽灵匹配、**不要**写 banner 白名单——unknown 不再 park 之后它们自然失效,一行新硬编码都别加。

## 环境铁律(有事故背书)

- cargo 串行 `CARGO_BUILD_JOBS=1` + `--test-threads=1`;只 `cargo test --lib` 和 `cargo check --all-targets`;禁全量/集成/e2e;**严禁后台 shell 跑任何测试**;前台同步。
- 前台 commit,不 push 不 PR。遇阻写 worktree 根 `.operator-question` 停下。

## 病根(实锚)

`src/prompt_handler/gating.rs::classify_capture`:NoMatch(非已知 case)+ 无 idle marker + 无 prompt_experience 命中 → 返回 `PromptGateDecision::Unknown` → 经 `src/prompt_handler/resolve.rs` 把 agent transit 成 `STATE_PROMPT_PENDING`(park)。**这条"不认识就 park"路径是今天所有 ghost 卡死的元凶**:pane 上任何 scanner 不认识的文本(残影/横幅/回显)都会把 agent 钉成 PROMPT_PENDING 永久等人。

## 要做什么(删除 + 收紧,负 diff)

1. **删掉 unknown→park 路径**:`Unknown` 决策**不再** transit 成 PROMPT_PENDING。改为:只 `insert` 一条观测事件留痕(复用现有 `prompt_handler/events.rs::emit_unknown_prompt_detected` / `UNKNOWN_PROMPT_DETECTED`,payload 带**截断样本**供人复盘),然后**不产生任何状态**。fail-closed 硬约束:**绝不无声吞**——每条 unknown 都必须留下观测事件。

2. **park 白名单化**:transit 成 PROMPT_PENDING **只允许**由**已知交互对话框白名单**触发——即那些确实需要人类决策、不能自动 resolve 的已知形状(trust / update / 权限确认等)。你要:
   - 从现有 kb / 已知 case(`GateContext.kb`、`PromptKb`)里**枚举**出"需要人类决策 → 应 park"的已知对话框形状,列成显式白名单;
   - 其余一切(unknown、以及可自动 action 的 KnownAction)都**不 park**(KnownAction 继续走既有自动处理,不受影响)。
   - 在 commit message 里**逐条列出**这个 park 白名单及每条依据(哪个 kb case、为什么需要人)。park 从"黑名单兜底(不认识就 park)"变成"白名单进入(只有这些已知人机对话框才 park)"。

3. **不加任何 pane 硬编码**:不写 ghost 匹配、不写 banner 白名单、不写关键词表。unknown 不再 park 后,幽灵输入行、push-notification 横幅这些自然不再造成任何状态——靠"删除",不靠"识别"。若你发现自己在加 pane 文本模式匹配,停下——方向错了。

## 兜底(写进 commit message 的设计说明)

删除后若某个**真**需要人处理的对话框恰好不在白名单里被漏挡:派发会停滞 → 刚合入的 A/B 看门狗(QUEUED 饥饿告警 + PROMPT_PENDING 压制升级)会**告警** → 人去看 pane 用语义判断处理。这比 scanner 猜谜强,且失效方向安全(停滞+告警,不是伪造状态)。这是有意的权衡,不是遗漏。

## TDD 框线 · 验收(先 RED 后 GREEN)

回归**钉死今天两个真场景**(用脱敏 fixture,`tests/fixtures/` 已有基建;素材:幽灵输入行文本、push-notification 横幅帧,可从今天 a4 pane 取真样脱敏——无真实用户名/主机/state id/邮箱/真实路径,保解析语义):

- **C1(幽灵输入行,RED)**:pane = 幽灵/回显输入行文本(scanner 不认识)→ 断言:**不 transit PROMPT_PENDING**(不造状态)+ **发了 UNKNOWN_PROMPT_DETECTED 观测事件**。现码会 park → RED。
- **C2(push-notif 横幅,RED)**:pane = push-notification 横幅样本 → 同上:不 park、不造状态、有观测事件。现码 park → RED。
- **C3(任意 unknown,一般化)**:pane = 任意不认识杂文本 → 不 park、有观测事件(默认不 park)。
- **C4(白名单正面,回归)**:pane = 白名单内的已知人机对话框(如 trust/update,取真实已知形状)→ 断言**仍正常 park PROMPT_PENDING**(白名单没被误删)。
- **C5(KnownAction 回归)**:可自动处理的已知 case → 仍走既有自动 action,行为不变。
- **C6(留痕/fail-closed)**:所有不 park 路径 → 断言各有观测事件(绝不无声吞)。

先把 C1/C2 写成会 RED 的测试(现码 park→断言不 park 失败),记 RED 实证;删除路径后转 GREEN。保留现有 gating/resolve 测试全绿(除被删路径相关的断言按新语义调整,且在 commit message 说明每处调整)。

## 本地验证 & 收口

- `cargo check --all-targets` + `CARGO_BUILD_JOBS=1 cargo test --lib -- --test-threads=1`(前台,覆盖 gating/resolve/prompt_handler + 新断言)。不跑全量、不放后台。
- 回滚自检:`git diff` 应呈现**净删除**(删 park 路径 > 加的白名单枚举+测试);KnownAction 自动处理、A/B 看门狗、其它 provider 一律不动。commit message:删了哪条路径、park 白名单逐条依据、每处测试语义调整、RED→GREEN 实证、fail-closed 如何保证。
- 完成停下回报 master:commit 号 + park 白名单清单 + 净删除行数 + RED→GREEN 实证 + 测试名 + `--lib`/`check` 结果。master 亲验后派 a4 审。

拿不准/越界(尤其"某已知形状该不该进 park 白名单"拿不准):STOP,写 `.operator-question`,回报,别自行发挥。
