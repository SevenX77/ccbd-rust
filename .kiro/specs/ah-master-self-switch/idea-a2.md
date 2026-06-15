# Step-4 宽版 master 自换 — a2 (gemini) 第一性原理思路 (1c)

> 2026-06-15 a2 (gemini) 作为设计咨询产出, Master PM 转录 (a2 边界=不改文件, 故 PM 落盘供 1d audit)。
> 这是**思路 (idea)**, 不是正式 design。下一步 a1+a3 audit 思路本身 (1d)。

## ① 核心思路: 从"旁观者"到"驻留者" (Self-Hosting Bootstrap)

**第一性原理**: 一个能托管自身的系统, 必须解决"第一推动力 (Bootstrap)"问题。你不能用 ah 起 ahd、再用 ah 起 master, 同时还让这个新起的 master 去接管当前正在说话的旧 master。

**大方向**: 采用 **"双活蓝绿部署 + 状态转移 (Blue-Green Cutover with State Handover)"** 机制。

**核心流转**:
1. **Bootstrap (蓝阶段)**: 旧 Master (当前跑在 ccb 上) 通过 `ah start` 启动 ahd (若没起), 并发起 `session.create` 建长期会话。
2. **驻留 (启动绿 Master)**: 旧 Master 发 `ah session.spawn_master_pane`, ahd 在内部拉起新的 `claude --continue /remote-control` 进程 (新 Master)。
3. **身份移交 (State Handover)**: 新 Master 起来后, 因带 `--continue`, 连接到同一 Provider 账户并同步出当前对话上下文。
4. **脑裂静默 (Split-Brain Avoidance)**: 此时系统有两个 master。旧 Master 确认新 Master 启动成功后, 主动"自我放逐 (Self-Deactivation)" (进入 sleep 死循环, 或退出当前 ccb 会话)。从此 ah 托管的新 Master 接过话筒。
5. **自愈闭环达成**: 进入此状态后新 Master 在 ahd 监控下。OOM → ahd 复活它 → 再 `--continue` → 真正实现保护。

## ② 各关键张力的判断与理由

**1. Bootstrap / Cutover 机制: 必须"自己起自己, 然后自己退"**
- 判断: 坚决反对外部 helper 脚本做硬切换。
- 理由: 外部脚本引入环境不一致 (env / 路径 / SSH 代理)。由旧 Master 亲自触发 `ah start` + spawn, 完美继承运行时鉴权上下文。K8s 里叫"自举 (Self-Hosted Control Plane)", 最干净。

**2. 递归归属: 扁平图谱, 逻辑挂载**
- 判断: ahd 内部不应在物理结构上区分"谁是谁的子进程"。Master 和 Worker 在 systemd scope 层应平级, 都在 ahd 之下。
- 理由: 若 Master 物理上挂载 Worker, Master OOM 会连带带走 Worker。只有平级挂载才能实现"Master 死而 Worker 不受影响"。
- 身份自洽: 靠 home_layout 角色隔离 (Master 有自己沙箱 + MASTER_RULES, 通过 socket 与 ahd 通信; socket 是全双工多路复用总线, 不关心调用者是不是自己人)。

**3. Master-Revive 适配与"续断点"语义**
- 判断: Master OOM 后 `--continue` 恢复的是 **Provider 对话上下文 (大模型记忆)**, 但**不恢复 Ah-Job 级在途状态** (发到一半还没等回来的 ah ask)。
- 理由: Master PM 是基于文本的交互式代理。它死了, 正在跑的 Worker 还在干活, 干完结果输出到 Worker 的 pane。新 Master 活过来后通过 `ah ps` / `ah logs` 查 Worker 进度, 这是完全合乎逻辑的 PM 工作流。试图在底座恢复"Master 发了一半的 CLI 命令"既不可能也不必要。

**4. 回退 / 双调度安全: 显式"接管令牌 (Cutover Token)"**
- 判断: 保留 ccb 随时作逃生舱, 但必须防脑裂双发。
- 理由: 蓝绿切换后旧 Master 若不小心醒来 (或有人在旧终端敲字), 会两个 PM 抢指挥。
- 机制: 旧 Master 完成 Handover 前, 向工作区写 `.ah_cutover_active` 锁文件。任何依赖 ccb 调度的流程 (或旧 Master 的 prompt) 必须先检查这个锁, 存在则主动退出报错 = 物理 fencing。

**5. 验收定义: 真实灾难演习 (Chaos Engineering)**
- 判断: 仅用 `ah ask` 跑几个任务不算宽版通过。
- 验收标准:
  1. 新 Master (在 ah pane 里) 能成功派出 >10s 的真实 worker 任务。
  2. 任务进行中, 从外部对该 Master 进程 `kill -9` (模拟 OOM)。
  3. 观察到 ahd 自动 `spawn_master_pane` 复活它。
  4. 新复活的 Master 能在终端用语言告诉你"我刚才好像重启了", 并能用 `ah ps` 查到正在跑的任务, 且该任务最终成功结束并返回结果给 Master。

## ③ 最大风险 / 不确定点 (a2 诚实标)

1. **TTY 流向截断黑洞 (Critical)**: 旧 Master 跑在 SSH 终端 (你能看输出 + 打字)。新 Master 跑在 ahd 托管的 tmux pane 里 = 后台进程。你如何"看到"新 Master 输出并跟它对话?
   - 应对: 切换完成后在前台终端 `ah attach master` 去 attach 新 tmux pane。但中间有几秒盲区 (Blind Spot), 此时交互输入可能丢失。需要极精确的 Handoff Prompt 步骤。
2. **Claude `--continue` 的重叠边界**: 旧 Master 还没退出, 新 Master 用 `--continue` 连到同一云端对话 ID。这两个端点会不会触发防并发控制 (如 429)? 需实测验证 Provider 在交接期的容忍度。
