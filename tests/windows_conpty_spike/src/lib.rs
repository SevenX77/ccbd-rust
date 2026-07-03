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
    const SETTLE_TIMEOUT: Duration = Duration::from_secs(3);

    struct CaptureSnapshot {
        raw_len: usize,
        raw_text: String,
        grid_filtered: String,
        grid_unfiltered: String,
    }

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
        let mut raw_bytes = Vec::new();

        if !wait_for_any_output(&rx, &mut term, &mut parser, &mut raw_bytes, SETTLE_TIMEOUT) {
            eprintln!(
                "cmd.exe produced no initial output within {:?}; sending blank line to settle",
                SETTLE_TIMEOUT
            );
            writer
                .write_all(b"\r\n")
                .expect("write settle newline to ConPTY");
            writer.flush().expect("flush settle newline");
            let _ =
                wait_for_any_output(&rx, &mut term, &mut parser, &mut raw_bytes, SETTLE_TIMEOUT);
        }
        eprintln!("initial ConPTY bytes before commands: {}", raw_bytes.len());

        writer
            .write_all(b"echo AH_CONPTY_READY\r\n")
            .expect("write READY command to ConPTY");
        writer.flush().expect("flush READY command");
        let ready_capture = wait_for_raw_text(
            &rx,
            &mut term,
            &mut parser,
            &mut raw_bytes,
            "AH_CONPTY_READY",
        );
        assert_raw_contains("READY marker", &ready_capture, "AH_CONPTY_READY");
        assert_grid_contains("READY marker", &ready_capture, "AH_CONPTY_READY");

        writer
            .write_all(b"set /a 40+2\r\n")
            .expect("write arithmetic command to ConPTY");
        writer.flush().expect("flush arithmetic command");
        let computed_capture = wait_for_raw_text(&rx, &mut term, &mut parser, &mut raw_bytes, "42");
        assert_raw_contains("computed output", &computed_capture, "42");
        assert_grid_line("computed output", &computed_capture, "42");

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

    fn wait_for_any_output(
        rx: &Receiver<Vec<u8>>,
        term: &mut Term<VoidListener>,
        parser: &mut Processor,
        raw_bytes: &mut Vec<u8>,
        timeout: Duration,
    ) -> bool {
        let starting_len = raw_bytes.len();
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(bytes) => {
                    raw_bytes.extend_from_slice(&bytes);
                    parser.advance(term, &bytes);
                    if raw_bytes.len() > starting_len {
                        return true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        false
    }

    fn wait_for_raw_text(
        rx: &Receiver<Vec<u8>>,
        term: &mut Term<VoidListener>,
        parser: &mut Processor,
        raw_bytes: &mut Vec<u8>,
        needle: &str,
    ) -> CaptureSnapshot {
        let deadline = Instant::now() + TIMEOUT;

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(bytes) => {
                    raw_bytes.extend_from_slice(&bytes);
                    parser.advance(term, &bytes);
                    if String::from_utf8_lossy(raw_bytes).contains(needle) {
                        return capture_snapshot(term, raw_bytes);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if String::from_utf8_lossy(raw_bytes).contains(needle) {
                        return capture_snapshot(term, raw_bytes);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        capture_snapshot(term, raw_bytes)
    }

    fn capture_snapshot(term: &Term<VoidListener>, raw_bytes: &[u8]) -> CaptureSnapshot {
        CaptureSnapshot {
            raw_len: raw_bytes.len(),
            raw_text: String::from_utf8_lossy(raw_bytes).into_owned(),
            grid_filtered: serialize_grid_filtered(term),
            grid_unfiltered: serialize_grid_unfiltered(term),
        }
    }

    fn assert_raw_contains(label: &str, capture: &CaptureSnapshot, needle: &str) {
        if !capture.raw_text.contains(needle) {
            dump_diagnostics(label, capture);
            panic!("ConPTY raw output never contained {needle:?} for {label}");
        }
    }

    fn assert_grid_contains(label: &str, capture: &CaptureSnapshot, needle: &str) {
        if !capture.grid_filtered.contains(needle) && !capture.grid_unfiltered.contains(needle) {
            dump_diagnostics(label, capture);
            panic!("alacritty grid never captured {needle:?} for {label}");
        }
    }

    fn assert_grid_line(label: &str, capture: &CaptureSnapshot, expected: &str) {
        if !capture
            .grid_filtered
            .lines()
            .any(|line| line.trim() == expected)
        {
            dump_diagnostics(label, capture);
            panic!("alacritty grid never captured line {expected:?} for {label}");
        }
    }

    fn dump_diagnostics(label: &str, capture: &CaptureSnapshot) {
        eprintln!("--- windows_conpty_spike diagnostics: {label} ---");
        eprintln!("raw byte count: {}", capture.raw_len);
        eprintln!("raw lossy UTF-8:\n{}", capture.raw_text);
        eprintln!("grid dimensions: cols={COLS}, rows={ROWS}");
        eprintln!(
            "grid unfiltered with row boundaries:\n{}",
            capture.grid_unfiltered
        );
        eprintln!("grid filtered:\n{}", capture.grid_filtered);
        eprintln!("--- end diagnostics ---");
    }

    fn serialize_grid_filtered(term: &Term<VoidListener>) -> String {
        grid_lines(term)
            .into_values()
            .map(|line| line.trim_end().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn serialize_grid_unfiltered(term: &Term<VoidListener>) -> String {
        grid_lines(term)
            .into_iter()
            .map(|(line, content)| format!("{line:03}|{content}|"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn grid_lines(term: &Term<VoidListener>) -> BTreeMap<i32, String> {
        let mut lines = BTreeMap::<i32, String>::new();
        for indexed in term.grid().display_iter() {
            lines
                .entry(indexed.point.line.0)
                .or_default()
                .push(indexed.cell.c);
        }
        lines
    }
}
