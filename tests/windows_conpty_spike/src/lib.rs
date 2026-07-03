#[cfg(not(target_os = "windows"))]
#[test]
fn windows_conpty_spike_is_windows_only() {
    eprintln!("windows_conpty_spike runs only on Windows CI");
}

#[cfg(target_os = "windows")]
mod windows {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::sync::mpsc::{self, Receiver};
    use std::thread;
    use std::time::{Duration, Instant};

    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    const COLS: usize = 100;
    const ROWS: usize = 30;
    const TIMEOUT: Duration = Duration::from_secs(15);

    struct SpikeTermSize {
        columns: usize,
        screen_lines: usize,
    }

    impl Dimensions for SpikeTermSize {
        fn total_lines(&self) -> usize {
            self.screen_lines
        }

        fn screen_lines(&self) -> usize {
            self.screen_lines
        }

        fn columns(&self) -> usize {
            self.columns
        }
    }

    #[test]
    fn conpty_input_drives_shell_and_alacritty_grid_captures_output() {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: ROWS as u16,
                cols: COLS as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open Windows ConPTY");

        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.arg("/Q");
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .expect("spawn cmd.exe in ConPTY");
        drop(pair.slave);

        let mut writer = pair.master.take_writer().expect("take PTY writer");
        let rx = spawn_reader(pair.master.try_clone_reader().expect("clone PTY reader"));
        let mut term = Term::new(
            Config::default(),
            &SpikeTermSize {
                columns: COLS,
                screen_lines: ROWS,
            },
            VoidListener,
        );
        let mut parser = Processor::new();

        writer
            .write_all(b"echo AH_CONPTY_READY\r\n")
            .expect("write READY command to ConPTY");
        writer.flush().expect("flush READY command");
        let ready_capture = wait_for_grid_text(&rx, &mut term, &mut parser, "AH_CONPTY_READY");
        assert!(
            ready_capture.contains("AH_CONPTY_READY"),
            "alacritty grid never captured READY marker; final grid:\n{ready_capture}"
        );

        writer
            .write_all(b"set /a 40+2\r\n")
            .expect("write arithmetic command to ConPTY");
        writer.flush().expect("flush arithmetic command");
        let computed_capture = wait_for_grid_text(&rx, &mut term, &mut parser, "42");
        assert!(
            computed_capture.lines().any(|line| line.trim() == "42"),
            "shell did not produce computed output 42; final grid:\n{computed_capture}"
        );

        let _ = writer.write_all(b"exit\r\n");
        let _ = writer.flush();
        let _ = child.kill();
    }

    fn spawn_reader(mut reader: Box<dyn Read + Send>) -> Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0_u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        rx
    }

    fn wait_for_grid_text(
        rx: &Receiver<Vec<u8>>,
        term: &mut Term<VoidListener>,
        parser: &mut Processor,
        needle: &str,
    ) -> String {
        let deadline = Instant::now() + TIMEOUT;
        let mut capture = serialize_grid(term);

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(bytes) => {
                    parser.advance(term, &bytes);
                    capture = serialize_grid(term);
                    if capture.contains(needle) {
                        return capture;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    capture = serialize_grid(term);
                    if capture.contains(needle) {
                        return capture;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        capture
    }

    fn serialize_grid(term: &Term<VoidListener>) -> String {
        let mut lines = BTreeMap::<i32, String>::new();
        for indexed in term.grid().display_iter() {
            lines
                .entry(indexed.point.line.0)
                .or_default()
                .push(indexed.cell.c);
        }

        lines
            .into_values()
            .map(|line| line.trim_end().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
