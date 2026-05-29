# ccbd Keeper Spawn 死循环 — 事故分析与 ccbd-rust 需求设计输入

> **来源**:2026-05-26 VPS 内存压力事故的物理取证分析
> **用途**:作为 ccbd-rust(Rust 重写版 ccbd)的可靠性需求设计输入
> **性质**:基于物理日志/进程实证的工程缺陷分析 + 行为契约规格。**这是设计输入材料,不是最终 design spec**——正式 design 由 ccbd-rust 的设计流程产出。

---

## 1. 事故背景

2026-05-26 约 05:26–05:49 (UTC),一台 7.7 GiB RAM 的 VPS 上同时运行多个 claude/gemini/codex + 多个 ccbd 实例。期间所有 claude 进程被内存压力按住卡死,systemd-oomd 于 05:49:47 杀掉一个累计 15h16m CPU 的 claude 进程以保护整机。手动 kill ccb 后机器恢复。

事后取证确认:卡死的源头**不是**被杀的那个 claude,而是 **agent-harness 项目的 ccbd 陷入了 keeper spawn 死循环**,把整个用户 session 的内存压力(PSI)顶到 81%,越过 oomd 的 80% 阈值。

## 2. 物理取证证据

| 证据 | 内容 |
|---|---|
| ccbd stderr 日志 | agent-harness 的 `.ccb/ccbd/ccbd.stderr.log` 在 ~23 分钟内刷出 **606 个 Traceback**,大小 1.68 MB |
| 稳定错误 | 末尾循环稳定为:`OwnershipConflictError: ccbd lease is held by pid=1545308 generation=224: healthy` |
| 健康进程存在 | 同时已有一个**健康的** `main.py`(pid 1545308)正常持有 lease(generation=224, healthy)|
| 冲突抛出点 | `lib/ccbd/services/ownership.py:98` `verify_or_takeover()` 抛 `OwnershipConflictError` |
| oomd 记录 | `Killed .../claude-*.scope due to memory pressure ... being 81.21% > 80.00% for > 20s with reclaim activity` |
| earlyoom | 全程未触发(可用内存 32–35%,远高于其 10% 阈值)→ 问题是 **PSI 压力维度**,非绝对内存耗尽 |
| 自停时刻 | 循环在 05:49:51 停止(oomd 释放压力 + 手动 kill ccb 后),1545308 成为稳定持有者 |

## 3. 根因(现有 Python ccbd 的缺陷)

keeper 的 reconcile / spawn 决策逻辑与 ownership lease 状态**没有对齐**:

1. keeper 决定"是否需要 spawn 一个 main.py daemon"时,**没有先检查 lease 是否已被一个健康进程持有**;它只看"我刚 spawn 的子进程还在不在",看不到那个已经健康运行、持有 lease 的 main.py(1545308)。
2. 于是它持续 spawn 新的 main.py;每个新进程启动时在 `ownership.py:98` 发现 lease 已被健康进程占用,抛 `OwnershipConflictError` 立即退出。
3. keeper 检测到子进程退出,**立即再次 spawn**,无退避、无熔断,形成高频空转循环。
4. 每次 spawn 都要启动一个完整的 Python 解释器(加载解释器 + 模块),高频反复启动持续抖动内存与 CPU,把用户 session 的 PSI 压力顶到 81%。

### 与历史 spawn 风暴的同源关系

本次缺陷与历史上另一类 spawn 风暴是**同源 bug 的不同表现**,根都在 keeper 的 spawn 决策缺乏权威状态来源 + 缺乏失败退避/回收:

- **历史 bug A(进程堆积型)**:`lib/ccbd/ccbd/daemon_process.py:42-61` `_wait_for_ccbd_ready` 在 5s 就绪 timeout 后**只 raise 不 kill** 失败的 main.py 子进程 → 失败进程泄漏堆积(实测堆到 51 个)。
- **本次 bug B(快速空转型)**:keeper spawn 与 lease 状态不对齐 → spawn-冲突-再 spawn 高频空转(本文 §3)。

## 4. ccbd-rust 需求规格(行为契约)

Rust 重写版必须满足以下行为,从根上杜绝上述两类 spawn 风暴:

### R1 — spawn 前必须查权威 lease 状态
keeper 在 spawn 任何 daemon 之前,**必须**先读取 ownership lease:
- lease 已被一个**健康**进程持有(心跳新鲜)→ **不 spawn**,进入"已就绪"稳定态,只做心跳监控。
- 仅当 lease 空缺、或持有者已判定死亡(心跳过期)→ 才允许 spawn。

### R2 — spawn 失败必须指数退避
连续 spawn 失败(含 ownership 冲突、启动超时)时**不得立即重试**:采用指数退避(如 0.5s → 1s → 2s → … 上限 30s),退避期间不 CPU 空转。

### R3 — spawn 死循环必须熔断 + 告警
滑动窗口内(如 60s)spawn 失败次数超过阈值(如 5 次)→ **熔断**:停止 spawn、进入降级态、输出 ERROR 级日志 + 可观测信号,**不再静默空转**。

### R4 — 启动超时的子进程必须被回收
spawn 的 daemon 若在就绪 deadline 内未通过健康检查 → **必须 kill 该子进程**(SIGTERM→SIGKILL),不得只报错不回收(根治历史 bug A 的进程泄漏)。

### R5 — ownership 冲突是正常信号,不是需要重试的失败
新 daemon 遇到"lease 已被健康进程持有"时,这**不应**被当作需要重试的错误:新进程应安静退出(或在 R1 下根本不被 spawn),keeper 应将其解读为"已有健康 daemon"的正向信号、转入稳定态,而非触发再次 spawn。

### R6 — 全程可观测
spawn / 退避 / 熔断 / 子进程回收 / 进入稳定态 —— 每个决策点都要有结构化日志(决策 + 原因),使运维能仅凭日志重建完整的 keeper 行为序列。

## 5. 验收标准

- **AC1**:已有健康 daemon 时触发 keeper reconcile,**不产生**任何新 spawn(日志显示 "lease healthy, skip spawn")。
- **AC2**:模拟 daemon 持续启动失败,keeper spawn 间隔呈指数退避,且 60s 内 spawn 次数 ≤ 熔断阈值。
- **AC3**:超过熔断阈值后 keeper 停止 spawn、输出 ERROR、进入降级态。
- **AC4**:模拟 daemon 启动超时,验证超时子进程被 kill(无泄漏残留)。
- **AC5**:24h 稳定运行,`ps` 中 daemon 进程数恒为 1(或 0),无空转 spawn,stderr 无 `OwnershipConflictError` 刷屏。

## 6. 影响与优先级

- **影响度**:High — 生产事故,把 7.7 GiB 机器内存压力顶到 81%,触发 oomd 杀进程 + 全部 claude 卡死。
- **证据度**:High — 物理日志 + 进程状态实证(见 §2)。
- **优先级**:must-fix — ccbd-rust 作为 ccbd 的根治重写,keeper 与 ownership lease 的状态对齐 + spawn 退避/熔断/回收 是其核心可靠性需求。
