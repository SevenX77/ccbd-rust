# 病例:respawn 后 pane/session 命名错位(2026-07-11,观察日志 #49①)

## 现象(实证)

对 DISPATCHED job 执行 `ah cancel` → agent kill+respawn 路径,respawn 出的席位落进**错误命名**的 tmux session:

```
# respawn 后的 pane map(tmux list-panes -a)
agent_g1-m1 %2 2701280   ← 真 g1-m1(原有)
agent_g1-m1 %7 2824083   ← 实为 g1!(ah ps 认 2824083=g1)
agent_g2-m1 %4 2701698   ← 真 g2-m1(原有)
agent_g2-m1 %8 2824106   ← 实为 g2!
```

- DB(`ah ps`)对 pid 的归属是对的(2824083=g1),但 tmux session 名是错的(`agent_g1-m1`),且与真 g1-m1 的 session **重名并存**。
- 干净整栈重启(`ah stop` + `ah start`)后命名全对齐——bug 只在 cancel→kill→respawn 路径。

## 危害

感知层(pane scanner / 就绪复查 / capture 取证)按 **session/pane 名**定位目标。重名并存时:
- g1 的完成/停摆信号可能被记到 g1-m1 头上(或反之);
- master 按名投递 `/clear`、prompt 会投错席;
- 与 #30(2026-07-10:realign 后新 g2 的 session 名错标 `agent_g2-m1`)同族——**第二例,且升级为重名并存形态**,同族两例=结构病标准。

## 修向(设计约束,非补丁)

- session/window 名必须从 **slot 身份**原子推导,respawn 路径与首次 spawn 走同一条命名代码路径(现状疑似 respawn 复用了错误的 slot 上下文或邻位模板)。
- 回归测试:kill/cancel 触发的 respawn 后,断言 `session_name == agent_<agent_id>` 且全栈 session 名无重复(契约边界可观测:tmux list-sessions 输出)。

## 复现路径

活栈上:`ah ask <agent> ...`(job 进 DISPATCHED)→ `ah cancel <job>` → 观察 respawn 出的 tmux session 名。
