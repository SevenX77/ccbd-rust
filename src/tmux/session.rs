use crate::error::CcbdError;
use crate::tmux::{TmuxError, TmuxPaneId, compute_socket_name};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[derive(Clone, Debug)]
pub struct TmuxServer {
    socket_name: String,
}

impl TmuxServer {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            socket_name: compute_socket_name(state_dir),
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
        let output = Command::new("tmux")
            .args(args)
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
    Err(CcbdError::from(TmuxError::CommandFailed {
        cmd: parts.join(" "),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        exit: output.status.code().unwrap_or(-1),
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
