# r1 审核单 · 模块D网关嫁接实施(轨1,2026-07-12)

## 你的角色
你是 r1(claude,只审不写)。c1 独立实施(全链路自写自测)交付,**你是唯一审核关卡,c1 不自审**。请做回滚自检(逐条断言实跑复核,不只读 diff),这是唯一铁律。

## 被审对象
- worktree:`/home/sevenx/coding/ccbd-rust-wt-graft-c1`
- 分支:`feat/gateway-graft-modD`
- 待审 commit:`9c9f636`(base = main `7bae3b1`)
- 差异:`git -C /home/sevenx/coding/ccbd-rust-wt-graft-c1 diff 7bae3b1..9c9f636`(80 文件,+3225/-535)

## 权威设计(唯一裁决依据,新会话必读,按顺序)
1. `.kiro/specs/ah-per-worker-credentials/design-graft-frozen-2026-07-11.md`(冻结设计,7 分歧点裁决)
2. `.kiro/specs/ah-per-worker-credentials/design-graft-addendum-2026-07-12.md`(daemon 所有权补丁,5 点裁决——ClaudeGatewayService 挂 Ctx、seed credential 读写规范、生产 refresh 端点契约、master UDS 所有权、`master_command_with_env` 改动)
3. `.kiro/specs/ah-per-worker-credentials/incident-2026-07-11-wsl2-symlink-logout.md`(验收铁律:worker 不得持有写穿宿主 `/mnt/c` 凭据的链)
4. 参考(不作裁决依据,仅背景):`research/ab-experiment-gateway/REVIEW-gateway-ab-verdict.md`(A/B 终审,你之前的裁决——本次是嫁接实施,不是重新做 A/B 对比)

## 审核重点(逐条回滚自检,不要只信 c1 自报的"已跑通过")

1. **AC 表全绿**(冻结设计 §7.1 + 补丁追加项)——`tests/gateway_graft_acceptance.rs` 里列出的所有 `ac_*`/`addendum_*` 测试,你要**亲自重跑**而非读测试名字面意思采信:
   - `ac_single_flight_expired_workers_refresh_once`
   - `ac_zero_credentials_worker_home_has_no_real_token_bytes`
   - `ac_rewrite_upstream_sees_real_token_not_fake_jwt`
   - `ac_uds_channel_isolation_rejects_wrong_worker_jwt`
   - `ac_uds_header_limit_returns_400`
   - `ac_failure_cache_suppresses_invalid_grant_refreshes_and_records_event`
   - `ac_bridge_dynamic_ports_do_not_conflict`
   - `ac_bridge_wrapper_fail_fast_path_is_observable`
   - `ac_wsl_mount_guard_rejects_windows_credentials_path`(**incident 铁律的直接测试锚点,重点复核**)
   - `addendum_seed_reader_accepts_real_claude_oauth_schema_and_expired_zero`
   - `addendum_seed_writeback_rotates_refresh_token_atomically_for_linux_path`
   - `addendum_wsl_guard_skips_writeback_without_touching_windows_path`
   - `addendum_transient_refresh_errors_do_not_poison_failure_cache`
   - `addendum_production_refresh_maps_only_400_invalid_grant_to_invalid_grant`
   - `addendum_service_register_master_is_idempotent_across_reconcile`

2. **Kill List 复核**(冻结设计 §五):亲自 grep,不要信 c1 的"grep 无命中"自述——确认以下均未出现在 `src/`/`tests/`:
   - `worker_gateway_for_test`(A 的测试副本模式)
   - python3 heredoc 桥(任何 `python3 -c`/heredoc 拼接命令)
   - 全局 HMAC JWT 签名
   - 固定端口 `8206` 或 `port_from_slot_id` 哈希端口
   - `link_credentials` 函数或 `.claude/.credentials.json` 出现在 `PROVIDER_AUTH_WHITELIST`

3. **回滚自检式验证**(不是读代码猜,是真操作验证):
   - 亲自构造一个含 `expiresAt: 0`(登出残根)的假 host credentials 文件,验证 reader 是否如 c1 所述"优雅吃掉"而不 panic/不写穿。
   - 亲自验证 `ac_wsl_mount_guard_rejects_windows_credentials_path` 测试的断言是否真的覆盖"任何路径不得写穿 `/mnt/c`"这条 incident 铁律,而不是只测了字符串匹配这种表面实现。
   - 复核 `master_command_with_env` 的 sandbox_overrides 改动是否真的只对新增路径生效,没有意外改变现有 worker spawn 行为(回归检查)。

4. **cargo 政策合规**:确认 c1 本地只跑了 `cargo check`/定点测试,没有跑全量 `cargo test`(全量交给 CI)。

5. **不打补丁/不后兼容原则符合度**:检查是否有"为了不改动旧接口而加 shim/兼容层"这类模式——本次是全新代码路径,不应该有历史包袱。

## 裁决产出
- ACCEPT / REJECT,REJECT 需给出具体退回项(file:line + 断言),c1 会照此修。
- 若 ACCEPT:在会话里回报关键验证证据摘要(你亲跑的测试输出、grep 结果),master 会据此亲自 `git push` 触发 CI(你不 push、不开 PR)。
- 完成后把审核结论写入 `research/ab-experiment-gateway/r1-verdict-graft-impl-2026-07-12.md`。
