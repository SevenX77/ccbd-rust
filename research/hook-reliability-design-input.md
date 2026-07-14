# hook 可靠性三方案(用户提出,operator 判定可行,2026-07-09)
下一轮 orchestration-reliability 立项输入;与幽灵文本误判、限流/卡死分辨、MAX_LOG_MONITOR_WAIT 同轮。

## 1. 完成语义:显式声明三层
- 首选:显式完成 tool/命令 `ah job done <job_id> --summary`,worker 规则写死必调;完成信号=agent 主动结构化 RPC(同 R3 语义 ACK 思想)。
- 强制层:claude Stop hook 支持 block+reason 回塞——Stop 时检查当前 job 是否已声明完成,未声明则拦截结束并提示"继续或显式声明"(顺带堵 end_turn+后台任务在跑 的过早判完洞)。
- 兜底:现有 log/hook/pane 检测降级为超时兜底,不删。
- 可选:关键 job 完成先落待确认态,master double-check 后终结(全量开会拖慢流水)。

## 2. 投递可靠:outbox 模式(比 dead-letter 更强)
- hook 先追加写本地事件流水账(沙箱内 jsonl,带事件 id),再同步投递 ahd 并等 ACK;超时/失败时流水账即错误本。
- ahd 启动/重连先重放各 agent 流水账,按事件 id 去重(at-least-once + 幂等)。
- 覆盖:ahd 不在、socket 断、hook 进程中途被杀,全部不丢。

## 3. hook 健康自检:三档
- 启动静态校验:settings.json 里 ah 物化的 hook 条目存在性 diff。
- 启动合成触发:ahd 直接执行 hook 脚本本体验证脚本+socket 链路(零 token,不惊动 agent)。
- 深度端到端(ah doctor / 沙箱重建后):真派最小任务验证 agent→hook→ahd 全链路。
- 漂移处置:仅在 agent IDLE 时重物化配置(忙时重置=对干活 agent 发键的老坑)。
