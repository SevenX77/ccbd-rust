# playbook · 运行时 / 基础设施故障(项目层)

进入条件:socket 泄漏、磁盘满、栈失联/islanding、沙箱堆积、进程僵死、ahd 挂——基础设施坏了,你在决定怎么救。
通用细则见 `research/config-pack/pack/OPERATOR-HANDBOOK.md`「场景细则 · 运行时故障」章。

## 本项目的标准恢复手段

栈级复位:`ah stop` → 清理残留(socket、僵尸 tmux session)→ `ah start`,分钟级闭环后回主线。

实锤(2026-07-13,obs #59):worker 误删活栈 master socket 致 islanding。事实:本质是一次分钟级栈重启,却被错判为业务 SOP 断点,处置停在写事故记录,系统持续瘫痪;该记录事后被裁定为无可交付价值。教训:此类故障按本节恢复手段直接闭环。

## 本项目已知的复发病:开修复工单,不再记录

下列故障属于已登记的复发性基础设施缺陷。复现时必须立即开修复工单并派发实施,禁止仅追加观察记录:

- tmux/socket 泄漏 → 工单方向:活栈 socket 与测试垃圾的命名隔离。
- cargo 测试残留 → 工单方向:测试自清理 fixture。
- 沙箱不 GC(曾累积 445 个沙箱、48G,打满磁盘)→ 工单方向:沙箱 GC。

## 隔离红线(发凭据/搭栈时的运维约束,原 O4)

worker 沙箱只挂鉴权 + 二进制,配置全独立;共享同一份可变状态是登出/越权/竞态的源头,能隔离就隔离。本项目实锤:

- 共享 OAuth 凭据 symlink:任一进程刷新 token,其余全部登出(ah#18)。
- master+worker 共享同一 git 树:两个 worker 不能同时分支/commit,代码任务必须进独立 worktree;markdown 类可并行。
- worker 沙箱多挂了配置:worker 越权把自己当主控。

这一问题域同时由 Module D 凭据设计处理;对你而言它是发凭据、搭栈时的运维红线。

## 本机 cargo 约束(VPS,原 O5)

本机 cargo 必须串行:`CARGO_BUILD_JOBS=1` + `--test-threads=1`,否则并行编译/测试会 OOM 把主控杀掉。串行只是防本机 OOM 的兜底约束,不是验收标准:串行绿 ≠ 并发安全,CI 并行绿才是真验收。

- 实锤(PR#146):ubuntu 测试在本地被真凭据文件掩蔽,到 CI 才暴露 HOME-override bug,连红多轮。
- 平台特定的修改(如 windows-only cfg)本机 linux 无法真验:让 worker 用 `cargo check` + CI-windows 验证,不逼本机跑全量串行测试。
