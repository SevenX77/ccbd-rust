# agent-harness 编程 SOP 调研

## §A 概览

agent-harness 的主流程是“每个任务从 `origin/main` 切一个独立 worktree + 分支，在该 worktree 内开发和验证，脚本推 PR 并打开 squash auto-merge，CI 绿后自动合入 main，再显式清理自己的 worktree”。这不是单纯文档约定：根规则把它定义成唯一管线，`scripts/wt-new.sh`、`scripts/wt-dev.sh`、`scripts/wt-ship.sh`、`scripts/wt-clean.sh` 分别物化开工、预览、发 PR、清理。

与 ah 当前“共享单一工作树、串行 commit、较多流程靠 PM/worker 脑内纪律”相比，agent-harness 把最容易漂移的几步做成了可执行工具：

- 开工基线固定：根规则要求永远从 `main` / `origin/main` 开新工作，且警告不要基于旧分支或旧 worktree；脚本实际执行 `git fetch origin --prune` 后 `git worktree add -b "$branch" "$dir" origin/main`。证据：`/home/sevenx/coding/agent-harness/AGENTS.md:9-21`，`/home/sevenx/coding/agent-harness/scripts/wt-new.sh:25-33`。
- 并行隔离固定：worktree 放 `.worktrees/`，主仓根保留 `main`，一任务一 worktree；`.worktrees/` 被 gitignore。证据：`/home/sevenx/coding/agent-harness/AGENTS.md:19-21`，`/home/sevenx/coding/agent-harness/.gitignore:1-2`。
- 发版/合并固定：`main` PR-only、CI 过后 squash auto-merge；`wt-ship.sh` 会拒绝 main/HEAD、拒绝未提交改动、push、开 PR、启用 `gh pr merge --auto --squash`。证据：`/home/sevenx/coding/agent-harness/AGENTS.md:16-18`，`/home/sevenx/coding/agent-harness/AGENTS.md:42-48`，`/home/sevenx/coding/agent-harness/scripts/wt-ship.sh:12-33`。
- CI 门禁固定：GitHub Actions 在 PR 和 main push 上跑质量门、Python 多版本测试、前端门禁；根规则列出本地 push 前应跑的同一组门禁。证据：`/home/sevenx/coding/agent-harness/.github/workflows/ci.yml:1-6`，`/home/sevenx/coding/agent-harness/.github/workflows/ci.yml:17-158`，`/home/sevenx/coding/agent-harness/AGENTS.md:105-123`。

边界：`gh pr list` 无法访问，当前环境提示需要 `gh auth login` / `GH_TOKEN`，所以 PR 历史只用本地 `git log --oneline -40` 观察到的 squash commit 形态作证。最近提交普遍是 `feat(...)` / `fix(...)` / `docs(...)` / `chore(...)` 并带 `(#NNN)`，例如 `feat(studio): ... (#369)`、`fix(studio): ... (#366)`、`chore(scripts): ... (#334)`。

## §B worktree 详解

生命周期是“创建 -> 开发预览 -> 发 PR -> CI 合并 -> 清理”。

1. 创建：`scripts/wt-new.sh <type>/<short-desc>` 要求传入分支名，解析 repo root 时使用 shared git dir，所以从任意 worktree 调用也能回到主仓根；它只 `git worktree prune` 管理元数据，不自动清理别人的 worktree；随后从 `origin/main` 创建 `.worktrees/<type>-<desc>/`。证据：`/home/sevenx/coding/agent-harness/scripts/wt-new.sh:12-23`，`/home/sevenx/coding/agent-harness/scripts/wt-new.sh:25-33`。

2. 创建后预热依赖：`wt-new.sh` 后台跑前端 `npm ci` 并写 `.wt-install-done` 标记，也可后台跑 `uv sync --all-packages --all-extras --group dev`，日志和 pid 放在 `.worktrees/.<wt-name>.*`。这把“新树先装依赖”的等待从人工步骤变成后台准备。证据：`/home/sevenx/coding/agent-harness/scripts/wt-new.sh:35-60`。

3. 预览：`scripts/wt-dev.sh` 在当前 worktree 启动自己的 Vite 端口，默认代理到主仓根 sidecar；如果任务改 backend/engine/gateway，`--backend` 会从当前 worktree 的 Python 代码起私有 sidecar，并生成独立 token。证据：`/home/sevenx/coding/agent-harness/scripts/wt-dev.sh:1-21`，`/home/sevenx/coding/agent-harness/scripts/wt-dev.sh:80-107`，`/home/sevenx/coding/agent-harness/scripts/wt-dev.sh:109-124`。

4. 验证约束：文档明确“验证 worktree 改动”只能走 per-task Vite，不从 worktree 再起第二套 Tauri；前端改动代理主 sidecar，后端类改动必须用 `--backend` 验证自己的树，不能拿 main 的后端充数。证据：`/home/sevenx/coding/agent-harness/docs/development/RUN_AND_SCREENSHOT.md:166-191`，`/home/sevenx/coding/agent-harness/AGENTS.md:278-293`。

5. 清理：`scripts/wt-clean.sh <branch-or-dir>` 只清命名的 worktree。它拒绝 main，要求该分支曾以自己的名字 push 过，要求远端同名分支已消失，要求工作树干净，然后才 `git worktree remove` 和删除本地分支；`--all` 是显式全局 sweep。证据：`/home/sevenx/coding/agent-harness/scripts/wt-clean.sh:1-20`，`/home/sevenx/coding/agent-harness/scripts/wt-clean.sh:39-70`，`/home/sevenx/coding/agent-harness/scripts/wt-clean.sh:76-97`。

6. 并行礼仪：其他 agent 的 worktree 通过 `git worktree list` 可见；有未提交改动、open PR、近期文件修改的 worktree 视为 active，不能编辑、杀进程、清理。证据：`/home/sevenx/coding/agent-harness/docs/development/RUN_AND_SCREENSHOT.md:193-200`。

## §C git 全流程物化

| 环节 | agent-harness 状态 | 物化位置 |
| --- | --- | --- |
| 基线与分支 | 工具化。`wt-new.sh` 从 `origin/main` 切 `<type>/<short-desc>`，目录名把 `/` 映射为 `-`。 | `AGENTS.md:9-21`，`scripts/wt-new.sh:25-33` |
| 分支命名 | 半工具化。脚本要求 `<type>/<short-desc>`，但没有校验 type 枚举或 slug 规则。 | `scripts/wt-new.sh:4-12` |
| 开发隔离 | 工具化。一任务一 worktree，`.worktrees/` gitignored，主仓根是 main。 | `.gitignore:1-2`，`AGENTS.md:19-21` |
| 本地预览 | 工具化。`wt-dev.sh` 自动找 Vite/sidecar 端口，按是否 `--backend` 决定代理主 sidecar 或当前 worktree 私有 sidecar。 | `scripts/wt-dev.sh:38-59`，`scripts/wt-dev.sh:80-124` |
| 本地门禁 | 文档 + CI。根规则列本地应跑命令；CI 真正强制 PR 门禁。没有看到单个 `check-all` 脚本把所有本地门禁串起来。 | `AGENTS.md:105-123`，`.github/workflows/ci.yml:17-158` |
| commit 规范 | 文档约定 + squash commit 实践。`CONTRIBUTING.md` 明说 Conventional Commits；本地 git log 最近 40 条也呈现 `feat(scope): ... (#n)` / `fix(scope): ... (#n)`。未看到 commit-msg hook 强制。 | `docs/development/CONTRIBUTING.md:21-25`，本地 `git log --oneline -40` |
| PR 创建 | 工具化。`wt-ship.sh` push 当前分支，复用已有 PR 或创建 PR，title 可传入，否则 `--fill`。 | `scripts/wt-ship.sh:22-30` |
| 合并 | 工具化 + 仓库设置。`wt-ship.sh` 启用 auto-merge squash；根规则记录 main protected、PR required、required checks、squash-only、delete branch on merge。 | `scripts/wt-ship.sh:32-38`，`AGENTS.md:99-103` |
| 清理 | 工具化。按命名 worktree 清理，remote branch gone + clean tree 才删；避免启动新任务时扫掉别人。 | `scripts/wt-clean.sh:39-70`，`AGENTS.md:49-58` |
| PR 模板 | 未发现。`.github/` 下只有 CODEOWNERS、dependabot 和 workflows，没有 PR template 文件。 | `find .github -maxdepth 3` 结果 |
| CODEOWNERS | 部分物化。关键契约/规范文件有 owner，但主流程仍是 0 approvals 的 auto-merge 设定。 | `.github/CODEOWNERS:1-10`，`AGENTS.md:99-103` |

补充：`CLAUDE.md` 把 `AGENTS.md` 作为跨工具规则入口，并要求证据先行、文件+行号举证；这让 Claude/Codex 等不同 agent 读取同一套规则，而不是每个工具维护一份 SOP。证据：`/home/sevenx/coding/agent-harness/CLAUDE.md:3-6`，`/home/sevenx/coding/agent-harness/CLAUDE.md:17-29`。`docs/development/CONTRIBUTING.md` 也只指向 `AGENTS.md`，避免流程双写漂移。证据：`/home/sevenx/coding/agent-harness/docs/development/CONTRIBUTING.md:11-13`。

## §D 值得 ah 学的 N 条

### 1. 一任务一 worktree，根工作树只保 main

是什么：把并行隔离从“大家自觉别互相踩”变成文件系统级隔离：每个任务在 `.worktrees/<branch-slug>/` 有自己的 checkout，主仓根保持 `main`。agent-harness 物化在根规则和 `.gitignore`，并由 `wt-new.sh` 创建。证据：`/home/sevenx/coding/agent-harness/AGENTS.md:19-21`，`/home/sevenx/coding/agent-harness/.gitignore:1-2`，`/home/sevenx/coding/agent-harness/scripts/wt-new.sh:27-33`。

为什么值得 ah 学：ah 当前共享单工作树，天然放大“worker 改动互相覆盖、测试状态互相污染、PM 串行 commit”的成本。worktree 是最直接的隔离原语。

照搬风险：ah 有 master/worker/PM-proxy 状态机和 tmux/pane 资源，不能只创建 git worktree；还要把 agent session 的 cwd、端口、日志、清理绑定到任务 worktree。

### 2. 开工脚本必须从 origin/main 切新分支，并顺手预热依赖

是什么：`wt-new.sh` 固定 `git fetch origin --prune` 后从 `origin/main` 切新 worktree，同时后台启动 `npm ci` / `uv sync`，让“基线正确”和“依赖准备”都不靠人工。证据：`/home/sevenx/coding/agent-harness/scripts/wt-new.sh:25-33`，`/home/sevenx/coding/agent-harness/scripts/wt-new.sh:35-60`。

为什么值得 ah 学：ah 可以把“从最新 main 开始、不要从脏树开工、开工后环境可测试”做成 `ah task start` 级别的动作，减少 PM 逐条提醒。

风险/不适配：Rust 项目依赖预热可能是 `cargo fetch` / target dir 策略，不应盲目复制 npm/uv；要避免多个 worktree 共用 target 时产生锁竞争。

### 3. 发 PR 和启用 auto-merge 用脚本收口

是什么：`wt-ship.sh` 拒绝在 main/HEAD 上发货，拒绝 dirty tree，push 当前分支，创建或复用 PR，然后 `gh pr merge --auto --squash`。证据：`/home/sevenx/coding/agent-harness/scripts/wt-ship.sh:12-33`。

为什么值得 ah 学：ah 现在“worker 不 push、PM audit 后 push/PR/merge”有价值，但机械环节仍靠人。可以把“确认审查通过后 ship”的机械动作工具化，同时保留 PM 审查门。

风险/不适配：agent-harness 是 0 approvals auto-merge；ah 如果需要 PM/a2 审查，不应直接启用无审查 auto-merge。可先学脚本化 push+PR+检查，不学自动合并策略。

### 4. 清理脚本默认只清命名 worktree，绝不扫别人

是什么：`wt-clean.sh` 无参数只打印 usage；默认只处理用户命名的 worktree/branch；只有显式 `--all` 才全局 sweep。清理前还校验 branch 非 main、曾以同名 push、远端分支已消失、工作树干净。证据：`/home/sevenx/coding/agent-harness/scripts/wt-clean.sh:8-20`，`/home/sevenx/coding/agent-harness/scripts/wt-clean.sh:39-70`，`/home/sevenx/coding/agent-harness/scripts/wt-clean.sh:82-97`。

为什么值得 ah 学：多 worker 并行后，清理是高危动作。默认 own-scoped 能防止“开新任务/扫尾时删掉别人未完成工作”。

风险/不适配：ah 还要清 tmux panes、agent DB rows、socket/log 文件；清理条件要用 ah 自己的任务状态 + git 远端状态联合判断。

### 5. per-worktree 预览要连到“自己的代码”，不能误验 main

是什么：`wt-dev.sh` 默认每个 worktree 自己开 Vite；后端/engine/gateway 改动用 `--backend` 从当前 worktree 起私有 sidecar，否则会误连 main 的 sidecar。文档把“不要在 5173 验自己的活”写成明确约束。证据：`/home/sevenx/coding/agent-harness/scripts/wt-dev.sh:80-124`，`/home/sevenx/coding/agent-harness/docs/development/RUN_AND_SCREENSHOT.md:175-186`，`/home/sevenx/coding/agent-harness/docs/development/FRONTEND_HANDOFF_PROMPT.md:95-103`。

为什么值得 ah 学：ah 若引入 worktree，测试/运行命令必须绑定到该 worktree，否则“看起来过了”的验证可能实际跑的是主树或另一个 worker 的代码。

风险/不适配：ah 是 CLI/runtime 项目，未必有前端 dev server；但同理适用于 `cargo test`、ahd socket、tmux server、sandbox HOME 等运行资源。

### 6. 把分支/PR/合并规则写进一个跨工具 SSOT

是什么：`CLAUDE.md` 明确 canonical project rules 在 `AGENTS.md`，并 `@AGENTS.md` 导入；`CONTRIBUTING.md` 也只指向 AGENTS，不复述流程。证据：`/home/sevenx/coding/agent-harness/CLAUDE.md:3-6`，`/home/sevenx/coding/agent-harness/CLAUDE.md:29`，`/home/sevenx/coding/agent-harness/docs/development/CONTRIBUTING.md:11-13`。

为什么值得 ah 学：ah 现在有 AGENTS.md，但许多运行规则仍由 PM prompt 现场携带。把 worktree/git 生命周期规则放进单一、跨工具入口，能减少不同 agent 读到不同流程。

风险/不适配：文档 SSOT 仍不是强制执行；关键路径要继续下沉为 `ah` 子命令或脚本。

### 7. CI required checks 做合并门，本地门禁做提前反馈

是什么：CI 在 pull_request 和 main push 上跑；quality gates 包括 ruff/mypy/pytest/pip-audit，frontend gates 包括 lint/typecheck/test/build，graph-agent tests 覆盖 Python 3.11/3.12/3.13。根规则记录这些是 required checks。证据：`/home/sevenx/coding/agent-harness/.github/workflows/ci.yml:1-6`，`/home/sevenx/coding/agent-harness/.github/workflows/ci.yml:17-158`，`/home/sevenx/coding/agent-harness/AGENTS.md:99-123`。

为什么值得 ah 学：ah 的 review/audit 强，但如果合并门没有机器强制，PM 仍要当人肉 gatekeeper。把必过测试和 lint 固化成 required checks，可以把 PM 注意力留给行为和设计判断。

风险/不适配：ah 的真实 provider/e2e 测试可能不适合作 required check；需要拆 required smoke 和 manual/real-provider observation。

### 8. 记录“哪些共享文件不能并行改”，比事后解冲突便宜

是什么：agent-harness 在规则里点名 design tokens、`components/ui/`、手册 `index.html` 等共享文件会跨并行 PR 冲突，需要串行或指定 owner；任务书规范也要求 `owns_files` 不得相交。证据：`/home/sevenx/coding/agent-harness/AGENTS.md:291-293`，`/home/sevenx/coding/agent-harness/docs/development/task-spec-standard.md:50`。

为什么值得 ah 学：worktree 只能隔离工作目录，不能自动解决同一文件的逻辑冲突。ah 若并行 worker，需要把“文件所有权/冲突热点”前置到派单层。

风险/不适配：如果 ah 任务粒度较小，可先从“声明 touched files / owned files”开始，不必一次做完整文件锁系统。

### 9. 合并后的主树刷新和运行态重建也要 SOP 化

是什么：AGENTS.md 明确合并后 root 要 `git pull`，依赖清单变更要在 root 补装，engine/gateway 源码变更要重建 vendored sidecar，否则主 app 会跑旧代码。证据：`/home/sevenx/coding/agent-harness/AGENTS.md:59-68`，`/home/sevenx/coding/agent-harness/AGENTS.md:69-97`。

为什么值得 ah 学：ah 未来若 worktree 并行，main/root 运行态也会滞后于已合并代码。把“合并后刷新主运行环境”纳入工具或 checklist，能减少“PR 已合但 master/ahd 仍跑旧二进制”的误判。

风险/不适配：Rust 二进制重建/重启比前端依赖补装更直接，但涉及 ahd 长驻进程，需要设计无损重启或明确停机窗口。

## 待定/未坐实

- GitHub branch protection 的具体后台配置无法从本地文件直接验证；AGENTS.md 记录了 `enforce_admins`、required checks、squash-only、auto-merge、delete-branch-on-merge。证据是规则文档本身：`/home/sevenx/coding/agent-harness/AGENTS.md:99-103`。
- `gh pr list` 因未认证不可用，不能核对最近 20 个 merged PR 的远端元数据；本报告只用本地 `git log --oneline -40` 观察 squash commit 形态。
- 没看到 PR template，也没看到 commit-msg hook 强制 Conventional Commits；这些更像“文档约定 + 项目实践”，不是工具硬门。
