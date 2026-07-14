# 病例+结构缺陷:agent 工作区无环境级指派——"叮嘱式 worktree"必然失守(2026-07-11,观察日志 #49③)

## 现象(实证,两轮独立失守)

Gateway A/B 实验要求两臂各在自己的 worktree 工作(`ccbd-rust-wt-gw-a/-b`),指派方式=席位规则 md 里钉死(文件/分支/铁律措辞俱全)+ brief §0 工作区条款:

- **第一轮**(旧规则,无钉死):两 codex 在 cwd(主树)混写 46 文件——可归因于规则没送达(#47)。
- **第二轮**(respawn 后,新规则已物化进 `.codex/AGENTS.md`,`wt-gw-a` 标记亲验存在):codex **仍**在主树实施,并把实现 **commit 到本地 main**(f174687 + amend 0ed41d1)。规则送达了,照样失守。

## 病理(第一性)

**工作区目前是"叮嘱"不是"环境"。** ah 的事实现状(代码亲验):

- config 层无任何按席位 cwd/workdir 配置(grep `cwd|workdir|working_dir` 于 src/config* 零命中);
- 所有席位 tmux spawn 一律 `-c <项目根>`(src/tmux/session.rs / scope.rs),即**每个 agent 的世界从主树开始**;
- 要求 agent "cd 到别处并一直呆在那"= 与它的每一个默认(文件探索、git 命令、构建路径)对抗。对 LLM,**可被忽略的指令,规模够大就必然被忽略**——这与"pane 生命周期推断整体删除"(unknown 文本永不造状态)和"撤停下==完成"同一条设计公理:**关键属性必须由机制/环境承载,不能由模型自觉承载。**

## 修向(三层,按治本程度排序)

1. **环境层(治本,ah 功能)**:工作区成为 slot 的环境属性——
   - `[agents.<id>] workdir = "<path>"`(ah.toml 静态指派),spawn/respawn 的 tmux `-c` 用它;
   - 或 `ah ask --workdir <path>`(按单指派,realign 不受影响);
   - 验收:spawn 后 `pane_current_path == workdir`;respawn 保持;workdir 不存在时 fail-closed 拒绝 spawn(不回退主树)。
2. **物理闸(今天可用,已装)**:仓库级 pre-commit hook——本地 `main` 分支禁止直接 commit(worktree 共享 `$GIT_COMMON_DIR/hooks`,一个 hook 罩全部 worktree+主树;operator 以 `AH_ALLOW_MAIN_COMMIT=1` 覆盖)。挡住最坏结局(污染 main 历史),挡不住主树工作区脏写(由 operator 监视器兜:主树 src/tests 写入告警)。
3. **指令层(免费但最弱)**:brief 开工自验条款(cd + `git rev-parse --abbrev-ref HEAD` 核对分支)。只作补强,不作依赖。

## 关联

- 观察日志 #49③、#47;A/B 实验协议 `research/ab-experiment-gateway-2026-07-11.md` §4
- 同公理病例:`feedback_delete_pane_lifecycle_inference`、完成检测根子(撤"停下==完成")
