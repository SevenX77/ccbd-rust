# dev-programming 场景样板

配置包的**实例化样板**:1 master + 2 实现者(codex)+ 1 设计者(antigravity)+ 1 审计者(claude)。这是我们「用 ah 自托管开发 ah」实际在跑的拓扑,直接可用。

## 装法

1. 把本目录的 `.ah/` 拷进你的项目根;`ah.toml` 内容并进你项目的 `ah.toml`。
2. 填一份你项目的 `VERIFY.md`(模板见配置包根部),并把关键构建/测试命令同步进 `a1.md`/`a2.md`。
3. `ah start`。

## 和模板的对应关系

| 本样板文件 | 配置包模板 | 说明 |
|---|---|---|
| `.ah/rules/master.md` | `pack/.ah/rules/master.md` | 原样 |
| `.ah/rules/a1.md` / `a2.md` | `pack/.ah/rules/impl.md` | 同内容双实例,纯为并发 |
| `.ah/rules/a3.md` | `pack/.ah/rules/design.md` | 设计者 |
| `.ah/rules/a4.md` | `pack/.ah/rules/audit.md` | 审计者 |

改拓扑(增删实例、换 provider、换 id)看 `ROLES.md` 的能力表;协作规矩看 `GUIDE.md`。
