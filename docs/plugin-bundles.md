# Plugin Bundles

Plugin bundles package project-owned skills, hooks, rules, and MCP server definitions under
`.ah/bundles/<name>`. A bundle is inert until `ah.toml` references it from `[master]` or an
`[agents.<id>]` table.

## Layout

```text
.ah/bundles/domain-x/
  bundle.toml
  skills/doc-writer/SKILL.md
  hooks/notify.sh
  rules/worker.md
```

Minimal complete `bundle.toml`:

```toml
name = "domain-x"
version = "1"

[skills]
include = ["doc-writer"]

[hooks]
PostToolUse = [{ command = "hooks/notify.sh" }]

[rules]
worker = "rules/worker.md"

[[mcp.servers]]
name = "context"
transport = "stdio"
command = "npx"
args = ["-y", "@example/context-server"]
env = { CONTEXT_TOKEN = "${CONTEXT_TOKEN}" }
```

Paths in `bundle.toml` are relative to the bundle directory and must stay inside that directory.
MCP placeholders use environment variable names only; validation and digests must not print resolved
secret values.

## Referencing Bundles

```toml
version = "1"

[master]
bundle = ["domain-x"]

[agents.a1]
provider = "claude"
bundle = ["domain-x"]
```

`ah up` sends bundle names to `ahd`; the daemon recomputes the current digest from disk during
realign, crash recovery, and master revive. Mutating bundle content without editing `ah.toml` is
therefore enough for `ah up` realign to observe bundle drift.

## Provider Translation

- Claude workers receive bundle skills, hooks, worker rules, and MCP config in Claude home layout.
- Codex workers receive bundle skills, hooks, worker rules, and MCP config in Codex home layout.
- Antigravity workers receive bundle skills, hooks, worker rules, and MCP config in `.gemini`.
- Master bundle rules are currently Claude-only. Codex and Antigravity master rules are rejected.
- Plugins are not part of bundles. Keep `plugins = [...]` in `ah.toml` during and after migration.

## CLI

Validate bundles referenced by the nearest `ah.toml`:

```bash
ah bundle validate
```

Validate every directory under `.ah/bundles`:

```bash
ah bundle validate --all
```

Validate explicit bundles:

```bash
ah bundle validate domain-x another-bundle
```

List bundles, references, counts, and status:

```bash
ah bundle list
```

Both commands accept the global `--config` flag.

## Migration

To migrate scattered project config into a bundle:

1. Copy skill directories from `.ah/skills/<name>` into `.ah/bundles/<bundle>/skills/<name>`.
2. Copy hook scripts into `.ah/bundles/<bundle>/hooks/` and reference them from `[hooks]`.
3. Copy worker or master rules into `.ah/bundles/<bundle>/rules/`.
4. Move MCP server entries into `[[mcp.servers]]`.
5. Add `bundle = ["<bundle>"]` to each target master or agent entry in `ah.toml`.
6. Run `ah bundle validate` and then `ah up`.

Coexistence is additive: scattered `skills`, `hooks`, `rules`, `plugins`, and bundle references can
be used together while migrating. Unreferenced `.ah/bundles/<name>` directories are inert, so a staged
rollout can commit bundle files before any worker uses them.

Rollback is the reverse: remove the bundle reference from `ah.toml`, restore the scattered
`skills`/`hooks`/rules/MCP entries, run `ah bundle validate` for the remaining references, and run
`ah up`. Leave the bundle directory in place if you want a quick rollback point; it has no effect
until referenced again.
