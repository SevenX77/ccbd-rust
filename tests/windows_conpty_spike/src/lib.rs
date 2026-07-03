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
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

    const COLS: usize = 80;
    const ROWS: usize = 24;
    const TIMEOUT: Duration = Duration::from_secs(15);

    struct CaptureSnapshot {
        raw_len: usize,
        raw_hex: String,
        raw_text: String,
        grid_filtered: String,
        grid_unfiltered: String,
        child_status: String,
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
    fn official_whoami_example_captures_output() {
        let (output, child_status) =
            run_one_shot_read_to_string("official whoami", CommandBuilder::new("whoami"));
        dump_raw("official whoami output", output.as_bytes());
        eprintln!("official whoami child status: {child_status}");

        assert!(
            !output.as_bytes().is_empty(),
            "official whoami example produced no output; child status: {child_status}"
        );
    }

    #[test]
    fn conpty_noninteractive_cmd_c_output_reaches_raw_and_grid() {
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/C", "echo AH_CONPTY_READY& set /a 40+2"]);
        let (output, child_status) = run_one_shot_read_to_string("cmd /C", cmd);

        let mut term = new_term();
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, output.as_bytes());
        let snapshot = capture_snapshot_with_status(&term, output.as_bytes(), child_status);

        assert_raw_contains("/C READY marker", &snapshot, "AH_CONPTY_READY");
        assert_grid_contains("/C READY marker", &snapshot, "AH_CONPTY_READY");
        assert_raw_contains("/C computed output", &snapshot, "42");
        assert_grid_line("/C computed output", &snapshot, "42");
    }

    fn run_one_shot_read_to_string(label: &str, cmd: CommandBuilder) -> (String, String) {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: ROWS as u16,
                cols: COLS as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap_or_else(|err| panic!("open Windows ConPTY for {label}: {err}"));

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .unwrap_or_else(|err| panic!("spawn {label} in ConPTY: {err}"));

        drop(pair.slave);

        let (tx, rx) = mpsc::channel();
        let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
        thread::spawn(move || {
            let mut output = String::new();
            let read_result = reader.read_to_string(&mut output);
            tx.send((read_result, output))
                .expect("send one-shot output");
        });

        {
            let mut writer = pair.master.take_writer().expect("take PTY writer");
            let to_write = "";
            if !to_write.is_empty() {
                thread::spawn(move || {
                    writer.write_all(to_write.as_bytes()).unwrap();
                });
            }
        }

        let child_status = format!("{:?}", child.wait().expect("wait for one-shot child"));
        eprintln!("{label} child status: {child_status}");
        drop(pair.master);

        let (read_result, output) = rx.recv().expect("receive one-shot output");
        eprintln!("{label} read_to_string result: {read_result:?}");
        dump_raw(&format!("{label} output"), output.as_bytes());
        (output, child_status)
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
        cmd.args(["/D", "/Q", "/K"]);
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .expect("spawn cmd.exe in ConPTY");

        #[cfg(not(windows))]
        drop(pair.slave);
        #[cfg(windows)]
        let _slave_keepalive = pair.slave;

        eprintln!(
            "interactive child status after spawn: {}",
            describe_child_status(child.as_mut(), "after spawn")
        );

        let reader = pair.master.try_clone_reader().expect("clone PTY reader");
        let mut writer = pair.master.take_writer().expect("take PTY writer");
        let rx = spawn_reader_loop(reader);
        let mut term = new_term();
        let mut parser = Processor::new();
        let mut raw_bytes = Vec::new();

        write_all_logged(
            &mut writer,
            b"echo AH_CONPTY_READY\r\n",
            "interactive READY command",
        );
        let ready_capture = wait_for_raw_text(
            &rx,
            &mut term,
            &mut parser,
            &mut raw_bytes,
            child.as_mut(),
            "AH_CONPTY_READY",
        );
        assert_raw_contains(
            "interactive READY marker",
            &ready_capture,
            "AH_CONPTY_READY",
        );
        assert_grid_contains(
            "interactive READY marker",
            &ready_capture,
            "AH_CONPTY_READY",
        );

        write_all_logged(
            &mut writer,
            b"set /a 40+2\r\n",
            "interactive arithmetic command",
        );
        let computed_capture = wait_for_raw_text(
            &rx,
            &mut term,
            &mut parser,
            &mut raw_bytes,
            child.as_mut(),
            "42",
        );
        assert_raw_contains("interactive computed output", &computed_capture, "42");
        assert_grid_line("interactive computed output", &computed_capture, "42");

        let _ = writer.write_all(b"exit\r\n");
        let _ = writer.flush();
        let _ = child.kill();
    }

    fn new_term() -> Term<VoidListener> {
        Term::new(
            Config::default(),
            &SpikeTermSize {
                columns: COLS,
                screen_lines: ROWS,
            },
            VoidListener,
        )
    }

    fn write_all_logged(writer: &mut Box<dyn Write + Send>, bytes: &[u8], label: &str) {
        writer.write_all(bytes).expect(label);
        writer.flush().expect(label);
        eprintln!("write_all succeeded for {label}: bytes={}", bytes.len());
    }

    fn spawn_reader_loop(mut reader: Box<dyn Read + Send>) -> Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0_u8; 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        eprintln!(
                            "interactive read chunk bytes={n}, hex={}",
                            hex_dump(&buf[..n])
                        );
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        eprintln!("Error reading from PTY: {err}");
                        break;
                    }
                }
            }
        });
        rx
    }

    fn wait_for_raw_text(
        rx: &Receiver<Vec<u8>>,
        term: &mut Term<VoidListener>,
        parser: &mut Processor,
        raw_bytes: &mut Vec<u8>,
        child: &mut (dyn Child + Send + Sync),
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
                        return capture_snapshot(term, raw_bytes, child);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if String::from_utf8_lossy(raw_bytes).contains(needle) {
                        return capture_snapshot(term, raw_bytes, child);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        eprintln!(
            "wait_for_raw_text timeout for {needle:?}; child final status: {}",
            describe_child_status(child, "timeout")
        );
        capture_snapshot(term, raw_bytes, child)
    }

    fn capture_snapshot(
        term: &Term<VoidListener>,
        raw_bytes: &[u8],
        child: &mut (dyn Child + Send + Sync),
    ) -> CaptureSnapshot {
        capture_snapshot_with_status(
            term,
            raw_bytes,
            describe_child_status(child, "capture snapshot"),
        )
    }

    fn capture_snapshot_with_status(
        term: &Term<VoidListener>,
        raw_bytes: &[u8],
        child_status: String,
    ) -> CaptureSnapshot {
        let (raw_len, raw_hex) = raw_count_hex(raw_bytes);
        CaptureSnapshot {
            raw_len,
            raw_hex,
            raw_text: String::from_utf8_lossy(raw_bytes).into_owned(),
            grid_filtered: serialize_grid_filtered(term),
            grid_unfiltered: serialize_grid_unfiltered(term),
            child_status,
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
        eprintln!("raw hex:\n{}", capture.raw_hex);
        eprintln!("raw lossy UTF-8:\n{}", capture.raw_text);
        eprintln!("child status: {}", capture.child_status);
        eprintln!("grid dimensions: cols={COLS}, rows={ROWS}");
        eprintln!(
            "grid unfiltered with row boundaries:\n{}",
            capture.grid_unfiltered
        );
        eprintln!("grid filtered:\n{}", capture.grid_filtered);
        eprintln!("--- end diagnostics ---");
    }

    fn dump_raw(label: &str, raw_bytes: &[u8]) {
        let (raw_len, raw_hex) = raw_count_hex(raw_bytes);
        eprintln!("{label} byte count: {raw_len}");
        eprintln!("{label} hex:\n{raw_hex}");
        eprintln!(
            "{label} lossy UTF-8:\n{}",
            String::from_utf8_lossy(raw_bytes)
        );
    }

    fn describe_child_status(child: &mut (dyn Child + Send + Sync), context: &str) -> String {
        match child.try_wait() {
            Ok(Some(status)) => format!("{context}: exited with {status:?}"),
            Ok(None) => format!("{context}: still running"),
            Err(err) => format!("{context}: try_wait error: {err}"),
        }
    }

    fn raw_count_hex(bytes: &[u8]) -> (usize, String) {
        (bytes.len(), hex_dump(bytes))
    }

    fn hex_dump(bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return "<empty>".to_string();
        }

        bytes
            .chunks(16)
            .enumerate()
            .map(|(row, chunk)| {
                let offset = row * 16;
                let hex = chunk
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{offset:04X}: {hex}")
            })
            .collect::<Vec<_>>()
            .join("\n")
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
