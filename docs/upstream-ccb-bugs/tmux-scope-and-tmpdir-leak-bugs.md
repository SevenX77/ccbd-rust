# 上游 CCB Bug：ccbd-tmux scope + /tmp 工作目录持续泄漏（合并报告）

| 字段 | 值 |
|---|---|
| **状态** | 全部未修；本次（2026-05-05）VPS 上手动清理一次但 leak 源头还在 |
| **首次集中观察** | 2026-05-05 在 VPS `vultr-sever-sv` 上做服务器健康检查时发现 |
| **影响范围** | 所有跑 python ccbd（`/home/sevenx/.local/share/codex-dual/lib/ccbd/`）的项目；越长寿越严重；服务器异常重启或 ccbd 主进程异常退出后必发 |
| **所属仓库** | 上游 [bfly123/claude_code_bridge](https://github.com/bfly123/claude_code_bridge) python 实现，**不是** ccbd-rust 仓库；本文是给 ccbd-rust 重写者的设计输入，明确这些坑要在 rust 实现里避开 |
| **优先级建议** | 高 — 单次 VPS 清理释放 1.6G 磁盘 + 1.2G 内存 + 424 个孤儿进程 + 232 个孤儿 systemd scope，泄漏速度足够在两周内把 7.7G RAM 的小机器搞崩 |
| **撰写人** | Claude Opus 4.7（主控）+ sevenx（识别 + 决策方向） |
| **关联文档** | `docs/upstream-ccb-bugs/gemini-dispatch-and-completion-bugs.md`（同一个 ccbd 的另外三个 bug）；`docs/dispatch-design-lessons.md`（设计教训风格参考） |
| **建议交付者** | 任何对 `claude_code_bridge` 仓有写权限的开发者；或 ccbd-rust 重写者直接把这些坑当 must-not-repeat 设计输入 |

---

## 0. TL;DR — 一句话讲清楚发生了什么

**python ccbd 通过 `systemd-run --scope` 把每个 agent 的 tmux server 起在独立的 `ccbd-tmux-*.scope` 里，但 scope 跟 ccbd 主进程没有任何 systemd 级别的生命周期绑定关系。结果就是：ccbd 主进程一旦异常退出（OOM 杀、服务器重启被打断、人工 SIGKILL），它创建的所有 ccbd-tmux scope 全部成为孤儿——systemd 视它们为合法 active scope，永远不会自动清；scope 里的 tmux server 因为 PPID=1 是 init 也不会自然退出；scope 里那个空 bash 进程更是会一直活着到天荒地老。同时每个 scope 关联的 `/tmp/.tmpXXX` 工作目录也没人删，长期滚雪球积累成几千个无主目录加上几万个零散文件。**

最坏的不是磁盘空间——是 systemd 的 scope unit 永远 active running 这件事让 janitor、自动清理脚本、监控告警全部失效，**只能靠人工一次次手动重启服务器才能 reset**。

---

## 1. 现场实测数据（2026-05-05 03:00 LA 时间）

### 1.1 清理前服务器状态

```
- 进程总数:           669
- 内存使用:           3.5G / 7.7G（available 3.9G）
- /tmp 占用:          2.1G
- /tmp 顶层条目:      15828
- 其中 .tmp* 目录:    915 个
- 其中 .tmp* 顶层文件: 14330 个
- ccbd-tmux-*.scope:   239 个（systemctl --user list-units 显示 loaded active running）
- /tmp/tmux-1001/ 下的 socket: 253 个
- who / utmp 登录会话: 234 个（实际只有 17 个真 ssh）
```

### 1.2 关键活物（清理时必须保留）

| PID | 命令 | 角色 | 备注 |
|---|---|---|---|
| 2838492 | `python3 ccbd/keeper_main.py --project /home/sevenx/coding/ccbd-rust` | ccbd-rust 项目 keeper | 在 session-505.scope 下 |
| 2838493 | `python3 ccbd/main.py --project /home/sevenx/coding/ccbd-rust` | ccbd-rust 项目 ccbd 主进程 | 在 session-505.scope 下 |
| 2838510 | `tmux -S .ccb/ccbd/tmux.sock ... -s ccb-ccbd-rust-29acbe42 -n __ccb_ctl` | ccbd-rust 控制平面 tmux | 不在 ccbd.slice |
| 2838981 | `claude --setting-sources user,project,local --dangerously-skip-permissions` | ccbd-rust 项目 master claude | 不在 ccbd.slice |
| 3934895 | `claude --dangerously-skip-permissions --continue /remote-control` | /remote-control master | 不在 ccbd.slice |
| 914709/747/791 | `tmux -L ccbd-4057850c... + node gemini` | `.ccb-rs` 测试实例的活 gemini | **在** `ccbd-tmux-4057850c.scope` 内，必须从 kill list 排除 |
| 2777336 | `tmux -L ccbd-cce74fe6... -c /tmp/ccb-rs-test-2777298/.ccb-rs/ccbd` | `/tmp/ccb-rs-test-...` 测试目录的 tmux | **在** `ccbd-tmux-cce74fe6.scope` 内，必须从 kill list 排除 |

剩下 **237 个 ccbd-tmux scope 是确认孤儿**，cmdline 里 `-c` 参数全部指向已废弃的 `/tmp/.tmpXXX` 目录，cgroup 里只有 tmux server + 空 bash 两个进程，没有任何 agent process（gemini / codex / claude）在跑。

### 1.3 清理后状态对照

| 指标 | 清理前 | 清理后 | 净变化 |
|---|---|---|---|
| 进程总数 | 669 | 245 | **-424** |
| 内存使用 | 3.5G | 2.3G | **-1.2G** |
| 可用内存 | 3.9G | 5.1G | **+1.2G** |
| `/tmp` 占用 | 2.1G | 538M | **-1.6G** |
| `/tmp` 顶层条目 | 15828 | 586 | **-15242** |
| `ccbd-tmux-*.scope` | 239 | 7（2 keeper + 5 新 spike）| **-232** |
| `/tmp/tmux-1001/` socket 数 | 253 | 16 | **-237** |
| `who` tty 会话 | 234 | 17 | **-217** |

清理过程中**0 个活物受损**——三个 master claude、python ccbd 主链、`.ccb-rs` 活 gemini、`/tmp/ccb-rs-test` 测试实例全部健在。

---

## 2. 五个 Bug 概要

| # | 短名 | 严重性 | 一句话现象 | 一句话根因 |
|---|---|---|---|---|
| **A** | scope-not-bound-to-ccbd | **P0** | ccbd 死后它创的 ccbd-tmux scope 全部成孤儿，systemd 永远不清 | systemd-run 起 scope 时没加 `BindsTo=` 或 `PartOf=` 关系 |
| **B** | mkdtemp-leak-on-fork-failure | **P1** | `prepare_namespace_root_pane` 在 fork 失败抛 `BlockingIOError` 时已创的 tmpdir + scope 不回滚 | 调用没用 try/finally 或 contextmanager 包，失败路径就是直接抛 |
| **C** | janitor-prefix-mismatch | **P2** | `claude-ccb-orchestrator` 的 janitor 扫的命名前缀跟 ccbd 实际起的 scope 名字对不上 | 一边叫 `claude-ccb-*.service`，一边叫 `ccbd-tmux-*.scope`，两套命名族没人对齐 |
| **D** | no-startup-reconcile | **P3** | ccbd 重启时不扫 `ccbd.slice` 里有没有上次没收的孤儿 | 启动 hook 缺 reconcile 步骤 |
| **E** | agent-restart-loop-without-cleanup | **P4** | codex a1 / gemini a2 / claude a3 在 InitGate 失败时被 supervisor 反复 restart，每次 restart 都新创 scope/tmpdir，旧的不清 | supervisor 的 `recover_started → recover_succeeded` 路径只管创新，不管销旧 |

外加一条**架构债**作为 P5（不是 bug 但是踩坑根源）：

| # | 短名 | 严重性 | 一句话现象 | 一句话根因 |
|---|---|---|---|---|
| **F** | flat-tmpdir-namespace | P5 | `/tmp/.tmpXXX` 散在 /tmp 顶层，跟系统其他 `tempfile.NamedTemporaryFile` 调用者混在一起 | `tempfile.mkdtemp()` 默认走 /tmp，ccbd 没用项目隔离前缀 |

---

## 3. Bug A — scope 跟 ccbd 主进程不绑定（P0，226 孤儿的元凶）

### 3.1 现象

VPS 上 `systemctl --user list-units 'ccbd-tmux-*.scope' --no-pager --plain --no-legend` 显示 239 个 scope，全部 `loaded active running`。其中 237 个的 cgroup 里只有两个进程：

```
ccbd-tmux-00024da7.scope:
  309192       1  1-04:41:26 /usr/bin/tmux -L ccbd-00024da7d39f2395 ...
  309209  309192  1-04:41:27 -bash
```

PPID=1 说明 tmux server 的父进程已经死了（它的父进程是当年起这个 scope 的 ccbd 主进程，那个 ccbd 因为某次崩溃或重启已经退出）。但 tmux 因为是 daemon 化的 server，不会因为父进程死就自己死；scope 因为没跟父进程做 systemd 绑定，systemd 也不会因为 ccbd 死就把 scope 一起收走。

cgroup 路径：
```
/sys/fs/cgroup/user.slice/user-1001.slice/user@1001.service/ccbd.slice/ccbd-agents.slice/ccbd-tmux-XXXX.scope
```

ccbd 主进程**不在**这条 cgroup 里——它在 `session-505.scope`（ssh 登录会话的 scope）下。所以两者是两条完全独立的 systemd 树，ccbd 死了对 ccbd.slice 完全没影响。

### 3.2 根因

ccbd 启动 agent 的 tmux server 时大致流程是：

```python
# 伪代码（基于 lib/ccbd/services/project_namespace_runtime/backend.py
# 和 lib/terminal_runtime/tmux_backend.py 反推）
subprocess.run([
    'systemd-run', '--user', '--scope',
    '--unit', f'ccbd-tmux-{short_id}',
    '--slice', 'ccbd-agents.slice',
    '/usr/bin/tmux', '-L', f'ccbd-{full_id}',
    'new-session', '-d', '-s', 'ccbd-agents',
    '-c', tmpdir,
    '-x', '200', '-y', '60',
])
```

这条命令缺关键的两个 systemd property：

- `--property=BindsTo=<ccbd-main-unit>`：让 scope 在 ccbd 主进程退出时被自动 stop
- `--property=PartOf=<ccbd-main-unit>`：让 scope 在 ccbd 主进程被 restart 时跟着 restart

加上其中任一条，systemd 会把 scope 跟 ccbd 主进程做生命周期绑定，ccbd 一退出 systemd 立刻 cascade 杀整片 scope 树。

### 3.3 ccbd-rust 的修法

ccbd-rust 重写时**强制要求** scope 必须有父子绑定关系。具体可以选两条路：

**方案 1：每个 ccbd 实例自己起一个 systemd service unit，所有 scope 挂在它下面**

```
[Unit]
Description=ccbd daemon for project <project_id>

[Service]
Type=simple
ExecStart=/usr/local/bin/ccbd-rust --project /path/to/project
Slice=ccbd-<project_id>.slice
```

然后 scope 起的时候：
```
systemd-run --user --scope \
  --slice=ccbd-<project_id>.slice \
  --property=BindsTo=ccbd-<project_id>.service \
  --property=PartOf=ccbd-<project_id>.service \
  /usr/bin/tmux ...
```

ccbd-rust 进程死了 → service unit 失败 → BindsTo 关系把所有 scope 一起 stop。

**方案 2：直接把 tmux server fork 在 ccbd-rust 进程的 cgroup 内（不用 systemd-run）**

更简单但失去了 systemd 视角的可观测性。如果 ccbd-rust 主进程崩了，所有 child tmux 也会被 cgroup OOM 一起带走（因为 cgroup 配置 `OOMPolicy=stop`）。但这种方式 systemd `list-units` 看不到 tmux 进程，调试不方便。

**推荐方案 1**——保留 systemd-run 的可观测性（每个 agent tmux 在 systemctl 里能单独看到状态、日志、资源占用），同时通过 `BindsTo=` 让生命周期绑死。

---

## 4. Bug B — mkdtemp + start-server 失败时不回滚（P1，新 leak 的元凶）

### 4.1 现象

清理完 226 个旧孤儿后 56 秒内，**5 个新的 ccbd-tmux scope 又冒出来了**，每个都带新 mkdtemp 出来的 `/tmp/.tmpXXX` 目录：

```
ccbd-tmux-290e16e5.scope  -c /tmp/.tmpDv6bdE
ccbd-tmux-3b559930.scope  -c /tmp/.tmpvmRrH6
ccbd-tmux-6f12bd59.scope  -c /tmp/.tmp5gjf9F
ccbd-tmux-fe7cb918.scope  -c /tmp/.tmpgNQDW1
ccbd-tmux-fef8149c.scope  -c /tmp/.tmp6msJOQ
```

每个 scope 里同样只有 tmux server + 空 bash 两个进程，没有任何 agent。

观察 `.ccb/ccbd/ccbd.stderr.log` 看到的 traceback：

```
  File "/home/sevenx/.local/share/codex-dual/lib/ccbd/services/project_namespace_runtime/ensure.py", line 34, in ensure_project_namespace
    prepare_namespace_root_pane(
  File "/home/sevenx/.local/share/codex-dual/lib/ccbd/services/project_namespace_runtime/ensure_identity.py", line 15, in prepare_namespace_root_pane
    prepare_server(context.backend)
  File "/home/sevenx/.local/share/codex-dual/lib/ccbd/services/project_namespace_runtime/backend.py", line 24, in prepare_server
    backend._tmux_run(['start-server'], check=False, capture=True)
  File "/home/sevenx/.local/share/codex-dual/lib/terminal_runtime/tmux_backend.py", line 81, in _tmux_run
    return _run([*self._tmux_base(), *args], check=check, **kwargs)
  File "/home/sevenx/.local/share/codex-dual/lib/terminal_runtime/tmux_backend.py", line 31, in _run
    return _sp.run(*args, **kwargs)
  File "/home/sevenx/.pyenv/versions/3.12.9/lib/python3.12/subprocess.py", line 1893, in _execute_child
    self.pid = _fork_exec(...)
BlockingIOError: [Errno 11] Resource temporarily unavailable
```

`BlockingIOError: [Errno 11]` 是 `EAGAIN`——fork() 因为打到 `RLIMIT_NPROC` 或 `pids.max` cgroup 限制无法创建新进程。当时服务器有 669 个进程，包括 226 个孤儿 scope 里的 226 个 tmux + 226 个 bash，已经接近 user@1001.service 的 pids.max 上限。

### 4.2 根因

`prepare_namespace_root_pane` 大致流程：

```python
# 伪代码
def prepare_namespace_root_pane(...):
    tmpdir = tempfile.mkdtemp(prefix='.tmp')  # 创目录 → 成功
    # systemd-run + tmux start-server     ← 这里 fork 失败抛 BlockingIOError
    prepare_server(context.backend)
    # ... 后面没机会跑
```

异常抛出后 stack 被 unwind，但 `tmpdir` 引用直接丢，**没有任何 cleanup 路径**——目录留在 `/tmp/.tmpXXX`。

更糟的是，scope 是 fork 之前就通过 `systemd-run --scope` 注册的（scope 注册不需要 fork tmux 子进程），所以**scope 已经在 systemd 里登记了**，只是它配的 ExecStart 那行 `/usr/bin/tmux ...` fork 失败了。这种半启动状态的 scope，systemd 通常会立刻 stop 自己（因为 ExecStart 失败），但不一定每次都干净——VPS 上观察到 5 个新 scope 是 active running 状态，里面也有 tmux server PID 在跑。

也就是说**至少一部分 fork 失败的 scope 实际上 fork 是成功了的**——可能是先 fork 出 tmux 但 tmux 后续某步失败了；或者在异常抛出前 tmux 已经独立 daemonize 完。具体路径需要在 ccbd 源码里追，但**结论已经够明确**：失败路径上目录和 scope 都不收。

### 4.3 ccbd-rust 的修法

任何 mkdtemp + 起子进程的组合**必须用 RAII / Drop / contextmanager 模式**，让失败路径上目录和 scope 都被清。Rust 风格：

```rust
struct NamespaceRoot {
    tmpdir: tempfile::TempDir,           // Drop 时自动 rmtree
    scope_unit: Option<ScopeHandle>,     // Drop 时 systemctl stop
}

impl Drop for NamespaceRoot {
    fn drop(&mut self) {
        if let Some(scope) = self.scope_unit.take() {
            scope.stop();  // systemctl --user stop ccbd-tmux-XXXX.scope
        }
        // tmpdir 由 TempDir 的 Drop 处理
    }
}

fn prepare_namespace_root_pane(...) -> Result<NamespaceRoot> {
    let mut root = NamespaceRoot { tmpdir: tempfile::tempdir()?, scope_unit: None };
    let scope = systemd_run_scope(&root.tmpdir.path())?;
    root.scope_unit = Some(scope);
    start_tmux_server_in_scope(&root)?;  // 失败时 root drop 自动清
    Ok(root)
}
```

把 scope handle + tmpdir 放进同一个 owning struct，调用方拿到 Result 后无论成功失败 Rust 编译器强制保证 Drop 一定跑。Python 时代靠程序员记得写 try/finally，Rust 时代靠类型系统强制。

---

## 5. Bug C — janitor 命名前缀跟 scope 不匹配（P2）

### 5.1 现象

`claude-ccb-orchestrator` 的 janitor（`~/.local/bin/claude-ccb-orchestrator`）每 5 分钟跑一次：

```
May 05 02:35:58 vultr-sever-sv claude-ccb-orchestrator[3381387]:
{"dry_run": false, "orphans_systemd_cleared": [],
 "orphans_systemd_killed": [], "orphans_tracking_removed": [],
 "scanned_systemd": 0, "tracked_count": 0}
```

`scanned_systemd: 0`——janitor 一个 scope 都没扫到。但同时间 `systemctl --user list-units 'ccbd-tmux-*.scope'` 有 226 个孤儿。

### 5.2 根因

janitor 内部硬编码扫的是 `claude-ccb-*.service`（claude-ccb-orchestrator 自己起 task scope 时用的命名），但 ccbd 起的 tmux scope 叫 `ccbd-tmux-*.scope`。两个命名族没有任何交集。

更深一层：这两套清理逻辑根本是两个独立的人在不同时间设计的——janitor 是 sevenx 自己写的 orchestrator 给 master claude task scope 用，ccbd 起 scope 是 upstream `claude_code_bridge` 设计的。**两边都假定对方的 scope 不需要自己管**，结果 ccbd-tmux-* 这一族成了"两边都不管的孤儿"。

### 5.3 ccbd-rust 的修法

ccbd-rust 自己起的 scope 命名前缀必须**显式跟现有 janitor 对齐**。具体两条路：

**方案 1：scope 命名改成 `claude-ccb-tmux-*.scope`**

直接复用现有 janitor 的扫描模式，零改动 janitor 侧。

**方案 2：ccbd-rust 自带 reconcile 子命令**

```
ccbd-rust janitor --reconcile
  → 扫 ccbd.slice 下所有 scope
  → 对照活的 ccbd-rust 主进程
  → 清掉无主的
```

并通过 systemd timer 定时跑（不依赖 sevenx 的 orchestrator janitor）。

**推荐方案 2**——ccbd-rust 不应该依赖 sevenx 的私人 orchestrator 工具，应该自带 janitor 能力。但 scope 命名可以同时遵循 `ccb-tmux-<project_id>-*.scope` 这种带项目隔离的模式，让 sevenx 的 orchestrator 和 ccbd-rust 自己都能扫到。

---

## 6. Bug D — ccbd 启动时不做"上次没清干净"扫描（P3）

### 6.1 现象

VPS 服务器至少经历过一次崩溃（4 月 30 日 pyenv-shim sentinel 的 mtime 是 `2026-04-30 00:01`，那次崩溃没让 pyenv-rehash 完成；详见第 9 节附记），崩溃前的 ccbd 进程留了一批孤儿 scope。崩溃后 ccbd 重启起来正常运行，**完全不知道**之前那批孤儿存在，新 ccbd 自己又开始堆新的孤儿。后续每次 ccbd 重启（被 systemd-oomd 杀过、被人手动 restart 过）都重复这个模式，孤儿数量线性上涨。

### 6.2 根因

ccbd 的 startup hook（`startup-report.json` 写入路径）只关心：
- 自己的 mailbox / queue / lifecycle 状态恢复
- agent ready-detection
- snapshot 重放

**完全不扫 systemd 这一层**——不知道 `ccbd.slice/ccbd-agents.slice/` 下面有什么 scope，更不会去清。

### 6.3 ccbd-rust 的修法

启动 hook 必须加一个 reconcile 步骤：

```rust
// 伪代码
fn startup_reconcile() -> Result<()> {
    // 1. 列出 ccbd.slice 里所有 scope
    let all_scopes = systemctl_list_scopes("ccbd-tmux-*.scope")?;

    // 2. 跟当前 ccbd-rust 主进程要管的 agent 列表对照
    let active_agent_scopes = self.agent_registry
        .iter()
        .map(|a| a.scope_unit_name())
        .collect::<HashSet<_>>();

    // 3. 不在活 agent 列表里的就是孤儿
    let orphans = all_scopes
        .iter()
        .filter(|s| !active_agent_scopes.contains(s))
        .collect::<Vec<_>>();

    // 4. 全部 stop
    for orphan in orphans {
        info!("startup_reconcile: stopping orphan scope {}", orphan);
        systemctl_stop_scope(orphan)?;
    }

    // 5. 清孤儿对应的 /tmp/.tmpXXX 目录
    cleanup_orphan_workdirs(&orphans)?;
    Ok(())
}
```

这个 reconcile 比 janitor 的优势是**有 agent registry 作为权威 ground truth**——janitor 只能猜，ccbd-rust 自己知道哪些 scope 是该自己管的。

---

## 7. Bug E — agent restart 死循环不带 cleanup（P4）

### 7.1 现象

`/home/sevenx/coding/ccbd-rust/.ccb/ccbd/supervision.jsonl` 里看到 codex a1 的状态序列（截取 33 秒一段）：

```jsonl
{"event_kind": "recover_started",   "agent_name": "a1", "occurred_at": "2026-05-03T16:07:29.513993Z",
 "prior_health": "pane-dead", "result_health": "pane-dead", "details": {}}
{"event_kind": "recover_succeeded", "agent_name": "a1", "occurred_at": "2026-05-03T16:07:29.513993Z",
 "prior_health": "pane-dead", "result_health": "healthy", "details": {"restart_count": 2}}
{"event_kind": "recover_started",   "agent_name": "a1", "occurred_at": "2026-05-03T16:07:34.475301Z",
 "prior_health": "pane-dead", "result_health": "pane-dead", "details": {}}
{"event_kind": "recover_succeeded", "agent_name": "a1", "occurred_at": "2026-05-03T16:07:34.475301Z",
 "prior_health": "pane-dead", "result_health": "healthy", "details": {"restart_count": 3}}
... (继续 5/6 次)
```

5 秒一次重启循环，`restart_count` 一路涨。每次 `recover_started → recover_succeeded` 之间 supervisor 创了一份新 scope + tmpdir，旧的不收。

更深一层：注意每条 `recover_succeeded` 都标 `result_health: "healthy"`，但**下一条 `recover_started` 又标 `prior_health: "pane-dead"`**——两者矛盾（健康才标 healthy，但下一秒就 dead 了）。说明 ccbd 的 health detection 跟 InitGate 之间有 race condition：标 healthy 时其实 agent TUI 还没真正 ready，几秒后 InitGate 真扫的时候发现没起来，重新 restart。

### 7.2 根因

两层错综在一起：

**根因 A：health detection 过于激进**——`recover_succeeded` 早于 InitGate 实际通过就标 healthy。

**根因 B：restart 路径只创不收**——`recover_started` 不会先把上一轮的 scope/tmpdir 释放掉。即使每轮 restart 失败 ccbd-rust 自己实现的 Drop 能清，supervisor 路径如果绕过那个 owning struct（比如 `recover_started` 直接调底层的 systemd-run 不走 RAII 包装），还是会 leak。

### 7.3 ccbd-rust 的修法

**修 A**：health 标志只在 InitGate 真正通过 + 第一个心跳信号收到后才设为 healthy。`recover_succeeded` 跟 `health_set_to_healthy` 应该是两个独立事件，不能一个写两个。

**修 B**：`recover_started` 必须先调用旧 scope 的 Drop（或显式 stop + cleanup），再起新 scope。不允许"上一轮还没收尾就开始下一轮"。可以用一个 `agent_lifecycle_lock` 保证 restart 的原子性。

---

## 8. Bug F — 扁平化的 /tmp 命名空间（P5，架构债）

### 8.1 现象

ccbd 创的工作目录全部走 `tempfile.mkdtemp()` 默认参数，结果落在 `/tmp/.tmpXXX` 这种通用前缀里。VPS 上 `/tmp` 顶层除了 ccbd 自己创的 915 个目录 + 14330 个文件以外，还有：

- `/tmp/agent-harness-backup-1777407779`（231M）
- `/tmp/npm-cache`（103M）
- `/tmp/claude-1001`（66M）
- `/tmp/node-compile-cache`（30M）
- `/tmp/ccbd-shellcheck`（18M）
- ……

有的是 ccbd 自己其他模块创的，有的是 IDE / claude CLI / Node 创的。**完全没法批量按"ccbd 创的"过滤清理**——只能靠目录名暴力 glob `.tmp*`，这个 glob 还会误中其他 Python 程序的 NamedTemporaryFile 文件（虽然今天的清理实测没误伤，但理论上有风险）。

### 8.2 ccbd-rust 的修法

**所有 ccbd-rust 创的临时目录必须带项目隔离前缀**：

```
/tmp/ahd/<project_id>/<scope_uuid>/
```

或者用 XDG 标准：
```
$XDG_RUNTIME_DIR/ahd/<project_id>/<scope_uuid>/
```

`$XDG_RUNTIME_DIR` 通常是 `/run/user/<uid>`，systemd 自动按用户登出清理。

好处：
1. 一个 `rm -rf /tmp/ahd/<project_id>` 能清掉一个项目的所有残留
2. `find /tmp/ahd -mindepth 2 -maxdepth 2 -type d` 能列出所有 scope 工作目录
3. 不会误中或被误中其他程序的 tmpfile
4. `ls /tmp/ahd/` 一眼能看到当前活的项目

---

## 9. 附记：本次清理顺带发现并修复的 OS 级问题（不是 CCB 的责任）

### 9.1 pyenv stale lock 导致 ssh / tmux 启动卡 60 秒

VPS 上每次 ssh 进来 / tmux 起新窗口，bash 启动要卡 60 秒才到 prompt，按 Ctrl+C 能跳过。`time bash -i -c 'echo READY' < /dev/null` 实测 1m1.145s。

**元凶**：`/home/sevenx/.pyenv/shims/.pyenv-shim` 是个 299 字节的旧 sentinel 文件（mtime `2026-04-30 00:01`，是上次服务器崩溃打断 `pyenv rehash` 留下的）。pyenv-rehash 用 `mv` 把新 shim 原子覆盖到这个名字上做互斥锁，但旧文件挡着、`mv` 不能跨 inode 覆盖，每次新 shell 启动 pyenv init 都重试 60 秒才超时放弃。

`fuser` / `lsof` 都显示**没有进程持有这个文件**——纯 stale state，跟 ccbd-tmux scope 是同一类病（异常退出后没清自己的状态）。

**修法**：`rm /home/sevenx/.pyenv/shims/.pyenv-shim`。

修完之后 `time bash -i -c 'echo READY' < /dev/null` 实测 0.798s，缩了 75 倍。

**为什么放在这份 CCB bug 报告里说**：因为这是同一类病的不同症状——异常退出留下的文件系统 sentinel 文件，跟 systemd scope 留下的孤儿 unit 是镜像问题。ccbd-rust 设计时**对所有"程序退出时该清的状态"必须有 Drop/finally 兜底**，pyenv 这种 30 年前写的 shell 工具没这个能力，但用 Rust 的 ccbd-rust 没有任何理由不做。

---

## 10. 完整清理脚本（给 ccbd-rust 重写者参考）

下面这段脚本是 2026-05-05 在 VPS 上手动跑过的完整清理流程，可作为 ccbd-rust `janitor --emergency-cleanup` 子命令的参考实现：

```bash
#!/usr/bin/env bash
# ccbd 紧急清理：清孤儿 scope + tmpdir + stale socket。
# 使用前必须先识别 KEEP_SCOPES，避免误杀活物。

set -euo pipefail

# 1. 列出所有 ccbd-tmux scope，识别"活的"（cgroup 里有 agent process 的）
KEEP_SCOPES=()
for cgproc in /sys/fs/cgroup/user.slice/user-1001.slice/user@1001.service/ccbd.slice/ccbd-agents.slice/ccbd-tmux-*.scope/cgroup.procs; do
    scope=$(basename "$(dirname "$cgproc")")
    has_agent=0
    for p in $(cat "$cgproc" 2>/dev/null); do
        comm=$(cat /proc/$p/comm 2>/dev/null || true)
        # 真 agent 是 node (gemini/claude) / python (codex) / 其他二进制；
        # 单纯 tmux + bash 的不算
        case "$comm" in
            'tmux: server'|bash|sh|-bash) ;;
            *) has_agent=1; break;;
        esac
    done
    [ "$has_agent" = "1" ] && KEEP_SCOPES+=("$scope")
done

# 2. 构造 kill list（全部 - keepers）
ALL=$(systemctl --user list-units 'ccbd-tmux-*.scope' --no-pager --plain --no-legend | awk '{print $1}')
KILL=()
for s in $ALL; do
    keep=0
    for k in "${KEEP_SCOPES[@]}"; do
        [ "$s" = "$k" ] && keep=1 && break
    done
    [ "$keep" = "0" ] && KILL+=("$s")
done

# 3. 批量 stop（每批 50 个，避免参数过长）
for ((i=0; i<${#KILL[@]}; i+=50)); do
    systemctl --user stop "${KILL[@]:i:50}"
done

# 4. 清 /tmp/.tmpXXX 目录和顶层文件
find /tmp -maxdepth 1 -name ".tmp*" -type d -print0 | xargs -0 rm -rf
find /tmp -maxdepth 1 -name ".tmp*" -type f -delete

# 5. 清 stale tmux socket（ccbd-* 命名族中已无对应 server 的）
for sock in /tmp/tmux-1001/ccbd-*; do
    name=$(basename "$sock")
    if ! tmux -L "$name" list-sessions > /dev/null 2>&1; then
        rm -f "$sock"
    fi
done
```

ccbd-rust 重写时可以把这套清理逻辑实现为 `ccbd-rust janitor --reconcile` 子命令，并在主进程启动时自动跑一次（参考 Bug D 的修法）。

---

## 11. 总结：ccbd-rust 重写时必须做的五件事

1. **每个 scope 必须 `BindsTo=` ccbd-rust 主进程**——杜绝孤儿 scope 的源头（解 Bug A）
2. **所有 mkdtemp + start-process 组合用 RAII / Drop owning struct 包**——失败路径必清（解 Bug B）
3. **scope 命名前缀跟现有 janitor 对齐 OR ccbd-rust 自带 reconcile 子命令**（解 Bug C）
4. **启动 hook 必加 reconcile 步骤**——清上次没清干净的孤儿（解 Bug D）
5. **agent restart 路径必须先释放旧 scope 再创新 scope**——supervisor 不能只创不收（解 Bug E）

外加架构层：所有临时目录必须走 `/tmp/ahd/<project_id>/<scope_uuid>/` 这种带项目隔离的层次结构，禁用 `tempfile.mkdtemp()` 默认参数（解 Bug F）。

做到这五件事，VPS 上"两周不重启就崩"的循环就会被打破——ccbd-rust 进程异常退出 systemd 自动 cascade 清掉所有 scope；正常退出走 Drop 自己清；启动时再 reconcile 兜底。三层防护下，孤儿无处可生。
