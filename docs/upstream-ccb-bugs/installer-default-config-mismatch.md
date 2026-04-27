# 上游 CCB Bug：项目级 ccb.config 默认模板与用户偏好长期不对齐

| 字段 | 值 |
|---|---|
| **状态** | Open |
| **首次记录** | 2026-04-26（ccbd-rust 项目立项当天又踩一次） |
| **影响范围** | 所有新建 CCB 项目（每次都需手改） |
| **所属仓库** | 上游 [bfly123/claude_code_bridge](https://github.com/bfly123/claude_code_bridge) 及其本地 fork `~/coding/claude_code_bridge/`（branch `personal`），**不是** ccbd-rust 仓库 |
| **优先级建议** | 中（不阻塞 workflow，但每次新建 CCB 项目都重复手改，是日常摩擦） |
| **撰写人** | Claude Opus 4.7（主控）+ sevenx（识别问题）|
| **建议交付者** | 任何对 `claude_code_bridge` Python 仓有写权限的开发者；可走 upstream PR 也可只在本地 fork 修 |

---

## 1. 上下文：什么是 "CCB installer"

`claude_code_bridge`（简称 CCB）是 sevenx 用的一个多 LLM-CLI 调度器（Python），用来让 master Claude 通过命令统一调度多个外部 agent CLI（codex / gemini / claude / opencode）。CCB 的安装产物部署在：

- 二进制入口：`/home/sevenx/.local/bin/ccb`（Python 脚本，调用下面的库）
- 库 + 配置：`/home/sevenx/.local/share/codex-dual/`（这个路径名是历史遗留，本来叫 codex-dual，现在已扩展成多 provider）
- 状态目录：每个项目一个 `<project>/.ccb/`（包含 `ccb.config`、`ccbd/`、`agents/` 等子目录）

CCB installer 指的是这个工具的**安装/初始化逻辑**——它在两种场景执行：

1. **CCB 整体重装/升级**：手动跑 install 脚本时，会刷写 `~/.local/share/codex-dual/`，把库和 default 配置 dump 进去
2. **新项目初始化**：在某个项目目录下首次跑 `ccb` / `ccb -n` 命令时，CCB 会自动给该项目创建 `.ccb/` 目录并写入默认 `ccb.config`

第二种场景是本 bug 的触发点。

---

## 2. 现象（具体观察）

**今天（2026-04-26）在 ccbd-rust 项目首次启动 CCB**，主控 Claude 自动检测到 `.ccb/ccb.config` 已被 installer 创建（mtime `10:41:48`，比 git initial commit 晚约 1 小时），内容是：

```
cmd, agent1:codex; agent2:codex, agent3:claude
```

注意三处异常：
1. **agent 短名格式不一致**：用的是 `agent1/agent2/agent3`，不是 sevenx 日常用的 `a1/a2/a3`
2. **agent2 = codex，不是 gemini**：sevenx 个人偏好是 `a2:gemini`（用于分析 / 思考 / 领域问题），项目级 default 写死 codex
3. **分隔符混用**：`agent1:codex; agent2:codex, agent3:claude`——第一个用 `;` 第二个用 `,`，看起来像手 type 错误的样本

---

## 3. 期望行为

新项目的 `ccb.config` 应当与用户的全局 `~/.ccb/ccb.config` 保持一致：

```
cmd, a1:codex, a2:gemini, a3:claude
```

或者至少：当 `~/.ccb/ccb.config` 存在时，installer 在新项目里**复用同一份内容**而不是写自己的 default。

---

## 4. 实际行为

CCB installer 在 `/home/sevenx/.local/share/codex-dual/lib/agents/config_loader_runtime/defaults_runtime/project.py` 里**硬编码**了 default：

```python
DEFAULT_AGENT_ORDER = ('agent1', 'agent2', 'agent3')

# 项目级 default（也写死在同文件里）：
('agent1', 'codex'),
('agent2', 'codex'),
('agent3', 'claude'),
```

这套硬编码值与用户 18 天反复使用的偏好（`a1:codex, a2:gemini, a3:claude`）**毫无对齐**。每次新建 CCB 项目，用户都要：
1. 注意到 default 错的
2. 手改 `.ccb/ccb.config`
3. 重启 ccbd（如果 ccbd 已起，要 `ccb -n` 或 kill 重起才能生效）

**多次升级 / 多次 patch 都没碰过这条 default**。Phase 1（v6.0.1 → v6.0.7）7 个补丁全部围绕 isolation / lifecycle / kill，没有人碰 installer 的 default config。

---

## 5. 影响

| 维度 | 影响 |
|---|---|
| **每次新建项目** | 用户 / 主控 Claude 必须手改 `.ccb/ccb.config`，否则 `a2:gemini` 派活全部派给 codex，目标错位 |
| **fork 上游污染** | `claude_code_bridge` 仓库自己也踩这个 bug——`.ccb/ccb.config` 被本地手改，但 git 里上游版本仍是 `agent1..agent5`，每次 upstream 同步会冲突。本地用 `git update-index --skip-worktree .ccb/ccb.config` 绕过（见 `~/.claude/rules/ccb-collaboration.md` "claude_code_bridge 源码仓的特殊处理"段）|
| **新主控 Claude 不知道**| 主控 Claude 第一次进新项目时，按"a2 应该是 gemini"的全局规矩派活，结果派给 codex。轻则结果不对，重则消耗 codex quota 跑分析任务（codex 不擅长） |
| **Issue 已存在** | upstream issue [#191](https://github.com/bfly123/claude_code_bridge/issues/191) 讨论是否改成 `.ccb.config.example` 清根，但未推进 |

---

## 6. 根因

**根因 1：default 硬编码在源码里，不查全局偏好**

源码：`/home/sevenx/.local/share/codex-dual/lib/agents/config_loader_runtime/defaults_runtime/project.py`

initialization 逻辑没有 fallback 链：
- 没有"先看 `~/.ccb/ccb.config` 是否存在 → 复用"的步骤
- 没有"读环境变量 `CCB_DEFAULT_AGENTS` 之类"的覆盖点
- 直接 dump 写死的 (`agent1:codex`, `agent2:codex`, `agent3:claude`)

**根因 2：default 模板的内容本身可疑**

`agent1/agent2/agent3` 这种"长名 + 数字尾"是 upstream 早期的 demo 命名，sevenx 早就改用 `a1/a2/a3` 短名（更易在 prompt / log / shell 里 ref）。upstream 没跟上这个简化。

**根因 3：installer 升级时不迁移现有项目的 config**

每次 CCB 主版本升级（v5 → v6），installer 只刷新 `~/.local/share/codex-dual/`，不动各项目 `.ccb/ccb.config`。这本身是对的（不该破坏用户的手改），但反过来意味着：
- 旧项目升级后 default 没变化（用户已手改的保留）
- 但**新项目仍然踩同一个不对齐的 default**

---

## 7. 修复方向（建议，三选一）

**方案 A（最小改动）**：把 `defaults_runtime/project.py` 里的 hardcoded default 改成与用户偏好对齐的 `(a1:codex, a2:gemini, a3:claude)`。
- 优点：一行改动，立即修复
- 缺点：这是个人偏好硬编码到上游，对其他用户不友好；upstream 不一定接受

**方案 B（推荐）**：installer 初始化新项目时，**优先复用** `~/.ccb/ccb.config`（如果存在），fallback 到 hardcoded default。
- 优点：尊重用户全局偏好；其他用户不受影响（他们可能没有 `~/.ccb/ccb.config`，仍走 default）
- 缺点：需要改 initialization 逻辑（约 10-20 行 Python）

**方案 C（清根）**：删除 hardcoded default，把 `.ccb/ccb.config` 改成 `.ccb/ccb.config.example`，强制用户显式 `cp ccb.config.example ccb.config` 后手填，installer 不写 default。
- 优点：彻底没有"被自动写错"的可能性
- 缺点：用户体验变差（多一步手填）；upstream issue [#191](https://github.com/bfly123/claude_code_bridge/issues/191) 在讨论这个方向

**sevenx 倾向方案 B**——既不破坏其他用户的 default 流程，又能让自己的全局偏好成为新项目的真实 default。

---

## 8. 验证清单（修完后请按此项验证）

- [ ] 在一个全新目录（如 `/tmp/test-ccb-new-project/`）跑 `ccb`，检查生成的 `.ccb/ccb.config`：是否是 `a1:codex, a2:gemini, a3:claude`（或当时 `~/.ccb/ccb.config` 的内容）？
- [ ] 在没有 `~/.ccb/ccb.config` 的清洁系统上重复上一步：是否落回到一份**清晰、已对齐 sevenx 习惯**的 default（不再是 `agent1/agent2/agent3` 长名）？
- [ ] 现有项目（如 `~/coding/claude_code_bridge/`、`~/coding/ccbd-rust/`）的 `.ccb/ccb.config` **不被修复脚本动**——保留用户手改

---

## 9. 不要做的事

1. **不要在 ccbd-rust 仓库里修这个 bug**——ccbd-rust 是 Rust 重写，跟 Python 的 CCB installer 是两个独立项目。等 ccbd-rust M7 完成后，旧 CCB 整体废弃，这个 bug 自然 obsolete
2. **不要把这个修复跟 ccb dispatch / completion detection bug 合在一个 PR 里**——那些是更复杂的状态机问题（synthesis-18-days 群 A），跟 installer 的 default config 无关
3. **不要 force overwrite 已有项目的 `.ccb/ccb.config`**——用户可能已手改

---

## 10. 相关文件路径（修复时 grep 起点）

- `/home/sevenx/.local/share/codex-dual/lib/agents/config_loader_runtime/defaults_runtime/project.py` — hardcoded default
- `/home/sevenx/.local/share/codex-dual/lib/agents/config_loader_runtime/common.py` — `DEFAULT_AGENT_ORDER`
- `~/coding/claude_code_bridge/lib/agents/config_loader_runtime/` — 上游源码 fork（修复在这里做，再同步到 install 路径）
- `~/.ccb/ccb.config` — sevenx 个人全局偏好（方案 B 的 fallback 来源）
- `~/.claude/rules/ccb-collaboration.md` — 全局规则文档，已记录"claude_code_bridge 源码仓的特殊处理"

---

## 11. 提交者注意事项

- 修改后请在本地用 `~/coding/ccbd-rust/.ccb/ccb.config` 之类的现有项目的 `.ccb/ccb.config` 不被破坏的前提下验证
- 如果走 upstream PR：附上本文档作为问题描述，并提供方案 B 的 patch
- 如果只在本地 fork 改：提醒 sevenx 在 install 脚本流程里复用本地改的 `defaults_runtime/project.py`
