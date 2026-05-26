# mvp13 e2e 验证 checklist

## 适用范围

mvp13 stage 0/1/2/3/4 主线集成验证。**只覆盖 NO_SANDBOX 模式**——sandbox 模式 (Stage 5 onboarding) 待 NO_SANDBOX 跑通后单独 e2e。

## 跑法

```bash
cd /home/sevenx/coding/ccbd-rust
bash scripts/mvp13-e2e-no-sandbox.sh
```

## 期望输出（每步）

| 步骤 | 预期 | 失败排查 |
|---|---|---|
| [1/8] cleanup | "fresh state_dir at target/dev_state/" | 如有 stale ccbd 进程跑着，脚本会先 kill 它再继续 |
| [2/8] build | "Finished `release` profile" | cargo build 失败：检查依赖 / 之前未 commit 改动 |
| [3/8] start daemon | "daemon_pid=NNN" + 日志在 /tmp/ccbd-e2e-*.log | 日志有 panic：检查 schema migration / state_dir 残留 |
| [4/8] daemon ready | "daemon ready after Ns" | 10 秒不 ready：daemon 起不来，看 log |
| [5/8] spawn 4 agents | "session_id=..." + 30-60s 等到 IDLE | 卡住：codex/gemini/claude binary spawn 失败 / first-run prompt 阻塞（虽然 NO_SANDBOX 不该弹但万一）|
| [6/8] ps verify | "4 agents IDLE" | 少于 4：某 provider 启动失败，看 ah logs <agent_id> |
| [7/8] ask reply | reply 含 "echo from a1" 干净文字（不是 ANSI 乱码） | 乱码：Stage 4 distill_reply 没生效，check src/db/jobs.rs::distill_reply |
| [8/8] kill | session 清干净 | zombie：检查 systemd cgroup / tmux session 残留 |

## 验收标志（4 个硬条件）

- [ ] **AC1** Stage 0 (multi-codex)：ccb ps 看到 a1=codex 跟 a2=codex 都 IDLE，两个 codex 独立 sandbox HOME（在 NO_SANDBOX 模式两者都用 host home，所以这条只在 sandbox e2e 才能严格 verify；NO_SANDBOX 下只验证不报错）
- [ ] **AC2** Stage 1 (systemd 总闸)：daemon kill 后所有 agent 自动 cascade 死（用 `pkill -9 ccbd`，5 秒后 `pgrep -af "codex|gemini"` 不应有 spawn 出来的 agent）。如果 user systemd 未在跑可能 fallback 行为不同，检查 doctor 输出
- [ ] **AC3** Stage 3 (PaneDiffWatcher)：spawn 一个真不会自然 IDLE 的命令（比如 `ah ask a1 "sleep 400"`），等 5 分 + 30s 看 ccb ps 是否 a1 状态切到 STUCK
- [ ] **AC4** Stage 4 (reply distill)：ask 的 reply 是干净文本不是 ANSI escape stream

## 已知 race 跟 limit

1. **跟 user 当前 Python ccb 共享 host config**：codex/claude/gemini host config (~/.codex / ~/.claude.json / ~/.gemini) 跟用户本身在跑的同 provider 共享，可能 short race。脚本短跑（≤2min）接受 race
2. **NO_SANDBOX 不验证 sandbox 模式**：sandbox 模式的 first-run onboarding (Claude bypass permission warning / Gemini auth picker) 这个 e2e 跳过。等 user 反馈后定 stage 5 真范围
3. **真 spawn 4 agent**：spawn codex/codex/gemini/claude 占 host 一定资源（CPU + mem + tmux server），跑完 cleanup
4. **没改 ah.toml master.enabled**：脚本用临时 config (`/tmp/ccb-e2e-*.toml`) 跑，master.enabled=false。原项目 ah.toml 不动

## 跑出问题反馈给 master Claude 的 minimal info

1. 跑脚本的完整 stdout（重定向 `bash scripts/... 2>&1 | tee /tmp/e2e-out.log`）
2. daemon log（脚本里提示了路径 `/tmp/ccbd-e2e-*.log`）
3. `cat target/dev_state/ccbd.sqlite | sqlite3 -- "select * from events order by seq_id desc limit 50"`（last 50 events）
4. `pgrep -af "codex|gemini|claude" | head -10`（zombie agents）
5. `systemctl --user list-units --all | grep -E "ccb-|ccbd-"`（systemd state）

## 后续 stage（NO_SANDBOX e2e 通过后）

1. **commit checkpoint**（按 user standing rule：测试通过 + 方向对齐）
2. **Stage 5 sandbox onboarding**：根据 sandbox e2e 实际弹窗补 mirror / SandboxOnboarder pattern
3. **Stage 7 sandbox e2e**：跑 sandbox 模式（不带 `CCBD_UNSAFE_NO_SANDBOX=1`）验证 stage 5 + 整体
4. **Stage 3D marker token**：如果 e2e 撞 marker_pattern 假阳性才加（per-provider 可选）
