# telemetry 立项补充(operator 消化用户三点意见,2026-07-09)

## 1. context 采集:优先用 provider 现成出口,推导法降为兜底
用户点破:别急着自己算累计 token ÷ 窗口表,provider 有现成出口:
- **claude statusline 注入法(最优雅)**:ah 掌管沙箱 settings.json,可以给每个 claude agent 配一个 statusline command,claude 每次刷新会把 context JSON(含 .context_window.used_percentage,见 research/findings/per-day/home-sevenx-2026-04-22.md:30)喂给该命令——命令就写"tee 到 ah 约定路径",ah 定期读文件即得实时 context %。零解析 transcript、零推导。
- **codex**:/status 命令有输出;能否非交互获取待查(可能要 pane 注入+捕获,或看 rollout session-header)。
- **hook 自报法(兜底)**:Stop hook 的 payload 里带 transcript path,hook 脚本自己算/摘 usage 也可,但不如 statusline 直接。
- 推导法(累计 usage ÷ model 窗口表)仅当上述都不可用时用。

## 2. antigravity 能力问它自己
standalone agy --print 自述查询已发(答案落盘后并入本文件);它有自己的使用指南 skill,后续这类"provider 能不能 X"的问题先问 provider 本人再翻代码。

## 3. 审计已确认的落地要点(见 research/telemetry-sources-audit.md)
- 按 job 归账走派单时字节游标窗口,不需要时间戳管道。
- claude 的 usage/model 在 ah 已解析的行上,白捡;codex 补 token_count 分支;antigravity 待其自述。
- 配额/限流唯一路子是 pane 指纹,且要能区分"限流"vs"卡死"(现在分不清)。

## 4. antigravity 自述结果(agy --print,2026-07-09,字段号需实证复核)
- **token 用量存在但是二进制**:不在 transcript/cli.log,而在 `~/.gemini/antigravity-cli/conversations/<conv-id>.db` SQLite 的 `gen_metadata` 表 data 字段(protobuf blob);varint 路径 1.4.1=本轮输入、1.4.2=累计/上下文总量、1.4.3=本轮输出(或 1.17.2.x 镜像)。采集可行但要 protobuf 解码,比 claude/codex 的 JSON tail 重。**注意:字段号是 agy 自己反解的,spec 阶段必须拿真库复核。**
- **1.4.2(累计 context 用量)顺带解决 antigravity 的 context 采集**——不用推导。
- statusline/status/context 接口:无(settings.json 仅 colorScheme/model/trustedWorkspaces)。
- transcript.jsonl 无 model/token/quota 结构化字段(与 a4 审计互证);model 名只在设置变更事件的 content 文本里。
- 官方 skill 文档无可观测性章节(产品上不存在这些接口)。

## 限流指纹实样(2026-07-09 02:5x,活栈捕获)
codex 弹出交互菜单(触发 PROMPT_PENDING,真 prompt 非幽灵):
- 标题行:`Approaching rate limits`
- 副行:`Switch to gpt-5.4-mini for lower credit usage?`
- 选项:1 Switch / 2 Keep current model / 3 Keep current model (never show again)
处置:选 3。prompt KB 应新增 case(provider=codex,trigger_state 任意):match "Approaching rate limits" → 自动答 3;这同时是配额遥测的信号源(出现即=接近限额)。
