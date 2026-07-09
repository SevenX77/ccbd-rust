# a4 审计 brief — Fix C(删除 unknown→park + park 白名单化)

**收件:a4(claude,逐任务审计)。审 worktree `/home/sevenx/coding/ccbd-rust-wt-scanner`,分支 `feat/scanner-delete-park-inference`,**HEAD commit `805bb54`**(基线 origin/main 678eb9e)。只审只读,不改代码。**

## 背景(方向)

用户裁决:pane 扫描的生命周期推断**整体删除**——不认识的 pane 文本**永不产生状态**,只发观测事件;park(PROMPT_PENDING)是强动作,**只许已知对话框白名单进入**,黑名单兜底废除。删除后的安全网 = 已合入的 A/B 看门狗 + 人看语义。**不新增任何 pane 文本硬编码**(不写 ghost 匹配/banner 白名单)。

## 忽略项(不算问题)

commit 805bb54 误把两份 `WORKORDER-FIXC-*.md` / `WORKORDER-FIXD-*.md`(仓库根目录)commit 进去了——**审计忽略它们**,operator 推 PR 前会加清理 commit 移除,**不用返工、不列 must-fix**。

## 审计重点(逐条给依据,附行号)

1. **park 白名单完整性(头号)**:`is_park_whitelisted`(src/prompt_handler/integration.rs)只放行 `trust_path_01` / `codex_update_01` / `master_resolve_*` / `user_*` 四类 block_reason。核:
   - **对照 kb 全量 case 枚举**——有没有**该 park 却被漏掉**的已知人机对话框 case(即真需要人类决策、但不在这四项里的已知形状)?逐个 kb case 过一遍,列出你认为应/不应进白名单的判断。
   - brief 要求"逐条列依据",而 commit message 只列了清单没逐条论证——请你在审计里补上这个 gap 的评估(四项各自是否确实"需要人类决策",以及是否有遗漏)。
   - 白名单键在 **kb case ID / block_reason** 上(非 pane 文本),确认**零新增 pane 文本硬编码**。

2. **unknown / DepthExceeded 分支语义**:确认 unknown(非白名单)与 depth-exceeded 分支现在都走 `emit_unknown_prompt_detected` + `NoActionNeeded`(**不造状态、不 park**),且**必发观测事件**(fail-closed,绝不无声吞)。特别核 **DepthExceeded** 分支语义是否正确(它也不该 park)。

3. **KnownPrompt 空 actions 死锁修复**:确认"KnownPrompt 但 actions 为空"改为立即 `Pending`(带 case_id 作 block_reason)走白名单判定,消解了 SameHash 死锁;且该改动没误伤正常 KnownAction 自动处理。

4. **test_fix_c_compliance 真驱动(非空转)**:核该测试**真穿过本次改的生产判定**、非自指/非 vacuous:
   - **C1(幽灵)/C2(横幅)/C3(任意 unknown)**:三场景都断言 agent **不 park**(state 保持非 PROMPT_PENDING)+ **发了 UNKNOWN_PROMPT_DETECTED 观测事件**;
   - **C4(白名单对话框)**:断言**仍正常 park** PROMPT_PENDING(白名单没被误删);
   - **C5(KnownAction)**:断言仍走自动处理不 park(STATE_IDLE)。
   - 确认这些断言真由生产 whitelist 逻辑驱动(改回旧码会红),harness 真起 pane/DB/scanner。

5. **blast radius / fail-closed**:改动面应仅限 prompt_handler(integration.rs / runner.rs)+ 测试;A/B 看门狗、KnownAction 自动处理、其它 provider 不动。所有不 park 路径必有观测事件。

## 铁律(审计受约)

- 你沙箱无 rust toolchain,**静态审 + 读代码**即可(a1 本地 `cargo test --lib` 已跑;CI 是最终门)。
- **严禁后台跑任何测试**,不碰活栈,只读不改。

## 产出

PASS / REJECT 明确判决 + 逐条依据(附行号)。must-fix 明列"哪行、应改成什么"(尤其若发现白名单有漏/DepthExceeded 语义错/测试空转)。审完停下回报 master。
