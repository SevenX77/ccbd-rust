# home-sevenx 2026-04-25 session 提炼

### 1. CCB / Claude / Gemini / Codex 的 bug 或失败行为

- [09:43] 首次 master Claude **Node.js 进程自崩**：上一会话 16:40 最后一个 tool_use 没有 tool_result，Claude CLI Node.js 进程自己 panic 了，不是 OS kill / SSH 断 / mosh 睡眠。Anthropic upstream bug，之前用户记忆里"自曝方案"实际从未实现。
- [21:55] **Codex (a1) `ccb ask --wait` 长任务被 Bash tool 10min 超时强切**：job 本身没停，但主控 wait 句柄断了，导致主控不知道 codex 何时回。
- [22:51] Monitor 1h 自动超时，pend a1 显示 job_id 已被替换（旧 `job_c49bf1e39139` 被新 `job_8f6d6baee32c` 顶掉），CCB **mailbox 出现 job_id 替换语义不明确**。
- [00:11] 用户暴露 CCB 设计 bug：**两个完全不同 git repo 的 Claude 主控强制共用同一个 ccbd**。根因是 fork 自家 PR #190 + `~/.claude/shell/ccb.sh:66` 写死 `export CCB_PROJECT_DIR="$HOME"`，把 CCB 原生 cwd-walk discovery 砍平。
- [01:44] 用户跑 `! sudo sed ...` 失败：`sudo: a terminal is required to read the password`。Claude Code 的 `!` 命令通道**无法走 askpass / -S 交互**，sudo 直接 fail。
- [09:31] **Gemini agent (a2) 卡死返空 6 小时**：pane 抓帧显示状态栏 "✖ 18 errors"。根因：`~/.bashrc` 用的是 `GOOGLE_GEMINI_BASE_URL`，但 CCB launcher env 白名单（`lib/provider_profiles/materializer.py:25`）只放行 `GOOGLE_API_BASE`，导致 Gemini agent 拿中转 key 打 Google 官方 endpoint，401 堆积撑爆错误队列，TUI 不崩但拒接新请求。
- [10:37] **a3 (Claude agent) 未登录返 `Not logged in · Please run /login`**。根因：upstream commit `a77a861` "Simplify ask skills" 把 `~/.claude/.credentials.json` 的 symlink 逻辑删了，OAuth 用户的 token 没传进 isolated home。
- [10:55] **Gemini CLI 0.39.1 + `.gitignore` 互锁 bug**：CCB 把 prompt 写到 `<work_dir>/.ccb-requests/job_xxx.md`，CCB 安装时给项目 `.gitignore` 自动加 `.ccb-requests/`，Gemini CLI 默认 `respectGitIgnore=true` 导致 ReadFile 报 "ignored by configured ignore patterns"，4 次 retry 后弹 "A potential loop was detected" 交互 dialog。Issue google-gemini/gemini-cli #5259 / #10071 / #16980 跨半年未修。`@path` 语法在 `pathReader.ts:75-78` 被硬编码 force-respect，settings.json `respectGitIgnore: false` 都绕不过。
- [11:03] Claude 让 Gemini 审计划自己也踩坑：`claude_code_bridge/.gitignore:19` 也有 `.ccb-requests/`，Gemini 接到自己审计划的 prompt 立刻又触发 loop dialog，活活演示 bug。
- [10:39] 第三方 Codex API key（neonode `sk-0aaaab7...`）配置同时存在 5 处（`~/.codex/config.toml`、`~/.ccb/agents/a1/...`、`~/.local/share/codex-dual/.ccb/agents/agent{1,2,4}/...`），下次 `ccbs codex` 起新 agent 会从 codex-dual 模板复活。
- [19:12] 用户 `codex login` 在 VPS 失败 `can't be reach`：OAuth 回调假设浏览器跟 codex 同机，VPS 上 callback server 在 vultr 的 localhost:1455，浏览器在 Mac 上访问 Mac 自己的 1455 自然 ERR_CONNECTION_REFUSED。Codex CLI 的 OAuth 流程不感知 headless / 远程场景。

### 2. 用户纠正 / 抱怨 / 吐槽 Claude 的内容（带原话）

- [00:11] 用户对 Claude 之前 PR #190 + `$HOME` 硬编码方案：原话 "**这是谁设计的sb方案？**"
- [00:15] 用户进一步骂：原话 "**你的视野太窄了而且非常短视，不适合做设计，只能沿着设计好的路径推进项目。让Gemini全局考虑，重新设计**"
- [00:17] Claude 回 "我打了个补丁不是设计"，用户立刻反击：原话 "**还嘴犟，补丁不是设计的吗？怎么打不用思考吗？**" Claude 收到后承认嘴犟，承担三条具体设计失误。
- [00:19] 用户警惕 Claude 又要乱开 PR：原话 "**先别提pr了，万一提了个刚才那样的东西，我脸都丢不起**"，紧接着澄清 "**我的意思是不要upstream**"。
- [20:38] 用户校正 Claude 用错时区：原话 "**你管北京时间干嘛？我又不在北京，我在la，继续干活儿，别停下**"。
- [10:34] 用户指出 Claude 在交接信里把"已修复"写成"建议下一个 session 做"的歧义：原话 "**：~/.bashrc 加 export GOOGLE_API_BASE="https://chatapi.onechats.ai" 这个刚才不是加过了吗?**" Claude 承认歧义。
- [22:46] 用户对 Claude monitor script 之前没监控到任何东西的吐槽（context summary 里）：原话 "**主控说挂起一个monitor监控codex, 结果什么都没监控到, 这个监控是干什么吃的?**"
- [23:51] 用户连续 4 轮逼问 CCB 项目隔离设计："**怎么会两个主控共用一个codex？**" → "**ccb不是绑定主控的吗？**" → "**问题也不是一个项目啊**" → "**我的意思另一个Claude跑的是另一个项目！**"。Claude 前 3 轮都没真正 grasp 用户痛点。

### 3. 用户强意图（带原话）

- [00:23] "**开始**"（拍板按 Gemini 重设计方案干 Step 1 撤毒药）
- [08:13] "**开始**"（compact 后继续 Q3 Stage 1b）
- [13:983] "**所有任务,让Gemini审核一下设计,codex审核一下执行计划, 全部完成,不要停下,有判断不了的问题问Gemini, 和Gemini有分歧的相互辩论直到收敛**"——明确要 Claude 不停、自己跟 Gemini 辩论收敛、不要老回头问。
- [09:31] "**Gemini什么情况?把除了主控Claude之外的所有ccb和测试进程都清理一下**"——清场指令。
- [20:575] "**这个之前改过,是不是会被更新掉?保险起见,在bashrc中根据白名单的参数再添加一个google api base, model没关系**"——明确指挥用户侧 belt-and-suspenders 修法。
- [21:512] "**直接做c,验证其可行**"——拍板做 .credentials.json symlink 修复方案 C。
- [22:973] "**不用,这个api用不了了,现在我要用官方的订阅账号,帮我清理掉所有第三方的登录信息,api , url等等**"——切官方订阅账号，清掉所有 neonode 第三方残留。
- [00:19] "**先别提pr了**" / "**我的意思是不要upstream**"——所有 CCB 隔离重设计改动只准留 fork 本地，禁止上 upstream。

### 4. 对话中暴露的设计缺陷

- **CCB project anchor 一刀切毒药**（Claude 自己设计的 PR #190）：env var override 强行砍掉 cwd-walk 这层，把"workspace 物理位置"和"task session 上下文边界"两个不同概念混为一谈。Gemini 第一性原理重设计：CCB 隔离粒度应等于 LLM 上下文边界（Task Session），不是文件系统位置；推荐 1 Master = 1 ccbd = N agents 的 ephemeral sandbox。
- **CCB launcher env 白名单不完整**：`materializer.py:25` 的 gemini 白名单只放行 4 个 var，不包含 `GOOGLE_GEMINI_BASE_URL` / `GEMINI_MODEL`。用户配过这两个但根本传不进 agent。修法只能用户侧 bashrc 加 `GOOGLE_API_BASE` 镜像，或改 fork（用户禁止上 upstream）。
- **CCB 的 `.ccb-requests/` 投递目录跟自家 `.gitignore` 模板互锁**：`scripts/build_linux_release.py:22` 在新项目 `.gitignore` 自动加 `.ccb-requests/`，又用 `@<work_dir>/.ccb-requests/...` 让 Gemini 读，Gemini 默认 respectGitIgnore=true → 自己屏蔽自己。投递机制设计跟项目自动化的副作用直接冲突。
- **Claude provider 在 fork 上的 OAuth credentials 处理被 simplify 删除**（upstream commit `a77a861`）：copy-only 不带 `.credentials.json`，OAuth 用户全部断登录。简化时没区分 API-key 与 OAuth 两种登录场景。
- **Monitor 脚本只 poll status 不检测 stuck**（用户原话指出）：之前主控挂了 monitor 监控 codex 但没监控到任何东西，因为脚本只覆盖 happy path、没检测卡住场景。Multi-signal detection + deadline 缺失。
- **CCB ccbd singleton + serial-per-agent queue 没设计成 per-master**：两个 master 同 cwd 自动共享 ccbd + agent，Codex 看到的对话历史跨主控混合（context 跨任务污染）。CCB 默认配置下"per-repo 隔离"对 Codex 完全无效。
- **claude-ccb-orchestrator 的 `start-task-scope` 不 auto-spin agents**：只创建 systemd unit + project dir，导致 Claude 想用独立 scope 但 agent 不自动起来，只好 fallback 到默认 ccbd——架构 escape hatch 没接通。
- **Codex CLI OAuth login 在 headless / 远程不可用**：localhost:1455 callback 假设浏览器同机，VPS / SSH 场景下需要用户手动切 Device Code 流程，CLI 不感知不提示。

### 5. 决策转折点

- [00:15→00:17] 用户两轮怒骂"sb方案 / 视野窄 / 还嘴犟"逼 Claude 把 CCB project 隔离从"打小补丁"翻到"让 Gemini 第一性原理重设计"。Gemini 给出 1 Master = 1 ccbd = N agents 的 ephemeral 模型（最高优先级 explicit env var anchor，次 cwd-walk，最低 $HOME），变成本日最大架构决策。
- [00:23] 用户拍板 "开始"，按 Gemini 重设计三步路径：撤 `~/.claude/shell/ccb.sh:66` 的 `$HOME` 硬编码 → 改 orchestrator 注入 `CCB_PROJECT_DIR` → fork 内 `lib/project/discovery.py` 不动（已支持）。
- [00:19→00:22] 用户两次澄清"不要 upstream"——Q3 / CCB redesign 之后所有改动留 fork 本地，新建 `~/.claude/projects/-home-sevenx/memory/project_ccb_session_anchor_redesign.md` 记录。
- [01:44→01:45] 用户在外部终端 sudo 撤 `/usr/local/bin/claude-sandbox` 里的 `CCB_PROJECT_DIR=$HOME` 后回报"跑完了"——CCB redesign Step 1 落地完成。
- [13:983] 用户全权授权 "**所有任务,让Gemini审核一下设计,codex审核一下执行计划, 全部完成,不要停下**"——本日下半场推动 Q3 Stage 1b/c 全闭环 + 156 单测 + 6 集成测试 + 7 commits。
- [10:43] 用户拍板 ".credentials.json symlink 修复方案 C"（symlink 而非 copy，OAuth 刷新写回共享）。
- [11:00] Gemini 自己审 plan 时被自家 .gitignore 互锁触发 loop dialog，活体演示 CCB-Gemini bug，加速决策"用 stdin paste 替代 @path"成为持久修方案。
- [22:973] 用户决定弃用 neonode 第三方 API key，切 ChatGPT 官方订阅 OAuth 账号——触发对 5 处第三方 key 残留的全面清理（包括 codex-dual 模板的 agent1/2/4，否则下次起新 agent 会复活老 key）。
- [19:12] 用户 codex login 失败 "can't be reach" → Claude 诊断 OAuth callback 跟 headless 不兼容，引导用户走 Device Code 流程。

---

**核心主题**：用户连续逼 Claude 把"打补丁式工程"提升到"第一性原理设计"——CCB 项目隔离、env 白名单、credentials 继承、第三方账号清理一连串都从 Claude 自己埋的设计债（PR #190 / simplify commit / `.gitignore` 模板）里挖出来，Gemini 被反复请来做架构裁判，所有修复禁止上 upstream，全部留 fork 本地。
