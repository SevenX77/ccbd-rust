use crate::error::CcbdError;
use crate::tmux::{
    TmuxError, TmuxPaneId, compute_socket_name,
    scope::{self, ScopePolicy},
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Right,
    Bottom,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitSpec {
    pub parent: Option<TmuxPaneId>,
    pub direction: Option<SplitDirection>,
    pub percent: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct TmuxServer {
    socket_name: String,
    scope_policy: ScopePolicy,
}

impl TmuxServer {
    pub fn new(state_dir: &Path) -> Self {
        let socket_name = compute_socket_name(state_dir);
        let policy = scope::detect_scope_policy(&socket_name);
        Self::new_with_policy(state_dir, policy)
    }

    pub fn new_with_policy(state_dir: &Path, policy: ScopePolicy) -> Self {
        Self {
            socket_name: compute_socket_name(state_dir),
            scope_policy: policy,
        }
    }

    pub fn from_socket_name(socket_name: String) -> Self {
        let scope_policy = scope::detect_scope_policy(&socket_name);
        Self {
            socket_name,
            scope_policy,
        }
    }

    pub fn socket_name(&self) -> &str {
        &self.socket_name
    }

    pub(crate) fn ensure_session_sync(
        &self,
        session_name: &str,
        cwd: &Path,
    ) -> Result<(), CcbdError> {
        let has_session = Command::new("tmux")
            .args(["-L", &self.socket_name, "has-session", "-t", session_name])
            .output()
            .map_err(map_command_io_error)?;

        if has_session.status.success() {
            return Ok(());
        }

        let cwd_arg = cwd.display().to_string();
        let args = [
            "-L",
            &self.socket_name,
            "new-session",
            "-d",
            "-s",
            session_name,
            "-c",
            &cwd_arg,
            "-x",
            "200",
            "-y",
            "60",
        ];
        let output = scope::wrap_in_scope("tmux", &args, &self.scope_policy)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn spawn_window_sync(
        &self,
        session: &str,
        window: &str,
        cwd: &Path,
        cmd: &[&str],
    ) -> Result<TmuxPaneId, CcbdError> {
        if let Some(initial_window) = self.reusable_initial_window_sync(session)? {
            return self.respawn_initial_window_sync(session, &initial_window, window, cwd, cmd);
        }

        let target = format!("{session}:");
        let cwd_arg = cwd.display().to_string();
        let args = [
            "-L",
            &self.socket_name,
            "new-window",
            "-d",
            "-t",
            &target,
            "-n",
            window,
            "-c",
            &cwd_arg,
            "-P",
            "-F",
            "#{pane_id}",
            "--",
        ];
        let output = Command::new("tmux")
            .args(args)
            .args(cmd)
            .output()
            .map_err(map_command_io_error)?;
        ensure_output_success("tmux", &args, cmd, output)
            .and_then(|stdout| TmuxPaneId::parse(stdout.trim()).map_err(CcbdError::from))
    }

    fn reusable_initial_window_sync(&self, session: &str) -> Result<Option<String>, CcbdError> {
        // mvp12 M12.6: when ccbd starts fresh, the tmux session doesn't exist yet
        // and list-windows fails. Treat that as "no reusable window" rather than
        // propagating the error — new-window will create the session on first call.
        let windows = match self.list_window_names_sync(session) {
            Ok(w) => w,
            Err(_) => return Ok(None),
        };
        let Some(window) = reusable_initial_window(&windows) else {
            return Ok(None);
        };
        let panes = self.list_panes_sync(&format!("{session}:{window}"))?;
        if panes.len() == 1 {
            Ok(Some(window.to_string()))
        } else {
            Ok(None)
        }
    }

    fn list_window_names_sync(&self, session: &str) -> Result<Vec<String>, CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "list-windows",
            "-t",
            session,
            "-F",
            "#{window_name}",
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        let stdout = ensure_output_success("tmux", &args, &[], output)?;
        Ok(stdout.lines().map(ToString::to_string).collect())
    }

    fn respawn_initial_window_sync(
        &self,
        session: &str,
        initial_window: &str,
        window: &str,
        cwd: &Path,
        cmd: &[&str],
    ) -> Result<TmuxPaneId, CcbdError> {
        let old_target = format!("{session}:{initial_window}");
        let rename_args = [
            "-L",
            &self.socket_name,
            "rename-window",
            "-t",
            &old_target,
            window,
        ];
        let output = Command::new("tmux")
            .args(rename_args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &rename_args, output)?;

        let target = format!("{session}:{window}");
        let cwd_arg = cwd.display().to_string();
        let respawn_args = [
            "-L",
            &self.socket_name,
            "respawn-pane",
            "-k",
            "-t",
            &target,
            "-c",
            &cwd_arg,
            "--",
        ];
        let output = Command::new("tmux")
            .args(respawn_args)
            .args(cmd)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &respawn_args, output)?;

        let pane_args = [
            "-L",
            &self.socket_name,
            "display-message",
            "-p",
            "-t",
            &target,
            "#{pane_id}",
        ];
        let output = Command::new("tmux")
            .args(pane_args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_output_success("tmux", &pane_args, &[], output)
            .and_then(|stdout| TmuxPaneId::parse(stdout.trim()).map_err(CcbdError::from))
    }

    pub(crate) fn window_exists_sync(
        &self,
        session: &str,
        window: &str,
    ) -> Result<bool, CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "list-windows",
            "-t",
            session,
            "-F",
            "#{window_name}",
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        let stdout = ensure_output_success("tmux", &args, &[], output)?;
        Ok(stdout.lines().any(|line| line == window))
    }

    pub(crate) fn split_window_sync(
        &self,
        target: &str,
        cwd: &Path,
        cmd: &[&str],
    ) -> Result<TmuxPaneId, CcbdError> {
        self.split_window_with_spec_sync(target, cwd, cmd, SplitSpec::default())
    }

    pub(crate) fn split_window_with_spec_sync(
        &self,
        target: &str,
        cwd: &Path,
        cmd: &[&str],
        spec: SplitSpec,
    ) -> Result<TmuxPaneId, CcbdError> {
        let cwd_arg = cwd.display().to_string();
        let args = build_split_window_args(&self.socket_name, target, &cwd_arg, &spec);
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let output = Command::new("tmux")
            .args(&arg_refs)
            .args(cmd)
            .output()
            .map_err(map_command_io_error)?;
        ensure_output_success("tmux", &arg_refs, cmd, output)
            .and_then(|stdout| TmuxPaneId::parse(stdout.trim()).map_err(CcbdError::from))
    }

    pub(crate) fn get_pane_pid_sync(&self, pane: &TmuxPaneId) -> Result<i32, CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "display-message",
            "-p",
            "-t",
            &pane.0,
            "#{pane_pid}",
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        let stdout = ensure_output_success("tmux", &args, &[], output)?;
        stdout
            .trim()
            .parse::<i32>()
            .map_err(|_| CcbdError::from(TmuxError::ParsePid(stdout.trim().to_string())))
    }

    pub(crate) fn pipe_pane_to_fifo_sync(
        &self,
        pane: &TmuxPaneId,
        fifo: &Path,
    ) -> Result<(), CcbdError> {
        let pipe_command = format!("cat > {}", shell_quote_path(fifo));
        let args = [
            "-L",
            &self.socket_name,
            "pipe-pane",
            "-t",
            &pane.0,
            "-O",
            &pipe_command,
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn send_keys_literal_sync(
        &self,
        pane: &TmuxPaneId,
        text: &str,
    ) -> Result<(), CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "send-keys",
            "-t",
            &pane.0,
            "-l",
            text,
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn send_keys_keysym_sync(
        &self,
        pane: &TmuxPaneId,
        keysym: &str,
    ) -> Result<(), CcbdError> {
        let args = ["-L", &self.socket_name, "send-keys", "-t", &pane.0, keysym];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn send_ctrl_c_sync(&self, pane: &TmuxPaneId) -> Result<(), CcbdError> {
        self.send_keys_keysym_sync(pane, "C-c")
    }

    pub(crate) fn kill_pane_sync(&self, pane: &TmuxPaneId) -> Result<(), CcbdError> {
        let args = ["-L", &self.socket_name, "kill-pane", "-t", &pane.0];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn kill_window_sync(&self, session: &str, window: &str) -> Result<(), CcbdError> {
        let target = format!("{session}:{window}");
        let args = ["-L", &self.socket_name, "kill-window", "-t", &target];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn kill_session_window_sync(&self, session_id: &str) -> Result<(), CcbdError> {
        self.kill_window_sync(crate::tmux::SESSION_NAME, session_id)
    }

    pub(crate) fn capture_pane_sync(&self, pane: &TmuxPaneId) -> Result<String, CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "capture-pane",
            "-p",
            "-t",
            &pane.0,
            "-S",
            "-200",
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_output_success("tmux", &args, &[], output)
    }

    pub(crate) fn select_layout_sync(
        &self,
        window_target: &str,
        layout: &str,
    ) -> Result<(), CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "select-layout",
            "-t",
            window_target,
            layout,
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn list_panes_sync(
        &self,
        window_target: &str,
    ) -> Result<Vec<TmuxPaneId>, CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "list-panes",
            "-t",
            window_target,
            "-F",
            "#{pane_id}",
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        let stdout = ensure_output_success("tmux", &args, &[], output)?;
        stdout
            .lines()
            .map(|line| TmuxPaneId::parse(line.trim()).map_err(CcbdError::from))
            .collect()
    }

    pub(crate) fn load_buffer_sync(&self, buffer_name: &str, text: &str) -> Result<(), CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "load-buffer",
            "-b",
            buffer_name,
            "-",
        ];
        let mut child = Command::new("tmux")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(map_command_io_error)?;

        match child.stdin.take() {
            Some(mut stdin) => {
                if let Err(err) = stdin.write_all(text.as_bytes()) {
                    let _ = child.kill();
                    return Err(CcbdError::TmuxCommandFailed {
                        cmd: format!("tmux {}", args.join(" ")),
                        stderr: format!("stdin write failed: {err}"),
                        exit: -1,
                    });
                }
            }
            None => {
                let _ = child.kill();
                return Err(CcbdError::TmuxCommandFailed {
                    cmd: format!("tmux {}", args.join(" ")),
                    stderr: "stdin pipe unavailable".into(),
                    exit: -1,
                });
            }
        }

        let output = child.wait_with_output().map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn paste_buffer_sync(
        &self,
        pane: &TmuxPaneId,
        buffer_name: &str,
    ) -> Result<(), CcbdError> {
        let args = [
            "-L",
            &self.socket_name,
            "paste-buffer",
            "-p",
            "-t",
            &pane.0,
            "-b",
            buffer_name,
        ];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn delete_buffer_sync(&self, buffer_name: &str) -> Result<(), CcbdError> {
        let args = ["-L", &self.socket_name, "delete-buffer", "-b", buffer_name];
        let output = Command::new("tmux")
            .args(args)
            .output()
            .map_err(map_command_io_error)?;
        ensure_success("tmux", &args, output)
    }

    pub(crate) fn send_enter_sync(&self, pane: &TmuxPaneId) -> Result<(), CcbdError> {
        self.send_keys_keysym_sync(pane, "Enter")
    }

    pub async fn ensure_session(
        &self,
        session_name: String,
        cwd: PathBuf,
    ) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::ensure_session", move || {
            server.ensure_session_sync(&session_name, &cwd)
        })
        .await
    }

    pub async fn spawn_window(
        &self,
        session: String,
        window: String,
        cwd: PathBuf,
        cmd: Vec<String>,
    ) -> Result<TmuxPaneId, CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::spawn_window", move || {
            let args = cmd.iter().map(String::as_str).collect::<Vec<_>>();
            server.spawn_window_sync(&session, &window, &cwd, &args)
        })
        .await
    }

    pub async fn window_exists(&self, session: String, window: String) -> Result<bool, CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::window_exists", move || {
            server.window_exists_sync(&session, &window)
        })
        .await
    }

    pub async fn split_window(
        &self,
        target: String,
        cwd: PathBuf,
        cmd: Vec<String>,
    ) -> Result<TmuxPaneId, CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::split_window", move || {
            let args = cmd.iter().map(String::as_str).collect::<Vec<_>>();
            server.split_window_sync(&target, &cwd, &args)
        })
        .await
    }

    pub async fn split_window_with_spec(
        &self,
        target: String,
        cwd: PathBuf,
        cmd: Vec<String>,
        spec: SplitSpec,
    ) -> Result<TmuxPaneId, CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::split_window_with_spec", move || {
            let args = cmd.iter().map(String::as_str).collect::<Vec<_>>();
            server.split_window_with_spec_sync(&target, &cwd, &args, spec)
        })
        .await
    }

    pub async fn get_pane_pid(&self, pane: TmuxPaneId) -> Result<i32, CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::get_pane_pid", move || {
            server.get_pane_pid_sync(&pane)
        })
        .await
    }

    pub async fn pipe_pane_to_fifo(
        &self,
        pane: TmuxPaneId,
        fifo: PathBuf,
    ) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::pipe_pane_to_fifo", move || {
            server.pipe_pane_to_fifo_sync(&pane, &fifo)
        })
        .await
    }

    pub async fn send_keys_literal(&self, pane: TmuxPaneId, text: String) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::send_keys_literal", move || {
            server.send_keys_literal_sync(&pane, &text)
        })
        .await
    }

    pub async fn send_keys_keysym(
        &self,
        pane: TmuxPaneId,
        keysym: String,
    ) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::send_keys_keysym", move || {
            server.send_keys_keysym_sync(&pane, &keysym)
        })
        .await
    }

    pub async fn send_ctrl_c(&self, pane: TmuxPaneId) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::send_ctrl_c", move || server.send_ctrl_c_sync(&pane))
            .await
    }

    pub async fn kill_pane(&self, pane: TmuxPaneId) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::kill_pane", move || server.kill_pane_sync(&pane)).await
    }

    pub async fn kill_window(&self, session: String, window: String) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::kill_window", move || {
            server.kill_window_sync(&session, &window)
        })
        .await
    }

    pub async fn kill_session_window(&self, session_id: String) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::kill_session_window", move || {
            server.kill_session_window_sync(&session_id)
        })
        .await
    }

    pub async fn capture_pane(&self, pane: TmuxPaneId) -> Result<String, CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::capture_pane", move || {
            server.capture_pane_sync(&pane)
        })
        .await
    }

    pub async fn select_layout(
        &self,
        window_target: String,
        layout: String,
    ) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::select_layout", move || {
            server.select_layout_sync(&window_target, &layout)
        })
        .await
    }

    pub async fn list_panes(&self, window_target: String) -> Result<Vec<TmuxPaneId>, CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::list_panes", move || {
            server.list_panes_sync(&window_target)
        })
        .await
    }

    pub async fn load_buffer(&self, buffer_name: String, text: String) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::load_buffer", move || {
            server.load_buffer_sync(&buffer_name, &text)
        })
        .await
    }

    pub async fn paste_buffer(
        &self,
        pane: TmuxPaneId,
        buffer_name: String,
    ) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::paste_buffer", move || {
            server.paste_buffer_sync(&pane, &buffer_name)
        })
        .await
    }

    pub async fn delete_buffer(&self, buffer_name: String) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::delete_buffer", move || {
            server.delete_buffer_sync(&buffer_name)
        })
        .await
    }

    pub async fn send_enter(&self, pane: TmuxPaneId) -> Result<(), CcbdError> {
        let server = self.clone();
        crate::db::common::spawn_db("tmux::send_enter", move || server.send_enter_sync(&pane)).await
    }
}

fn build_split_window_args(
    socket_name: &str,
    target: &str,
    cwd_arg: &str,
    spec: &SplitSpec,
) -> Vec<String> {
    let mut args = vec![
        "-L".to_string(),
        socket_name.to_string(),
        "split-window".to_string(),
        "-d".to_string(),
    ];
    if let Some(direction) = &spec.direction {
        args.push(
            match direction {
                SplitDirection::Right => "-h",
                SplitDirection::Bottom => "-v",
            }
            .to_string(),
        );
    }
    if let Some(percent) = spec.percent {
        args.push("-p".to_string());
        args.push(percent.to_string());
    }
    args.push("-t".to_string());
    args.push(
        spec.parent
            .as_ref()
            .map(|pane| pane.0.clone())
            .unwrap_or_else(|| target.to_string()),
    );
    args.push("-c".to_string());
    args.push(cwd_arg.to_string());
    args.push("-P".to_string());
    args.push("-F".to_string());
    args.push("#{pane_id}".to_string());
    args.push("--".to_string());
    args
}

fn reusable_initial_window(windows: &[String]) -> Option<&str> {
    let [window] = windows else {
        return None;
    };
    if matches!(window.as_str(), "0" | "bash" | "sh" | "zsh" | "fish") {
        Some(window)
    } else {
        None
    }
}

fn map_command_io_error(err: std::io::Error) -> CcbdError {
    CcbdError::from(TmuxError::Io(err))
}

fn ensure_success(program: &str, args: &[&str], output: Output) -> Result<(), CcbdError> {
    ensure_output_success(program, args, &[], output).map(|_| ())
}

fn ensure_output_success(
    program: &str,
    args: &[&str],
    extra_args: &[&str],
    output: Output,
) -> Result<String, CcbdError> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let mut parts = Vec::with_capacity(args.len() + extra_args.len() + 1);
    parts.push(program.to_string());
    parts.extend(args.iter().map(|arg| (*arg).to_string()));
    parts.extend(extra_args.iter().map(|arg| (*arg).to_string()));
    let cmd = parts.join(" ");
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let exit = output.status.code().unwrap_or(-1);
    tracing::debug!(cmd = %cmd, stderr = %stderr, exit = exit, "tmux command failed");
    Err(CcbdError::from(TmuxError::CommandFailed {
        cmd,
        stderr,
        exit,
    }))
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
pub(crate) fn parse_pane_pid_for_test(value: &str) -> Result<i32, CcbdError> {
    value
        .trim()
        .parse::<i32>()
        .map_err(|_| CcbdError::from(TmuxError::ParsePid(value.trim().to_string())))
}

#[cfg(test)]
mod split_tests {
    use super::{SplitDirection, SplitSpec, build_split_window_args, reusable_initial_window};
    use crate::tmux::TmuxPaneId;

    #[test]
    fn test_build_split_window_args_adds_direction_percent_and_parent() {
        let args = build_split_window_args(
            "sock",
            "ccbd-agents:s1",
            "/tmp/work",
            &SplitSpec {
                parent: Some(TmuxPaneId("%1".into())),
                direction: Some(SplitDirection::Right),
                percent: Some(50),
            },
        );

        assert!(args.windows(2).any(|window| window == ["-t", "%1"]));
        assert!(args.contains(&"-h".to_string()));
        assert!(args.windows(2).any(|window| window == ["-p", "50"]));
        assert!(args.windows(2).any(|window| window == ["-c", "/tmp/work"]));
    }

    #[test]
    fn test_build_split_window_args_bottom_direction() {
        let args = build_split_window_args(
            "sock",
            "ccbd-agents:s1",
            "/tmp/work",
            &SplitSpec {
                parent: None,
                direction: Some(SplitDirection::Bottom),
                percent: Some(40),
            },
        );

        assert!(args.contains(&"-v".to_string()));
        assert!(
            args.windows(2)
                .any(|window| window == ["-t", "ccbd-agents:s1"])
        );
        assert!(args.windows(2).any(|window| window == ["-p", "40"]));
    }

    #[test]
    fn test_reusable_initial_window_accepts_only_default_single_window() {
        assert_eq!(reusable_initial_window(&["bash".to_string()]), Some("bash"));
        assert_eq!(reusable_initial_window(&["0".to_string()]), Some("0"));
        assert_eq!(reusable_initial_window(&["project".to_string()]), None);
        assert_eq!(
            reusable_initial_window(&["bash".to_string(), "other".to_string()]),
            None
        );
    }
}
