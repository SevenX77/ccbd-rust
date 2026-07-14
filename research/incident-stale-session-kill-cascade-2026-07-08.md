# 事故:对死 session 执行 kill --session 误杀活栈(2026-07-08 10:49)

## 现象
operator 对两个 0-agent 的死 session 行(sess_89659f3e=FAILED/IDLE_MASTER_EXIT、sess_93bb2469=KILLED)执行 `ah kill <sess> --session`,数秒内**活 session(sess_42b1a4fe)的 master(pid 2083624)进程死亡**,master_watch 按 ActiveWork 语义级联杀 4 worker;gen-2 revive master 30s 内 semantic-ACK 超时被 reap;session 终态 KILLED/OOM_OR_CRASH。全栈覆灭。tmux server 同期消失。

## 证据
- ahd journal 摘录:scratchpad cascade-incident-log.txt(session 03c8737c1);关键行:
  - 10:49:39.094 master process exited(gen 1, pid 2083624)
  - 10:49:39.149 cascade 中出现 sess_93bb2469 的 anchor stop 失败 WARN(unit not loaded)——两条 kill 流交错
  - 10:49:39.227 master death worker cleanup completed classification=ActiveWork workers=4
  - 10:50:09 gen-2(pid 2630040)exited + readiness(ack/semantic)timed out → reap(按 6f5ce38 R3 设计)
- DB 事实:a1-a4 全挂在 sess_42b1a4fe(活栈);两个死 session 的 master_pane_id 字段都是 **'%0'**——与活栈 master 的 pane id 相同(pane id 在各自 tmux server 命名空间从 %0 起,死 session 行里是**陈旧值**)。

## 根因假设(高置信)
`ah kill --session` 对死 session 的清理路径拿着该行**陈旧的 master_pane_id('%0')/tmux 定位**去杀,而这些定位在共享 tmux socket 上已被**活栈复用**→ 杀到活 master 的 pane。与已修的"pane-pid 校验防 PID 复用"(38bae3a,只覆盖探针路径)同族:**kill/cascade 路径缺 ownership 校验**(pane 现属谁的 generation/session 不验就杀)。

## 复盘中正确工作的部分
- master_watch 秒级捕获死亡、按 corrected 语义连坐 worker(防僵尸)、capture recovery intent、写 redispatch marker——全按设计。
- gen-2 revive 失败后被 readiness-timeout reap,没留孤儿壳(6-30 finding 的修复生效)。
- 产物零丢失:分支/design 文档/spec 全在盘(落盘纪律的回报)。

## 修复方向(待立项,归 orchestration-reliability)
1. kill/cascade 路径全部加 pane ownership 校验(pane 现挂 session+generation 匹配才执行 tmux kill;不匹配跳过+WARN)。
2. 终态 session(FAILED/KILLED)的 kill --session 应为纯 DB/单元清理,**不碰 tmux**。
3. `ah up` 的 "multiple running sessions match" 匹配器把终态 session 也算 running(本次连清两具尸体后仍报)——过滤条件修为仅非终态。
4. operator SOP 教训(已入 ah-operate 素材):杀 session 前先查 DB agent 归属与 pane 现势,连"看起来空的尸体"也不例外;整治动作избега在流水在途时做。

## 时间线残留
- 前任栈 in-flight 损失:a1 的 "fix a2.md leakage" job(FAILED,MASTER_EXIT)、a4 的二审/e2e job(FAILED)。均由新 master 重派补做。
