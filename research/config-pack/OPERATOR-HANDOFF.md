# OPERATOR HANDOFF — 2026-07-08(/clear 后新 operator 读这个接上)

你是 **operator**(控制 master 的那层,不是 master、不是 worker)。**先读记忆锚点**:
- `memory/project_config_pack_real_target_fidelity_vs_scale.md` — 配置包真实目标(保真 vs 规模)。
- `memory/project_ah_codex_one_role_two_instances.md` — ★角色模型正确口径(codex 一个角色/a1a2 两实例只为并发)。
- `memory/project_ah_v1_public_release.md` — v1 场景层拆分方向。
- `memory/project_ah_relay_to_master_tmux_inject.md` — 给 master 派活的 tmux 直注姿势。
- `memory/feedback_antigravity_divergence_discipline.md` — 隔离 agy 发散纪律。

---

## 当前主任务(已从"SOP 沉淀成人读文档"掉头 —— 那个框歪了)

**用户真实需求(2026-07-07 澄清):造一套符合 ah 规范、可安装的「编程场景模板」= v1 的场景层。装上后 ah 就按现在这套 master+workers 跑编程。**

- 交付物 = 每个 slot 一份 `.ah/rules/<slot>.md`(master + a1..a4)+ `ah.toml` 拓扑 + `examples/scenarios/dev-programming/`(外部集成方复制的样板)+ README。
- **成功标尺 = 保真**:装到干净环境 → master+workers 行为跟现在一模一样。不是"零改动/零回归"。
- 范围:**先纯忠实复刻**(质量升级往 Fable5 抬另立一版);**装机对象 = 也给外部集成方(Studio 等)** → 要 v1-public-release 级。
- 机制真相(master 已从代码核实):组合 = `[kernel内嵌] + [bundle] + [.ah/rules/<slot>.md 或 default]` → provider 目标文件(`home_layout.rs:523-549`);**per-slot 差异化只能走 `.ah/rules/<slot>.md`**,bundle 只有 master/worker role 键做不到 → 交付物必须是 per-slot 文件不能做 bundle;override **替换** default(每个 slot 文件要自带仍需的通用内容 + 差异化增量);kernel 严禁复写(双重注入)。

### 角色模型(用户纠正,钉死)
- **codex(a1、a2)= 一个角色,两实例只为并发**;codex = 严谨编码,既实施(TDD/串行 cargo)也严审(grep/file:line/可 REJECT)。**`a1.md` == `a2.md`**。
- **a3 antigravity = 设计/领域分析**(不写码)。
- **a4 claude = 二审 + e2e**。
- 真正区别角色只有 3 种:codex / antigravity / claude。

### 用户已拍的放行决定
1. 实施顺序:先写文件 + examples/ + 单测 → 在**干净一次性项目**跑 e2e 验保真 → 确认一致后**再**把 ccbd-rust 本仓 `.ah/rules/` 填真切上去(别在活栈干活时改它自己规则)。
2. stale 债(仓库 `CLAUDE.md` 的 ccb→ah/旧角色、死掉的 `rules/*.md`):**本轮不搭车,另立小 PR**。
3. 安装形态 = copy-in 样板,不碰 bundle 代码;全局出厂 `defaults/{master,worker}.md` **不动**。

### 进度 / 下一步
- ~~master DESIGN 已落 `research/sop-scenario-template-design.md`(15KB),经角色纠正后正在改设计 + 进实施。~~
- **★主笔权移交(用户 2026-07-09 明确指派)**:「编程场景包的 SOP 配置全部由 operator 自己设计,文档全部由 operator 自己写」——这是 operator 不动手规则的明确例外。master 不再主笔本包,后续 master 只在被点名时做核对性任务。
- **2026-07-09 operator 第一轮改写已落盘**(pack/ 全套):修正 ROLES.md 设计权归属(原稿把 antigravity 定位"探索者,不宜担纲设计判断"、设计主笔给 review-codex,与用户已批准框架相反);新增 `.ah/rules/design.md`(设计者+自行实施护栏);impl.md 补严审/spec 转写/断言表门,复制 impl2.md;删 review.md/explore.md(职能并入 design/impl);master.md 补错峰排期/设计辩论/brief 纪律/派单验证;GUIDE.md 新增「三、设计管线」章(五步换手+辩论纪律+冻结稿权威)+实践 9/10 条(压缩吞消息、派单竞态且 job 落库≠送达)+纪律表两行(本机测试面收窄+CI 唯一门、断言表);ah.toml.example 换修正拓扑。
- 待办:examples/scenarios/dev-programming/ 样板 + 干净项目 e2e 保真验证 → 过目后开 PR;ccbd-rust 本仓 `.ah/rules/` 切换仍按用户放行决定 1 的顺序。

---

## 线A(理论侧,已交付一块,非阻塞)
- 目标(用户定):在已凿定的数学地基**之上找工程实现**(不是重推数学、不是零预设推翻)。
- 隔离 operator agy(独立 HOME + token 拷贝,`scratchpad/agy-operator-home`)已产出 `research/config-pack/agy-entropy-engineering-impl.md`。
- ★核心独立发现:KL 对参考分布 Q 检测"自信地错"是幻觉(critic 与 generator 共享盲点)→ 逻辑正确性只能靠真实执行(编译/沙箱/测试),judge 探针退守到架构/风格边界。工程落法:结构化决策槽取 logits 算 H;Likert logits 门驱动 ΔH 控制流;v0 = agy-commit-guard git hook。
- 数学地基锚点在 `memory/project_config_pack_real_target_fidelity_vs_scale.md` §主线锚点。**注意用户 ★★★★★ 纪律:发散前问题必须零预设、允许推翻问法。**

---

## live 栈信息(重启/清理别误杀)
- systemd unit `ah-9819d8d7587886a9.service`,socket `ahd-9819d8d7587886a9`,state `AH_STATE_DIR=/home/sevenx/.local/state/ah/29acbe42`,ahd 二进制 `target/release/ahd`。
- master pane `%0`(raw `claude`,provider 拓扑见 `ah.toml`:master=claude / a1a2=codex / a3=antigravity / a4=claude)。
- 整栈重启姿势:`AH_STATE_DIR=.../29acbe42 ah stop` → `ah start`(BindsTo 级联会连坐清 master+workers;Restart=on-failure 不自动 bounce;master 是 raw claude 非 ahd-tracked,重启后需手工 orient)。
- 给 master 派活:`ah tell` 在活栈失效 → Write 文件 → `tmux -L <socket> load-buffer` → `paste-buffer -p -t %0` → `send-keys -t %0 Enter`;capture-pane 有渲染延迟隔拍重抓。
- 机器上有一堆泄漏测试 socket(`ahd-*` 十几个)+ 泄漏沙箱,别误杀 live 栈。

## backlog(非阻塞)
- 沙箱 GC(445 个泄漏曾打满磁盘)。
- DevStateCleanupGuard → r1_shutdown_cleanup。
