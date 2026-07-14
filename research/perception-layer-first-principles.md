# ah 感知层第一性重构基准(north star)
2026-07-09,operator 起草。触发:用户指出 pane-diff 是"没想到 hook 的不得已",要求从第一性原理对标行业最佳实践重构,不迁就现状。本文是下一轮 orchestration-reliability 的设计北极星:先定理想形态,再列差距,实施向理想收敛而非继续打补丁。

## 一、第一性:hypervisor 必须确定性知道的五件事
F1 agent 进程活着/死了
F2 回合边界(在干活/等输入)
F3 任务结果(job 完成/失败 + 产物)——注意 F3≠F2,end_turn 不是任务完成
F4 agent 在等交互输入(信任对话框等)
F5 资源水位(token/context/配额)

## 二、信号分级(行业最佳实践的等级制,不是投票制)
- **T0 OS 真相**(pidfd/cgroup scope/exit code):不可伪造,负责 F1。
- **T1 源头结构化事务事件**(hook→outbox 记账→投递→ACK;显式完成声明 tool):负责 F2/F3/F4 的控制路径。要求:at-least-once+事件id幂等+本地journal,投递失败≠事件丢失。
- **T2 持久产物重放**(transcript log-tail+游标):负责恢复/审计/ahd重启后重推导,是耐久层不是主控制路径。
- **T3 UI 观察**(pane 文本):**只**负责 F4 里没有 API 的交互对话框驱动 + 人类调试。**永远不做生命周期推断**。
原则:高层级信号缺失时系统必须"响亮地降级"(告警+错误本),绝不无声滑落到低层级信号顶班。

## 三、现状 vs 基准的差距表
| 差距 | 现状 | 基准要求 | 定性 |
|---|---|---|---|
| G1 hook 投递 fire-and-forget | notify 无 ACK 无记账,ahd 不在=事件蒸发 | outbox:先journal后投递等ACK,ahd回来先读错误本重放 | T1 不达标,结构缺陷 |
| G2 无显式任务完成协议 | 从 end_turn/task_complete 回合信号**推断**任务完成 | 派单带job id,完成=worker 主动 `ah job done <id>` 声明;claude Stop hook block+reason 强制未声明不许结束;检测降级为"停了却没声明"的看门狗告警 | F3 被 F2 顶替,过早判完是结构病不是 bug |
| G3 pane 参与生命周期判定 | prompt-scanner 误判幽灵文本→锁 PROMPT_PENDING→卡派单;UI 兜底判完成 | pane 只驱动已知交互对话框(trust/update);生命周期状态一律不从 pane diff 推断 | T3 越权,三次幽灵事故是结构病 |
| G4 控制路径无自检 | hook 配置蒸发无告警;orphan reconcile 写了没接线没人发现;探针曾经不重装 | 启动自检套件:hook 配置 diff+合成触发+接线完整性断言;深度端到端进 ah doctor | "零件好的忘了装"病族的根治 |
| G5 可观测性缺层 | token/context/配额零采集 | telemetry 骑同一事件脊柱(已有审计+addendum) | 已立项待实施 |
| (达标) T0 | pidfd+启动重装+周期巡检+scope 连坐,#110 ownership 校验 | — | 事故硬化后基本达标 |
| (半达标) ahd→消费者 | PR2b 已做 job_transitions 原子发射+游标 | agent→ahd 段(G1)补齐后才算全链路事务 | 脊柱建了一半 |

## 四、重构方向(按依赖排序)
R1 事件脊柱补全:hook 侧 outbox+ACK+重放(G1)——其余一切的地基
R2 显式 job 协议:done 声明 tool + Stop 强制层 + 检测转看门狗(G2)
R3 pane 降权:scanner 只留交互对话框职责,生命周期推断全部拆除(G3,依赖 R1/R2 落地后才能安全拆)
R4 控制路径自检:启动三档检查 + 接线完整性断言(G4)
R5 telemetry 上脊柱(G5,已有完整输入)
注:R3 必须在 R1/R2 稳定后执行,拆早了没有替代信号——这是"不迁就"和"不冒进"的平衡点。

## 五、验收定义("好"的标准,不是能跑)
- 任一时刻 kill -9 ahd 再拉起,事件流无洞(outbox 重放可证)。
- agent 挂着后台任务 end_turn:job 不判完,看门狗告警"停了未声明"。
- 幽灵文本/banner 任意出现:零生命周期影响。
- 手工删掉某 agent 的 hook 配置:下次启动自检报警并自动修复,期间有响亮降级日志。
- 以上四条全部有自动化测试钉死。
