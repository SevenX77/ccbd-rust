# r1 统一评审 brief — Gateway A/B(Plan B Fake Gateway per-worker credentials)

你是本次 A/B 实验的**独立终审**。两个团队用**同一份冻结 brief**、从**同一 base commit `7bae3b1`** 出发,各自独立实现了同一个功能(Plan B Fake Gateway,根修 Claude worker OAuth 凭据 symlink 覆盖漏洞)。你的任务:用**同一把尺**审计两个实现的代码质量,给出 head-to-head 裁决。

## 唯一裁决问题

**哪个实现(A 还是 B)代码质量更高?** 只回答这一个问题。可靠性/成本/交接次数**不在你的裁决范围**(operator 另附账本)——你只看代码本身。

## 权威需求 / 设计(rubric 的来源,按此为准)

1. `.kiro/specs/ah-per-worker-credentials/design-rev.md` — 冻结设计(权威 #1)
2. `research/ab-experiment-gateway/task-brief-frozen.md` — 冻结任务 brief,含 AC-1~6 验收契约与 §3 约束

## 待审输入

- **实现 A**:`research/ab-experiment-gateway/ARM-A-diff.patch`(base `7bae3b1` → HEAD `7f5dc2b`,15 文件 +2877/-144,11 commit,验收测试 1132 行)。完整文件如沙箱可达:`/home/sevenx/coding/ccbd-rust-wt-gw-a`。
- **实现 B**:`research/ab-experiment-gateway/ARM-B-diff.patch`(base `7bae3b1` → HEAD `d55c26b`,14 文件 +1587/-185,11 commit,验收测试 454 行)。完整文件如沙箱可达:`/home/sevenx/coding/ccbd-rust-wt-gw-b`。

## 事实性 CI 结果(判定输入,非自动结论)

- **实现 B**:CI 全绿(`d55c26b`,test job pass)。
- **实现 A**:CI 至今**未绿**——`test` job 一直红在**同一个 src 单元测试** `platform::linux::scope::tests::test_spawn_command_scrubs_inherited_env_worker`(`src/platform/linux/scope.rs:656` panic),现象是本地 `--exact` 单跑过、CI 全量并行挂,历经 14 轮返工未收敛。

**你必须独立判定**:实现 A 的这个红,根因属于 **(a) 产品实现缺陷**,还是 **(b) 测试基础设施瑕疵(并行测试全局 env 串扰)**?这个定性直接影响质量评分——请亲自读 `test_spawn_command_scrubs_inherited_env_worker` 与它的兄弟单测、以及被测函数 `wrap_command_with_recovery_and_sandbox_overrides` 的 env 读取路径来论证,不要臆断。同样,CI 绿(B)只是**必要不充分**——绿不等于质量高,要看代码本身。

## Rubric(同一把尺量两臂,逐条落到 file:line 证据)

1. **AC-1~6 契约满足度**:逐条核对每臂是否**真实现**了契约(而非仅让测试变绿 / 测试造假)。
2. **强制 rollback 自检(必做,尤其对体量更小的那臂)**:亲自论证每臂是否**真堵住了根漏洞**——worker 沙箱内不得出现可刷新的真实凭据(AC-3:worker home 无 `.credentials.json`、无真 token 字节)。检查有没有"回滚/绕过"通道:比如去掉某个链接跳过后凭据是否仍从别处泄入、gateway 是否真的重写了 Authorization 且从不转发假 JWT、是否会写穿 `/mnt/c`。**不要只信测试名——读被测逻辑亲自验**。
3. **设计契合度**:与 `design-rev.md` 的偏差(哪臂更忠实,偏差是改进还是缺陷)。
4. **测试有效性**:测试是否有意义、是否能被平凡实现骗过、断言是否触及真契约。点评两臂验收测试体量差异(A 1132 行 vs B 454 行)——是更周全,还是过度/噪声/重复?
5. **正确性与健壮性**:并发、错误路径、边界。
6. **可维护性与复杂度**:过度工程 vs 过简;命名、结构、可读性。A 的整体体量近 B 的两倍(+2877 vs +1587),请判定这是"更完备"还是"更臃肿"。
7. **安全边界**:不泄真 token、UDS 身份、JWT 校验。

## 输出

写到 `research/ab-experiment-gateway/REVIEW-gateway-ab-verdict.md`,结构:
1. **每臂逐条 rubric 审计**(A 一节、B 一节,每条结论都带 `file:line` 证据引用)。
2. **实现 A 红根因定性**:(a) 产品缺陷 / (b) 测试基础设施瑕疵,附论证。
3. **分维度评分表**(同一 rubric,7 维,每维 A/B 各打分 + 一句依据)。
4. **head-to-head 裁决**:一句话哪臂代码质量更高 + 3~5 条决定性理由。
5. **各臂关键风险 / 遗留缺陷清单**。

## 纪律

- **只审不改**任何代码或分支。
- 每个结论必须可追溯:带 `file:line` 或 diff 引用;拿不准的点显式标注"需复验"而不是含糊带过。
- 不知道哪臂用了什么开发流程——**不要猜、也不要据此评判**,只看代码。
- 完成后在回复里给一句话结论(哪臂胜 + 核心理由),并确认 verdict 文件已落盘。
