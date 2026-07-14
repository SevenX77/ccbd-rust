# ah#18 Per-Worker Credentials — Tier-3 真机验收 Runbook

执行者:operator,在用户 Win11/WSL2 真机上跑(master 不代跑,交付到「就绪」为止)。
前置:PR #151(`feat/claude-shared-credentials-dir`)已合入 main,ah 已用含该改动的构建在真机部署。

## 0. 准备

1. 确认用户平时登录的那份 claude 凭据文件路径(Windows 侧,例如 `C:\Users\<user>\.claude\.credentials.json`),记下其 WSL 侧 drvfs 路径(例如 `/mnt/c/Users/<user>/.claude`)。
2. 在 ah 项目配置(`ah.toml` 或对应 provider 配置)里,把 `[providers.claude].shared_credentials_dir` 设为该 drvfs 目录的绝对路径(**目录本身**,不是 `.credentials.json` 文件路径;必须是真实存在目录、非 symlink)。
3. 确认用户当前用自己平时的 CLI 已登录一次(北极星前提:单次登录)。
4. 记录验证开始前的凭据文件 mtime 与 `expiresAt`(如可读),作为前后对比基线。

## 1. 验收项 ① —— worker 骑用户单次登录起到 IDLE

1. 按新配置起一个 ah claude worker(或 master)。
2. 观察:该 agent **不需要用户重新登录**,直接进入可用状态(`ah ps` 显示 IDLE / 正常应答任务)。
3. 判定:
   - PASS:worker 无需二次登录即可工作。
   - FAIL:worker 卡在登录态/报凭据错误 —— 记录具体报错 + `ah ps` sub_state,回退给 master 排查(不要现场改代码)。

## 2. 验收项 ② —— 刷新时新 RT 原地写进 Windows 真文件

1. 触发一次凭据刷新(可通过等待自然过期触发的 401 被动刷新,或用已知的手段促使一次请求触发刷新;具体触发方式按现场情况,若没有更快手段就等待自然过期窗口)。
2. 刷新发生后,检查 Windows 侧真文件(`C:\Users\<user>\.claude\.credentials.json`,而不是 WSL 侧任何其它路径)的 mtime 是否更新、`expiresAt`/token 内容是否变化。
3. 判定:
   - PASS:Windows 真文件原地更新(mtime 变化,新 token 内容),没有产生任何游离副本文件。
   - FAIL:真文件未更新,或者出现了写入其它路径的副本 —— 记录现象,回退排查(是否 direct-dir 配置错误、是否又混入了 symlink)。

## 3. 验收项 ③ —— 第二个 worker / 用户宿主 claude 不被登出

1. 在验收项②触发刷新的同时或之后,确认:
   - 第二个 ah claude worker(若已起)仍然可用,不需要重新登录。
   - 用户在 Windows 上平时用的宿主 claude CLI 仍然是登录状态,不需要重新登录。
2. 判定:
   - PASS:两者均未被登出。
   - FAIL:任一方被登出 —— 立即记录时间戳、涉及的 agent/进程、当时的凭据文件状态,回退给 master,不要自行重试掩盖。

## 4. 已知残余风险(不阻塞 PASS,但要如实记录是否触发)

以下是 requirements.md 里 operator 已签字接受的残余赌注,tier-3 跑的时候如果**没有触发**就正常记录「未触发」,如果**触发了**要如实记录,不能因为「文档说可接受」就不记:

- **D-1 残余窄竞态**(2026-07-12 用户签字接受):`ahd 宕机 + access token 恰好过期 + 用户正好在用原生 CLI` 三条件同时发生时,原生 CLI 可能被登出一次。这是 RTR 机制本身的物理天花板,不算本方案缺陷,但如果 tier-3 期间真的撞上这个窗口触发了一次登出,要记录现象+时间戳,不要误判成新 bug 也不要忽略不记。
- **F5 残余赌注**(仅当 Layer 2 定点代理停用/失效时才有意义;本方案的 Layer 2/Gateway 已撤,大概率不适用,如果观察到任何"上游对失效 token 触发盗用检测级联"的迹象,立即记录并升级,不要自行判断"应该没事"。

## 5. 汇报格式

跑完后向 master/operator 汇报,格式:

```
验收项①:PASS/FAIL(证据:...)
验收项②:PASS/FAIL(证据:...)
验收项③:PASS/FAIL(证据:...)
残余风险 D-1:触发/未触发(如触发,附时间戳+现象)
残余风险 F5:触发/未触发(如触发,附时间戳+现象)
```

三条全 PASS 才算「完成」;任一 FAIL,回退给 master 定位再复验,不允许"大部分过了就算完成"。
