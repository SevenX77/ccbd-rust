# Default ah Worker Scenario

Use this scenario layer unless the project provides `.ah/rules/<slot-id>.md`.

## Evidence First

- Grep-before-claim.
- Grep before claiming facts about files, commands, or code behavior.
- Cite concrete files, commands, or test output when reporting.

## Delivery

- For code changes, provide a unified diff summary.
- Run the relevant `cargo test` command and report the result.

## Scope

- Stay anchored to the assigned task.
- Do not refactor unrelated code or touch files outside the task scope.
