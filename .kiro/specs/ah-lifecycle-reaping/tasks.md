# AH Lifecycle Reaping Tasks

## T1 - Tests First: RED Coverage For Dynamic Daemon Unit Binding

**Depends on:** design.md accepted

**Goal:** 先写失败测试，锁住 PR3 的动态检测与三类 scope 注入契约。T1 完成后测试应为 RED，失败原因应是现有实现仍硬编码 `ccbd.service`、缺少 `PartOf=`、缺少 master/tmux 同步路径，或缺少新函数签名。

**Files:**

- `src/tmux/scope.rs`
- `src/sandbox/systemd.rs`
- `tests/r1_bindsto_alignment.rs`

**RED tests:**

1. `detect_current_service_unit_from_cgroup` 纯函数测试，使用真实 cgroup 字符串 fixture：
   - `0::/user.slice/user-1001.slice/user@1001.service/app.slice/ccbd.service` -> `Some("ccbd.service")`
   - `0::/user.slice/user-1001.slice/user@1001.service/app.slice/ah-p1.service` -> `Some("ah-p1.service")`
   - `0::/user.slice/user-1001.slice/user@1001.service/app.slice/ccbd.service/session-1.scope` -> `Some("ccbd.service")`
   - `0::/user.slice/user-1001.slice/user@1001.service/app.slice/app-org.gnome.Terminal.slice/vte-spawn.scope` -> `None`
   - `0::/user.slice/user-1001.slice/user@1001.service` -> `None`
   - `0::/init.scope` -> `None`
   - `0::/user.slice/user-1001.slice` -> `None`
2. `src/sandbox/systemd.rs` agent scope tests:
   - detected unit `Some("ccbd.service")` -> argv contains `--property=BindsTo=ccbd.service` and `--property=PartOf=ccbd.service`
   - detected unit `Some("ah-p1.service")` -> argv contains `--property=BindsTo=ah-p1.service` and `--property=PartOf=ah-p1.service`
   - detected unit `None` -> argv contains no `--property=BindsTo=` and no `--property=PartOf=`
   - `--slice=ccb-p1-ccbd-agents.slice` remains unchanged
3. `src/sandbox/systemd.rs` master scope tests:
   - detected unit `Some("ccbd.service")` -> argv contains `BindsTo=ccbd.service` and `PartOf=ccbd.service`
   - detected unit `Some("ah-p1.service")` -> argv contains `BindsTo=ah-p1.service` and `PartOf=ah-p1.service`
   - detected unit `None` -> no `BindsTo=`/`PartOf=`
   - `--slice=ccb-p1-ccbd-workspace.slice` remains unchanged
4. `src/tmux/scope.rs` tests:
   - `UnitConfig.binds_to=Some("ccbd.service")` -> `wrap_in_scope` emits `--property=BindsTo=ccbd.service` and `--property=PartOf=ccbd.service`
   - `UnitConfig.binds_to=Some("ah-p1.service")` -> emits both properties for `ah-p1.service`
   - `UnitConfig.binds_to=None` -> emits neither property
   - `--unit=ccbd-tmux-abc123de` remains unchanged
5. Update existing assertions:
   - `systemd.rs:151` from hardcoded global bind to detected-unit fixture plus `PartOf`
   - `systemd.rs:186` keeps “not session anchor” assertion, but checks actual daemon unit semantics
   - `systemd.rs:253` uses detected-unit fixture, not project-derived `ah-p1.service`
   - `tests/r1_bindsto_alignment.rs` changes only bind target assertions; `--unit=ccbd-tmux-*` assertions remain unchanged

**Acceptance:**

- Running targeted tests after T1 should fail RED for the expected missing API/behavior.
- No production code is changed in T1 except test code.
- No test expects `BindsTo=ah-<project>.service` unless that value is explicitly supplied as detected actual unit.

## T2 - Implement Actual Daemon Unit Detection

**Depends on:** T1 RED

**Goal:** Add the parser and runtime detector required by the RED tests.

**Files:**

- `src/tmux/scope.rs` or a small shared module used by both `tmux/scope.rs` and `sandbox/systemd.rs`

**Implementation:**

1. Add `detect_current_service_unit_from_cgroup(cgroup: &str) -> Option<String>`.
2. Add `is_daemon_service_unit(unit: &str) -> bool` with whitelist semantics:
   - accept `ccbd.service`
   - accept `ah-*.service`
   - reject `user@*.service`, `init.scope`, `session-*.scope`, `*.slice`, arbitrary `.scope`
3. Add `detect_current_service_unit() -> Option<String>` I/O wrapper that reads `/proc/self/cgroup`.
4. Keep parser independent of `project_id`; never derive bind target from project id.

**Acceptance:**

- T1 parser fixture tests pass.
- `user@1001.service` only case returns `None`.
- Parser returns daemon service when `user@1001.service` and `ccbd.service`/`ah-p1.service` coexist.

## T3 - Thread Detected Unit Through Daemon Context And Agent/Master Builders

**Depends on:** T2

**Goal:** Compute actual daemon unit once and pass it into agent/master scope builders as data, keeping builders testable.

**Files:**

- `src/bin/ccbd.rs`
- `src/rpc/mod.rs` or the `Ctx` owner file
- `src/rpc/handlers.rs`
- `src/sandbox/systemd.rs`

**Implementation:**

1. During daemon startup, compute `daemon_unit = detect_current_service_unit()`.
2. Store `daemon_unit: Option<String>` in daemon context.
3. Pass `ctx.daemon_unit.as_deref()` into `systemd::wrap_command`.
4. Pass the same detected unit into `systemd::master_command`.
5. Update `wrap_command` and `master_command` signatures to accept `daemon_unit: Option<&str>`.

**Acceptance:**

- Agent and master scope tests for `Some("ccbd.service")`, `Some("ah-p1.service")`, and `None` pass.
- Slice names remain unchanged.
- Scope description/name behavior remains unchanged.
- No code path constructs `ah-<project>.service` from `project_id` for binding.

## T4 - Inject BindsTo And PartOf Conditionally In systemd.rs

**Depends on:** T3

**Goal:** Make `src/sandbox/systemd.rs` produce the new argv contract.

**Files:**

- `src/sandbox/systemd.rs`

**Implementation:**

1. Add helper to append dependency properties:
   - if `daemon_unit=Some(unit)`: append `--property=BindsTo=<unit>` and `--property=PartOf=<unit>`
   - if `daemon_unit=None`: append nothing
2. Use helper in agent `wrap_command` under-systemd branch.
3. Use helper in master `master_command` under-systemd branch.
4. Preserve dev mode behavior: when `env_state.under_systemd == false`, no dependency properties.

**Acceptance:**

- `systemd.rs:151/:186/:253` migrated tests pass.
- New no-detect tests pass.
- `--slice=ccb-p1-ccbd-agents.slice` and `--slice=ccb-p1-ccbd-workspace.slice` remain unchanged.

## T5 - Update tmux Scope Binding To Use Actual Unit

**Depends on:** T2

**Goal:** Align tmux server scope with agent/master scope dynamic binding.

**Files:**

- `src/tmux/scope.rs`
- `tests/r1_bindsto_alignment.rs`

**Implementation:**

1. Replace `detect_self_in_service() -> bool` with `detect_current_service_unit() -> Option<String>`.
2. `detect_scope_policy` should set `UnitConfig.binds_to` to the detected actual unit.
3. `wrap_in_scope` should emit both `BindsTo=<unit>` and `PartOf=<unit>` when `binds_to=Some(unit)`.
4. Leave `unit_name_for_socket` and `--unit=ccbd-tmux-...` unchanged.

**Acceptance:**

- tmux `Some("ccbd.service")`, `Some("ah-p1.service")`, and `None` tests pass.
- `r1_bindsto_alignment.rs` passes with updated bind assertions.
- No test or code path renames tmux scope units.

## T6 - Full Verification And Cutover Guard

**Depends on:** T1-T5 green

**Goal:** Verify PR3 cutover as one coherent change.

**Files:**

- no additional files expected beyond T1-T5

**Commands:**

- `CARGO_BUILD_JOBS=1 cargo test --test r1_bindsto_alignment -- --test-threads=1`
- `CARGO_BUILD_JOBS=1 cargo test --all-targets -- --test-threads=1`

**Acceptance:**

- All PR3 unit/integration tests pass.
- Full test suite passes under the project’s normal real-provider gating policy.
- No `BindsTo=ccbd.service` hardcoding remains except as a test fixture detected actual unit.
- No `BindsTo=ah-<project>.service` is derived from `project_id`.
- No scope name or slice name is changed as part of PR3.
