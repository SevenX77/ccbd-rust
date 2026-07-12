# OPERATOR-HANDBOOK — operator 使用指南(从接管到跑通)

> 给**被任命为 operator 的 agent(或人)**的一本上手+日常驾驶手册。角色的"为什么"
> 与逐条实践在 `GUIDE.md` §四,任务推进保障机制在 `OPERATOR.md`(三层哨兵),
> 本手册回答"怎么从零把一套编队跑起来、跑到交付"。冲突时以 GUIDE / OPERATOR.md 为准。

## 0. 治理链(先把位置站对)

```
用户(出需求和目标)
 └─ operator = 用户的代理 CEO —— 监督和管理 PM 的工作:下目标、审产出、拍板阶段转换;
    │           git 权力面(push/PR/合并/发布)与凭据操作归 operator;
    │           不越过 master 直接调配团队(只读抽验除外)
    └─ master = 项目经理/小组长 —— 其他所有 agent 都在 master 管辖之下,由 master 调配
        └─ workers(实施/闸门/设计辩论/设计执笔…拓扑由项目 ah.toml 定)
```

- operator **只动脑不动手**:凡能写成 brief 派给 master 的活一律下派。亲手例外仅四类:
  ①git 权力面与凭据;②对外仓库/配置包维护与 SOP 主笔;③哨兵体系的搭建维护(控制回路
  本身,见 OPERATOR.md §六);④只读抽验取证。动手前自问:这活为什么不能派?
- 泳道/闸门类拓扑里,闸门的"泳道内终裁"是 **master 的裁决下放**,不是脱离管辖;
  同理,operator 给 master 下目标,不给 master 的团队直接下指令。

## 1. 接管四步(被任命时做一次)

1. **通读文档**:本手册 → `GUIDE.md`(§四逐条读)→ `OPERATOR.md` → `ROLES.md` →
   目标项目的 `.ah/README.md` + `.ah/VERIFY.md`。
2. **把身份块写进自己的指令文件**(Claude Code=`CLAUDE.md` 或记忆体系;Codex=`AGENTS.md`;
   Gemini 系=`GEMINI.md`——写进你的 harness 每次会话自动加载的地方),模板:

   ```markdown
   ## 我是 operator(用户代理 CEO)
   - 角色规范:<pack 路径>/GUIDE.md §四 + OPERATOR.md;本手册 <pack 路径>/OPERATOR-HANDBOOK.md。
     每次会话开始先 git pull pack 再重读,冲突以 pack 为准。
   - 治理链:用户 → 我(监督管理 PM,不越级调配)→ master(管辖调配全员)→ workers。
   - 我驱动的 ah 栈坐标(实测值,栈重建后必须更新):
     - worktree/项目根: <path>    - state dir: <~/.local/state/ah/<hash>>
     - tmux socket: <ahd-xxxx>    - master pane: <%0>
   - 铁律速记:只动脑不动手;每个等待必有闹钟(哨兵);job 状态双向不可信,先物理验证;
     派单验证到 pane 层;给 master 送话=文件注入 SOP;阶段收口停下向用户报告。
   ```
3. **实测栈坐标,别抄别猜**(坐标是事实不是配置,cutover/重建后全会变):

   ```bash
   ah ps && ah doctor                       # 栈活着吗,拓扑是什么
   ls ~/.local/state/ah/                    # 哪些 state dir,mtime 最新=活栈
   ls /tmp/tmux-$(id -u)/                   # 哪些 ahd-* socket
   systemctl --user list-units 'ah-*'       # daemon unit
   tmux -L <socket> list-panes -a -F "#{pane_id} #{pane_title}"   # pane 对号
   ```
4. **向用户报到**:一句话报清当前栈状态(活/死、在跑什么、队列里有什么),等用户给目标。
   不要上来自己找活干。以后每次会话:pull+重读 → 验坐标 → 接着 in-flight 干。

## 2. 从零拉一套编队(每个任务一次)

前提:目标项目已接入本包(项目根有 `ah.toml` + `.ah/rules/` + 填好的 `.ah/VERIFY.md`;
没接入就先按 `README.md`「一分钟上手」接入,拓扑参照根目录或 `dual-lane/` 模板)。

1. **同步与自检**:项目仓 `git pull`;机器级自检(时区/代理/凭据——这类**本机配方沉淀在
   项目 `.ah/README.md`**,不在本包;没有就边踩边回写,档案错了改档案)。
2. **一任务一工作区一栈**:在任务专用 worktree/目录里起栈,绝不在主仓根;并行任务各起各的。
3. **钉 state dir**:`ah` 各子命令的 state-dir 解析可能不一致(实测踩过)。驱动栈的每条
   命令统一显式 `AH_STATE_DIR`,钉到该项目的规范目录(`timeout 3 ah events --format json
   | head -1` 输出里的 `state_dir` 字段),任务间天然隔离。
4. **运行时注入不入库**:代理 env、master 的本机 cmd 变体等机器私有配置,改在 worktree
   的 ah.toml 里但**不 commit**;派单 brief 里写明"不改/不提交 ah.toml"。
5. **凭据(多 claude 席位必读)**:编队里 >1 个 claude 实例**绝不共享交互登录的轮转
   凭据链**——symlink 和复制都不行:claude 用「临时文件+改名」写回,第一次成功刷新
   就把 symlink 摘成私有文件,token 轮转随即作废源文件里的链,其余实例级联 401 登出,
   人类用户被迫一天多次重登(机制与文件系统取证:ah#18)。正道:`claude setup-token`
   生成长期令牌,经 `CLAUDE_CODE_OAUTH_TOKEN` env 注入编队(ah ≥1.3.2 已白名单直通);
   人类交互会话保留自己的轮转链,两不相扰。
6. **validate → 前台 start → 验到 pane**:

   ```bash
   ah config validate --config ./ah.toml
   ah start --wait          # 必须前台跑,别 & 后台——失败要当场看见
   ah ps                    # 全员 IDLE?
   tmux -L <socket> list-panes -a -F "#{session_name} #{pane_id} dead=#{pane_dead}"
   tmux -L <socket> capture-pane -p -t %0 | tail -5    # master 登录态/statusline 正常?
   ```
   worker 没到 IDLE / pane 秒死:先 capture 尸体 pane + 查 daemon 日志定死因,修根因再重启;
   常见死因是凭据链断与出网 env 缺(答案通常在项目 `.ah/README.md` 的本机配方里)。

## 3. 给 master 派单(每单)

1. **写 brief 文件**(要素:目标 → 阶段划分 → 分工建议 → 安全铁律 → 通用约束 → 完成信号)。
   必含:验证命令引用 `VERIFY.md`;不 push/不改运行时注入文件;阻塞与提问的**落盘信道约定**
   (worker→闸门用 `.lane-question`,master→operator 用 `.operator-question`,完成用
   `.operator-report`)——落盘的才算正式交付物,盯 pane 不可靠。
   若任务本身要做编排器/生命周期实验:**钉死"一切实验用一次性隔离环境,绝不碰编队自己的栈"**。
2. **文件注入 SOP**(唯一可靠姿势;绝不 printf/echo 双引号内联——反引号会被 shell 当命令
   替换真执行,出过险些 OOM 的事故):

   ```bash
   tmux -L <socket> load-buffer <brief文件>
   tmux -L <socket> paste-buffer -p -t %0
   sleep 1 && tmux -L <socket> send-keys -t %0 Enter
   sleep 8 && tmux -L <socket> capture-pane -p -t %0 | tail   # 隔拍验证真提交、真开始处理
   ```
   已知坑:Enter 被吞(再按,隔拍验证);capture 有渲染延迟(隔几秒重抓);master 上下文
   接近压缩时会吞消息(发完必须验证它动起来了)。
3. **派完 ≠ 结束**:立刻进入 §4 的哨兵态。

## 4. 监控与推进(哨兵体系,详见 OPERATOR.md)

- **每个等待都必须有闹钟**——这是第一性原则,靠自律必停摆。master 层每单挂 pend 哨兵
  (它的规则里有);operator 层常驻两件:90s 全局停摆体检 + DB 状态翻转监听,任一异常
  物理唤醒自己。你的 harness 怎么挂后台任务就用什么(后台进程/定时器),要点是**退出即唤醒**。
- **监控锚产物轨,不锚 job 状态**:可信证据只有 pane 实际内容、git 产物、约定落盘文件;
  job 的 COMPLETED/FAILED 双向都会撒谎。假 COMPLETED=状态作废、**绝不重派**(agent
  上下文完好还在干),盯到真产出。
- **全栈安静数分钟 = 亲自 capture master pane**:job 监控对"master 在等人"全盲,
  实测一单 87 分钟里 52 分钟是 master 在等拍板。
- 哨兵响了先取物理真相(capture-pane + git log/status),再走 OPERATOR.md §三的分诊树。
- 资源守卫:共享机器上编译/全量测试串行排队;brief 里禁止 agent 后台跑测试。

## 5. 收口与权力面

- **agent 说"完成"必须物理验证**:commit 在不在、落盘在不在、测试输出真不真,自己跑命令验;
  审计者的"决定性证据"也要抽验——审计也是被审对象。
- **git 权力面**:worker 只 commit 到任务分支;operator 审过 + CI 绿才 push/开 PR/合并。
  有分支保护的仓库可用 auto-merge;**无分支保护的仓库绝不 `--auto`**(实测=秒合不等 CI)。
- **请示线**:cutover/停正在干活的栈/发版/产品方向选择 → 请示用户;实施细节(commit 落点、
  任务排序、blocker 修复)→ operator 自决,不上抛。阶段收口停下向用户报告,这不算擅自停。
- **停栈善后**:`ah stop` 后检查残留(systemd unit enabled 会在重启后复活 daemon、
  tmux socket、沙箱尸体),项目 README 的配方怎么写就怎么清。
- **当日结转**:SOP 级教训进本包(改文档+CHANGELOG 记来源);项目/机器级事实进项目
  `.ah/README.md` 或你自己的记忆体系。发现实际运行偏离规范文档,按结构病立即告警修正。

## 6. 文档地图

| 文档 | 管什么 |
|---|---|
| `README.md` | 包是什么、两套拓扑怎么选、一分钟上手 |
| `GUIDE.md` | 三层拓扑、SOP 闭环、设计管线、**§四 operator↔master 逐条实践**、纪律清单 |
| `ROLES.md` | 角色原型→provider 能力;设计权/执笔权归属 |
| `OPERATOR.md` | 任务推进保障:三层哨兵、停摆分诊树、高危操作连带、闭环证据纪律 |
| 本手册 | operator 从接管到跑通的操作路径 |
| `VERIFY.md` | 项目验证档案模板(fill-once) |
| `ah.toml.example` / `dual-lane/` | 基础版 / 双泳道版拓扑模板 |
