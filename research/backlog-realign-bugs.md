# Realign semantics bugs — independent backlog (2026-07-10)

Surfaced by a live daemon `REALIGN` restart during the Gen-2 queue-jam recovery
(2026-07-10 night, operator-witnessed). Both are real, machine-fixable bugs in
realign's *behavior*, not its code location — explicitly **out of scope for
`ah-control-plane-refactor`'s D4** (`spawn_realign_agent` relocation), which is
a deliberate pure-move with no semantics changes (see that spec's design.md D4
section). Tracked here so they don't fall through the gap between "D4 doesn't
touch semantics" and "nobody owns semantics."

1. **Non-atomic session swap can drop an agent.** The realign restart path
   deletes an agent's old tmux session/registration before the replacement is
   confirmed created. When the create step for one agent (`g2`) failed or was
   skipped mid-sequence during tonight's restart, the agent was dropped
   entirely — it did not reappear in `ah ps` after the first `ah up`, and a
   second `ah up` was needed to repair it manually. Fix direction: make the
   swap atomic — create-new-then-delete-old (so a failure mid-sequence leaves
   the old session intact rather than the agent gone), or wrap both steps in a
   single transactional operation with rollback on partial failure.

2. **Respawn session-name misattribution.** After the same restart, the agent
   `g2`'s tmux pane was created under the tmux session name `agent_g2-m1`
   (colliding with/borrowing a different agent's session-naming slot) instead
   of `agent_g2`. Session-name-based addressing became unreliable as a result
   — pane-ID addressing (`%9`, `%11`, etc.) had to be used as a workaround for
   the remainder of the session to reliably target the right agent. Fix
   direction: ensure respawn session-naming is derived from the identity of
   the agent actually being (re)created, not from stale or adjacent slot
   state left over from the previous topology. Add a regression test
   asserting `session_name == expected_name_for(agent_id)` holds for every
   agent immediately after a realign restart, not just at initial `ah up`.

Both bugs are real, both are independent of one another (either could be fixed
without the other), and neither requires the D4 relocation to land first —
they can be scheduled as standalone machine-fix tickets whenever picked up.
