# 监督方物理证据补充(step-9 dogfood)— 你 ah ask/pend 看不到的部分

监督方在 tmux 外抓到的 worker pane + journald 实证。补你诊断缺的那块。

## 1. ahd 真日志在 journald,不在 ahd.log(那是 29 字节 stub)
```
journalctl --user -u ahd.service --since '8 min ago' --no-pager
```
沙箱里 `sqlite3` 没装(你撞的 exit 127),别走 sqlite,直接 journald grep。

## 2. 每厂商完成路径(监督方从 journald 已 grep 出来)
| agent | provider | 完成路径(journald reason) | 判定 |
|---|---|---|---|
| a4 | claude | **`log signal`**(transcript stop_reason)→ ah ps sub_state=**HookEvent** | log-event 路径работает,但**不是 hook-push RPC** |
| a3 | antigravity | **`UI-only completion` `reason="unsupported_provider"`** → sub_state=Matched | log signal 不支持 antigravity + **没有 push notify 到达**,退化到旧 pull 抓屏 |
| a1 | codex | trust-modal 卡住→监督方手动 trust 后才出 reply(reply_len=146),sub_state=HookEvent | 见 §3,**非干净 spawn** |
| a2 | codex | 没派任务,**此刻仍卡在 trust modal**(pane 实证) | 同 §3 |

**关键**:整个 dogfood 期 journald **零条 `ah agent notify` RPC 到达记录**(grep `notif` 只有 "skipped startup notification" 噪声)。也就是说注入的 Stop hook **没有真的 fire 并打到 ahd**。a4/a1 的 HookEvent sub_state 来自 log-signal 路径,不是 hook-push RPC。**#3 的头号卖点(hook agent.notify push)本次未被证明在 fire。**

## 3. codex 致命 onboarding gap(监督方 pane 亲眼实证)
a1、a2 两个 codex worker spawn 后**都卡在 codex "Hooks need review" 信任弹窗**:
```
Hooks need review — 1 hook is new or changed.
› 1. Review hooks   2. Trust all and continue   3. Continue without trusting (hooks won't run)
```
- ah 的 idle 检测把这个弹窗误判成 IDLE/Matched(检测 gap)。
- 派任务→prompt 排在弹窗后面(PROMPT_PENDING)永不执行。
- 监督方手动给 a1 选了 "2. Trust all and continue" 才解开 → 但 codex **重渲染 TUI,排队的 pong prompt 被丢**;a1 的最终 reply 是后续才出的,**不是干净 spawn 路径**。a2 至今仍卡弹窗。
- 解开后 codex 报弃用警告:**`[features].codex_hooks` is deprecated(codex v0.135.0),要用 `[features].hooks`**。`src/provider/home_layout.rs:849` 写的就是 `codex_hooks` —— 注入的 hook 在新版 codex 可能根本没激活。

## 4. 收敛判断(供你 root-cause,监督方不替你定方案)
本次 dogfood **未闭合 #3 目标**。暴露的 gap:
1. **hook-push RPC 全程没 fire**(三厂商都没 notify 到 ahd)— 头号 must-fix。
2. **codex trust-modal 把 worker 堵死**(dead-on-arrival)+ ah idle 检测误判弹窗为 idle。
3. **codex `codex_hooks` flag 在 v0.135.0 已弃用**(home_layout.rs:849)。
4. **antigravity 在 completion 路径是 unsupported_provider**(log-signal 不认它),即便 hook 注入了也只走 UI pull。

按 SOP-08 step-10:gap 找到 → 自驱回 research/design/impl 修一轮 → rebuild(debug)→ 重启 re-dogfood,直到三厂商真 hook-push 闭合。**不要 merge**,修完再报监督方。这是工程闭环,你 + a1/a2 主导;监督方只供证据 + 驱动 + 判收敛。
