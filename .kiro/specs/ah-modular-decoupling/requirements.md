# ah Modular Decoupling + Architecture Index Requirements

Status: requirement capture (design NOT started). Captured by operator 2026-07-12 from user verbatim directives. This is the **first task** of the post-换血#6 program per the user's 2026-07-12 priority ruling ("第一个任务应该是做模块化梳理"). Design (design.md) is owned by the design layer (o1 diverge + d1 pen); implementation by the codex implementers (multiple in parallel); this file only captures the requirement so it is not lost.

## Source (user verbatim)

- 2026-07-12 15:57: "换血重启后，我希望做一轮模块化解耦，同时整理一份项目架构地图索引文档，设计时必须先看索引，了解架构各模块功能，知道这个仓库有什么，避免刚才的事情发生"
- 2026-07-12 16:45: "你说的模块化的事情，多开几个codex一起做，codex现在预算用不掉，Claude比较吃紧要省着点用。记得要监控context，换任务一定要清ctx"

"避免刚才的事情发生" refers to the same-session incidents where the fleet made decisions on stale/partial knowledge of the codebase (e.g. the `current_exe` gateway defect and the stale-local-main wrong-PR-base incident) — designing/implementing without first knowing which modules already exist and what they do.

## Governing principles (project-wide, user-fixed)

第一性原理 · 不打补丁(同族病 >=2 次 = 结构病,升维重设计) · 模块化低耦合高内聚 · 不要后兼容(可推倒重写,不背历史包袱). See `research/design-principles.md`.

## Requirement MD1: Architecture Map / Index Document

The repository MUST have a maintained architecture index document that enumerates, for every module/subsystem, what it is, what it is responsible for, and where it lives, such that any design turn can start by reading the index instead of re-discovering the codebase.

Acceptance criteria:

- The index enumerates every top-level module/subsystem of `ah`/`ahd` with: name, one-line responsibility, source path(s), and key public entry symbols.
- The index captures the capability -> owner mapping (which module owns which capability), so a designer can answer "does this already exist, and where" without grepping blind.
- The index is a required first read for any design task (design SOP references it); a design that contradicts or ignores the index is a review-rejectable defect.
- The index has a freshness mechanism: it is updated at each module/PR close point so it does not silently drift from the code (a stale index is worse than none).

Out of scope for MD1: auto-generation tooling. A maintained hand-authored index that passes a drift check is acceptable for v1.

## Requirement MD2: Modular Decoupling Pass

The `ah`/`ahd` codebase MUST undergo one decoupling pass that raises cohesion and lowers coupling between modules, without preserving backward compatibility for internal structure (rewrite is allowed).

Acceptance criteria:

- Each decoupled module has an explicit, documented boundary (public surface) and does not reach into siblings' internals across that boundary.
- The decoupling is driven by the MD1 index (targets chosen from the capability->owner map, not ad hoc).
- No behavior regression: the full CI suite is green after each module's extraction (CI is the only full gate; local serial cargo does not substitute).
- Each module is extracted in its own worktree/PR (shared git tree constraint: only one branch/commit at a time), and merged at its own close point.

## Requirement MD3: Design-Before-Code Gate Wired to the Index

Every design task in this program MUST begin from the MD1 index and record which modules/capabilities it touched, so the index stays authoritative.

Acceptance criteria:

- A design deliverable states, up front, which index entries it read and which it changes.
- Reviewer (r1) rejects a design that invents a capability the index shows already exists, or that omits an index update for a module it changed.

## Execution constraints (from user 2026-07-12 16:45; orchestration, not acceptance)

- Run multiple codex implementers in PARALLEL (codex budget is under-utilized; Claude seats are scarce and used sparingly).
- Monitor each seat's context; on task switch, clear context (`/clear` at IDLE) to prevent context bleed.
- These are master's orchestration constraints, recorded here for traceability; they are not acceptance criteria for MD1-MD3.

## Status

- MD1 (architecture index): NOT started — no index document exists as of capture.
- MD2 (decoupling pass): NOT started.
- MD3 (design-index gate): NOT started.
- Priority: FIRST task of the post-换血#6 program (operator priority ruling 2026-07-12). Sequencing vs the in-flight credential (Module D) closure is an open operator decision.
