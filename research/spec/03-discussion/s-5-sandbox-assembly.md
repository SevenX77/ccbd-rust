# S-5：沙盒挂载映射与进程组装 (Sandbox & Process Assembly)

> **设计哲学**：进程启动是一场精密的手术。必须通过严格的步骤将 Systemd 的资源隔离、Bubblewrap 的文件/网络沙盒，以及跨进程的 UDS 管道拼装成一个安全的执行体。在这一阶段，宁可「启动失败」也绝不「带病奔跑」。

## 1. 挂载路径解算规则 (Path Resolution)

L2 调度层必须遵循 XDG 规范，且绝对禁止在沙盒外保留孤儿文件。

### 1.1 内部工作区隔离 (R-ISOLATION-1)
- **Base State Dir**: `~/.local/state/ccbd/` (或由 `CCBD_STATE_DIR` 覆盖)
- **Agent Sandbox Dir**: `{Base_State_Dir}/sandboxes/{agent_id}/`

### 1.2 外部挂载边界 (所有权矩阵)
- 沙盒**内部**视角的挂载点必须收敛到 `/workspace` 和 `/home/agent`。
- 外部的 Git Repo 只允许绑定到 `/workspace`。

---

## 2. 进程组装算法伪代码 (Pseudocode)

以下是处理 `agent.spawn` RPC 请求时的完整组装和防御回滚逻辑。

```rust
struct ProcessAssembler {
    agent_id: String,
    provider_profile: ProviderProfile,
    sandbox_dir: PathBuf,
}

impl ProcessAssembler {
    fn build_and_spawn(&self, overrides: Option<SandboxOverrides>) -> Result<SpawnedAgent, SandboxError> {
        
        // 1. 环境前置检查 (A-4 防静默降级)
        if !is_binary_available("bwrap") {
            if env::var("CCBD_UNSAFE_NO_SANDBOX") != Ok("1".into()) {
                return Err(SandboxError::BwrapNotFound); // 结构化错误
            }
        }

        // 2. 准备物理沙盒目录
        fs::create_dir_all(&self.sandbox_dir)?;
        
        // 3. 构建 bwrap 参数栈 (Baseline)
        let mut bwrap_args = vec![
            "--unshare-pid", "--unshare-uts", "--unshare-ipc",
            "--proc", "/proc",
            "--dev", "/dev",
            "--tmpfs", "/tmp",
            "--ro-bind", "/usr", "/usr",
            "--ro-bind", "/etc/resolv.conf", "/etc/resolv.conf", // 仅透传 DNS
            "--dir", "/home/agent", // 创建虚拟 HOME
            "--setenv", "HOME", "/home/agent",
        ];

        // 4. 解析 Provider 契约与 Overrides
        let net_mode = overrides.network.unwrap_or(self.provider_profile.network);
        if net_mode == "host" {
            bwrap_args.push("--share-net");
        } else {
            bwrap_args.push("--unshare-net");
        }

        // 处理自定义只读挂载 (必须校验是否越权访问系统敏感目录)
        for bind in self.provider_profile.extra_ro_binds.iter() {
            Self::validate_safe_path(bind.host_path)?;
            bwrap_args.extend(["--ro-bind", bind.host_path, bind.sandbox_path]);
        }

        // 5. 组装最终的 Command
        // 外层：Systemd Cgroup (A-6 决议)
        let mut cmd = Command::new("systemd-run");
        cmd.args([
            "--user",
            "--scope",
            "--slice=ccbd-agents.slice",
            // 如果 L2 崩溃，BindsTo 确保 Systemd 级联杀掉 Agent
            "--property=BindsTo=ccbd-rust.service", 
        ]);

        // 中层：bwrap 沙盒
        cmd.arg("bwrap").args(&bwrap_args);

        // 内层：目标二进制与 PTY 绑定
        cmd.arg(&self.provider_profile.entrypoint);
        
        // 6. 配置 PTY (A-3 决议的 portable-pty)
        let pty_system = native_pty_system();
        let pty_pair = pty_system.openpty(PtySize { rows: 200, cols: 200, .. })?;
        
        // 剥离当前进程的 stdio，将子进程的 stdio 重定向到 PTY 的 slave 端
        cmd.stdin(pty_pair.slave.try_clone_reader()?);
        cmd.stdout(pty_pair.slave.try_clone_writer()?);
        cmd.stderr(pty_pair.slave.try_clone_writer()?);

        // 7. 物理 Spawn
        let mut child = cmd.spawn()?;

        // 8. 附加 pidfd 监控 (A-5 决议)
        // 注意：因为使用了 --scope，child.id() 就是真正被包裹的根进程组 PID
        let pidfd = unsafe { libc::pidfd_open(child.id() as i32, 0) };
        if pidfd < 0 {
            // 失败回滚：如果拿不到 pidfd，说明内核不支持或进程已秒退，立即中止
            let _ = child.kill();
            return Err(SandboxError::PidfdAttachFailed);
        }

        // 9. 构建执行体并返回
        Ok(SpawnedAgent {
            process: child,
            pidfd,
            pty_master: pty_pair.master,
        })
    }
    
    fn validate_safe_path(path: &Path) -> Result<(), SandboxError> {
        // 阻止挂载 /etc/shadow, /root 等
        if path.starts_with("/etc/") || path.starts_with("/root") {
            return Err(SandboxError::MountPathForbidden);
        }
        Ok(())
    }
}
```

---

## 3. 失败回滚与孤儿收割 (Rollback & Reaper)

*   **Spawn 期失败**：如果在执行 `Command::spawn()` 后，获取 `pidfd` 或绑定 Epoll 失败，L2 必须立刻调用 `child.kill()`，并同步删除刚刚在步骤 2 创建的 `sandbox_dir`。此时数据库（S-2）事务直接 ROLLBACK，不留下任何状态变更。
*   **运行时崩溃**：由于采用了 `systemd-run --scope`，一旦包裹在内部的 `bwrap` 或 Agent 因为 OOM 等原因退出，`pidfd` 会立刻触发 `EPOLLIN` 唤醒 L2（S-1 状态机的 `pidfd.death` 转移），L2 在 CAS 更新状态为 `CRASHED` 后，执行物理目录回收。
