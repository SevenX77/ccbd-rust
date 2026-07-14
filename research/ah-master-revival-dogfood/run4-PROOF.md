# Run-4 dogfood: master OOM revive preserves worker session context (PASS)

Date: 2026-06-18. Binary: target/release/{ah,ahd} built from commit bec9770
(fix: defer anchor cascade while session ACTIVE). Isolated env AH_STATE_DIR=/tmp/ah-mtd2/state,
tmux -L ahd-6b34f3896f701bcc. Worker w1 provider=claude (recovery-eligible).

## Sequence
- 10:07:31 turn-1 (sync): planted codeword BANANA-42 -> w1 replied "noted". Session sess_1e9cbaf5.
- 10:07:31 turn-2 (async): w1 recalled BANANA-42 (single-session baseline).
- 10:09:01 turn-3 (long, async): submitted to keep w1 BUSY.
- 10:09:03 w1 BUSY confirmed -> kill -9 master (OOM injection, ActiveWork).

## Revive (ahd.log, run4-revive-proof-ahd.log)
- 10:09:03.215 captured recovery intent w1 previous_state=BUSY action=Revive interrupted_job=job_0551c518 (preserve-aware path)
- 10:09:03.240 master death worker cleanup classification=ActiveWork workers=1
- 10:09:03.299 continue instruction injected into revived master pane
- 10:09:03.395 interrupted turn-3 requeued after worker reprovision (requeued=1)
- master.log: 10:09:03 REVIVED master (turn1.done present) -> idle only, pid=3652813

## Home survival (the bug that was fixed)
Worker home /home/sevenx/.cache/ah/sandboxes/a133919fc346 EXISTS continuously
t+2s..t+32s -- PAST the ~7s anchor-inactive window where the OLD cross-monitor
race (session_watch ANCHOR_UNIT_STOPPED cascade) wiped it. Same transcript
b94f7908....jsonl grew 19520 -> 24905 bytes = reprovisioned --continue worker
resumed the SAME session and finished the requeued turn-3.

## Semantic proof (session continuity)
- requeued turn-3 answer: BANANA-42 (5 mentions in w1 logs)
- turn-4 (fresh query on revived worker): "BANANA-42" -> PASS

Conclusion: killed master -> ahd revived it -> worker reprovisioned --continue into
PRESERVED home -> worker retained prior-turn context -> same task continued.
The fix (commit bec9770) closes the home-wipe regression proven by run2-ahd.log.
