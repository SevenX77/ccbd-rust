# portable-pty 0.9 Windows ConPTY Output Capture Reference

Scope: research-only note for debugging a Windows ConPTY spike where `cmd.exe`
runs and accepts input, but the portable-pty master reader receives only a few
bytes. No project code was changed.

## Executive Finding

The closest known-good reference is the portable-pty 0.9.0 official examples:

- `examples/whoami.rs`: one-shot command capture.
- `examples/bash.rs`: long-lived shell with a continuous read loop.

For a long-lived `cmd.exe` spike, copy the `bash.rs` style, not the `whoami.rs`
`read_to_string` style. The critical pattern is:

1. `NativePtySystem::default()`.
2. `openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })`.
3. `pair.slave.spawn_command(cmd)`.
4. Start a reader from `pair.master.try_clone_reader()`.
5. Keep `pair.master.take_writer()` alive while the shell is alive.
6. Read with repeated `reader.read(&mut buf)`, not `read_to_string()`.
7. Stop by timeout/sentinel text/process state, not EOF on Windows.

Do not use EOF as the capture completion condition on Windows ConPTY. There are
reports that ConPTY output pipes may not close even after child stdout/stderr or
the child process exits.

## Known-Good / Official Skeletons

### Official one-shot example: `whoami.rs`

Source:
https://docs.rs/crate/portable-pty/latest/source/examples/whoami.rs

Important sequence in the official example:

```rust
let pty_system = NativePtySystem::default();
let pair = pty_system
    .openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })
    .unwrap();

let cmd = CommandBuilder::new("whoami");
let mut child = pair.slave.spawn_command(cmd).unwrap();

drop(pair.slave);

let (tx, rx) = channel();
let mut reader = pair.master.try_clone_reader().unwrap();
std::thread::spawn(move || {
    let mut s = String::new();
    reader.read_to_string(&mut s).unwrap();
    tx.send(s).unwrap();
});

{
    let mut writer = pair.master.take_writer().unwrap();
    let to_write = "";
    if !to_write.is_empty() {
        std::thread::spawn(move || {
            writer.write_all(to_write.as_bytes()).unwrap();
        });
    }
}

println!("child status: {:?}", child.wait().unwrap());
drop(pair.master);
let output = rx.recv().unwrap();
```

Notes:

- Size is non-zero: `24x80`. `pixel_width` and `pixel_height` are `0`.
- Reader is taken after `spawn_command`.
- The official one-shot example drops `pair.slave`.
- It uses `read_to_string`, but this is a poor model for long-lived Windows
  ConPTY shells because it waits for EOF.
- It waits for the child, then drops the master, then receives output.

### Official long-lived shell example: `bash.rs`

Source:
https://docs.rs/crate/portable-pty/latest/source/examples/bash.rs

Important sequence:

```rust
let pty_system = NativePtySystem::default();
let pair = pty_system
    .openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })
    .unwrap();

let cmd = CommandBuilder::new("bash");
let mut child = pair.slave.spawn_command(cmd).unwrap();

drop(pair.slave);

let (tx, rx) = channel::<String>();
let mut reader = pair.master.try_clone_reader().unwrap();
let master_writer = pair.master.take_writer().unwrap();

thread::spawn(move || {
    let mut buffer = [0u8; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let output = String::from_utf8_lossy(&buffer[..n]);
                println!("{}", output);
            }
            Err(e) => {
                eprintln!("Error reading from PTY: {}", e);
                break;
            }
        }
    }
});

let tx_writer = thread::spawn(move || {
    handle_input_stream(rx, master_writer);
});
```

Notes:

- This is the reference shape to copy for a persistent `cmd.exe` shell.
- The read loop consumes raw byte chunks with `read(&mut buffer)`.
- It does not use EOF to decide command output is complete.
- The writer is held in a separate path and is not dropped immediately.

## Windows Backend Facts

Source:
https://github.com/wezterm/wezterm/blob/main/pty/src/win/conpty.rs

Relevant facts from the Windows portable-pty backend:

- `openpty` creates two pipes, then calls `PsuedoCon::new`.
- `PtySize.cols` and `PtySize.rows` are passed directly into a Win32 `COORD`.
- The master readable side is `stdout.read`.
- The master writable side is `stdin.write`.
- `try_clone_reader()` clones the readable file descriptor.
- `take_writer()` consumes the stored writable descriptor.
- `spawn_command()` calls into the pseudo console object using the slave.

Implication:

- `rows` and `cols` must be non-zero. A `0x0` pseudo console is not a valid
  capture baseline.
- `try_clone_reader()` timing is not magic in the implementation; it clones an
  already-created readable pipe. The official examples take it after spawn.
- If input works but output is only a few bytes, the suspicious area is the
  ConPTY output pipe consumption/lifetime or how completion is detected, not
  the `CommandBuilder` spawn path.

## Reports Relevant to "No Output" / "Few Bytes"

### Windows EOF/read completion is unreliable

Source:
https://users.rust-lang.org/t/rust-pty-output-hangs-when-trying-to-read-command-output-in-terminal-emulator/102873

Key conclusion from the discussion:

- On Windows, do not use `read_to_string()` or any read operation that waits
  for end-of-stream.
- Use ordinary `read()` in another thread or async runtime.
- The explanation given is that ConPTY handle and IO handle lifecycles are not
  tightly coupled; the output pipe may stay open even when child stdout/stderr
  closes or all child processes exit.

This is directly relevant: a reader can appear "stuck" or incomplete even when
the child did emit output.

### Official `whoami` example has Windows failure reports

Source:
https://github.com/wezterm/wezterm/issues/1396

Reported sequence:

- `openpty(24x80)`.
- `spawn_command("whoami")`.
- `drop(pair.slave)`.
- `try_clone_reader()`.
- `drop(pair.master)`.
- `read_to_string()`.

Reported result:

- No output was read and the child exited unsuccessfully.
- The reporter observed that waiting for the child before reading changed the
  behavior, but noted that this is not viable for long-running programs.

Takeaway:

- Do not use this exact one-shot read-to-EOF shape for `cmd.exe /K` or any
  persistent shell.

### Dropping `pair.slave` on Windows has conflicting reports

Source:
https://github.com/wezterm/wezterm/issues/4206

Reported workaround:

- On Windows, the reporter kept `pair.slave` alive and only dropped it on
  non-Windows platforms.
- The issue says writing to the PTY failed if `pair.slave` was dropped.

Takeaway:

- This conflicts with official examples that unconditionally drop `pair.slave`.
- Since the current spike already ruled out slave-drop and input delivery, this
  is probably not the active failure, but a final reference implementation can
  keep the slave alive on Windows until after the first successful output read.

### Recent Windows example failure report

Source:
https://github.com/wezterm/wezterm/issues/7025

Observed report:

- The reporter copied the official `whoami` example on Windows.
- It reached "child spawned" and "slave dropped", then terminated with
  `STATUS_CONTROL_C_EXIT`.
- The issue is still open/triage in the fetched page.

Takeaway:

- There are environment-specific Windows failures with the official one-shot
  example. Treat "official example shape" as the API reference, not proof that
  every Windows host will behave.

## Comparison to Current Spike

Current spike path as reported:

```text
openpty(100x30)
-> slave.spawn_command
-> if !windows drop slave
-> master.try_clone_reader()
-> background thread loop reader.read(&mut buf[4096]) -> channel
-> main thread recv_timeout accumulates
```

What matches the known-good long-lived shell shape:

- Non-zero size.
- Reader is taken after spawn.
- Reader uses `read(&mut buf)`, not `read_to_string`.
- Reader runs on a background thread.
- Slave is retained on Windows, which matches the #4206 workaround.

Concrete differences worth testing:

1. Official examples use `24x80`; current spike uses `30x100`.
   - This should be valid, but test `24x80` exactly to remove one variable.

2. Official `bash.rs` takes and retains `master_writer` immediately after
   `try_clone_reader`.
   - If the spike writes through a short-lived writer or repeatedly calls
     `take_writer`, it may differ from the long-lived shell reference. In
     portable-pty, `take_writer()` consumes the one writer; keep that writer for
     the whole shell.

3. Official shell example is interactive and does not wait for EOF.
   - The spike's `recv_timeout` accumulation is acceptable only if it stops on
     sentinel text (`READY`, prompt text, command output), not on EOF.

4. Official examples print/handle every non-zero read chunk immediately.
   - For debugging "only 4 bytes", dump every chunk as both hex and lossy UTF-8.
     The first few bytes may be an escape/control sequence or partial UTF-8.

5. `cmd.exe` startup may not print a banner in the same way a Unix shell does.
   - The reliable test should explicitly write `echo READY\r\n` and wait for
     `READY`, not rely on the initial prompt/banner.

Most likely cause if input is proven delivered and child output is known to
exist: the reader is stopping or judging success too early, or reading only an
initial control-sequence chunk while subsequent reads are blocked behind a
writer/lifetime/ConPTY environment issue. The current high-level chain is
already close to the reference; the highest-value rewrite is to copy the
`bash.rs` long-lived writer + reader structure exactly and use sentinel-based
completion.

## Minimal Skeleton to Copy for `cmd.exe`

This uses portable-pty 0.9 APIs shown by the official examples. It is designed
to capture a sentinel from a persistent Windows `cmd.exe`.

```rust
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.args(["/D", "/Q", "/K"]);
    let mut child = pair.slave.spawn_command(cmd)?;

    // Official examples drop the slave here.  There are Windows reports where
    // keeping it alive avoids write failures, so keep it alive until after the
    // first successful capture when debugging.
    #[cfg(not(windows))]
    drop(pair.slave);
    #[cfg(windows)]
    let _slave_keepalive = pair.slave;

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = tx.send(buf[..n].to_vec());
                }
                Err(err) => {
                    eprintln!("pty read error: {err}");
                    break;
                }
            }
        }
    });

    // Keep this writer alive for the lifetime of cmd.exe.
    writer.write_all(b"echo READY\r\n")?;
    writer.flush()?;

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut captured = Vec::<u8>::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(chunk) => {
                eprintln!(
                    "chunk len={} hex={:02x?} text={:?}",
                    chunk.len(),
                    chunk,
                    String::from_utf8_lossy(&chunk)
                );
                captured.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&captured).contains("READY") {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    println!("captured:\n{}", String::from_utf8_lossy(&captured));

    let _ = writer.write_all(b"exit\r\n");
    let _ = writer.flush();
    let _ = child.wait();
    drop(pair.master);
    Ok(())
}
```

## Recommended a1 Rewrite Checklist

1. Use exact `24x80` first.
2. Spawn `cmd.exe /D /Q /K`.
3. Start reader thread immediately after spawn.
4. Take writer once and keep it alive.
5. Write `echo READY\r\n`, not just `\n`.
6. Stop on seeing `READY`, not EOF.
7. Hex dump every chunk.
8. Try both Windows slave keepalive and official drop after the first baseline.
9. Do not call `drop(pair.master)` until after capture or cleanup.
10. If still only 4 bytes, rerun with the official `whoami.rs` and `bash.rs`
    examples unmodified on the same host to separate project bug from host /
    portable-pty / ConPTY behavior.

