# o1 辩论单 · 模块 D 网关嫁接设计(轨1 首任务,2026-07-11)

## 你的角色
你是 o1(设计辩论席,只辩论不执笔)。本单产出是**辩论/红队备忘**,不是冻结设计——冻结稿由 d1 执笔。请穷举分歧点、风险、备选方案并给出你的倾向与理由,但不要产出"最终设计"格式的文档。

## 背景(必读,按顺序)
1. `research/ab-experiment-gateway/REVIEW-gateway-ab-verdict.md` —— A/B 独立终审裁决全文。**结论**:B 代码质量显著更高(38 vs 55 分,B 胜 6/7 维),**但 A 唯一胜在"设计运行时广度"**——A 真正搭起了 per-worker UDS 服务器 + 沙箱桥 + 挂载这条端到端管道,B 只有干净的 `GatewayCore` 核心逻辑 + env 拓扑串,`localhost:8206` 无人监听。裁决明确建议的路径:**把 A 的运行时管道嫁接到 B 的干净核心 + 凭据铲除之上**。
2. `.kiro/specs/ah-per-worker-credentials/design-rev.md` —— 冻结的 Plan B Fake Gateway 设计(架构拓扑、Fake JWT 构造 3.1、多租户物理隔离 3.2 两道防线、单飞刷新锁)。这是**架构权威**,嫁接不能违反其安全机制设计,尤其 3.2 的两道防线(UDS 物理隔离 + 应用层身份校验)必须真正落地为运行时代码,不能只是逻辑自洽。
3. `.kiro/specs/ah-per-worker-credentials/incident-2026-07-11-wsl2-symlink-logout.md` —— 第三例现场,WSL2 真机 symlink 写穿到 `/mnt/c` 宿主 Windows 用户凭据、致其登出。**验收铁律**:任何嫁接方案下,worker 侧绝不能持有指向宿主可写凭据文件的链接;Plan B 下 worker 本就无 `.credentials.json`,但要确认嫁接不会引入新的写穿路径(例如挂载/桥接脚本误写宿主文件)。

## 需要你辩论/红队的关键分歧点

1. **UDS 服务器落地方式**:A 的 `register_worker` 是否直接复用(裁决已指出该函数与其测试副本 `worker_gateway_for_test` ~180 行重复,且是"树红"的根源之一),还是在 B 的 `GatewayCore<U: ClaudeUpstream>` 泛型骨架上重新实现一个薄的 per-worker UDS listener 层,把 A 的服务器逻辑降解为对 `GatewayCore` 的调用?红队:重用 A 代码是否会把 A 的复制粘贴/树红问题一并继承进 B?

2. **沙箱桥接方式**:A 用内联 python3 heredoc(硬依赖 python3、朴素 4096 字节 recv 循环、后台 `&` 崩溃无感)。设计原文 3.2 提到 `socat` 或"内置轻量级转发器"。辩论:该不该在这次嫁接里直接把桥换成 ah 自己的小型 Rust TCP↔UDS 转发器(消灭 python3 依赖 + 崩溃可感知),还是先用 socat 占位、留后续任务?哪个对本任务范围更合适(不打补丁原则 vs 不过度扩大本次改动范围)?

3. **Fake JWT 签名方案**:A 用全局单一 HMAC 密钥签名(偏离设计 3.1 的 alg:none 空签名,也偏离 3.2 的"每-worker 密钥"),真正的隔离靠 per-socket 闭包硬编码 worker_id 比对。B 完全按 3.1 实现(alg:none)但缺少 3.2 第二防线(应用层身份校验在运行时无落地,因为没有服务器)。嫁接后:该按设计 3.1 用 alg:none,还是采纳 A 的思路加签名作纵深防御?3.2 第二防线(连接 UDS 身份 vs JWT 内 worker_id 一致性校验、不一致 403)必须在新服务器里真实现——这条怎么设计取舍?

4. **master 席位与 host env 剥离**:B 已经把这两条从源头修好(删 `PROVIDER_AUTH_WHITELIST` 项、删 `link_credentials`、master 复活链同步改 gateway 模型、`collect_spawn_env` 剥离宿主 `ANTHROPIC_API_KEY` 透传)。嫁接进服务器/桥之后,是否会有新路径重新引入这两个漏洞(例如服务器给 master 也分配 UDS 但接线到旧凭据逻辑)?红队检查点。

5. **invalid_grant 重试风暴 vs 永久黏死**:A 的失败态永久黏死(`last_failure` 一旦写入,不重启不自愈);B 每次都重试上游刷新(潜在速率保护风暴)。两者都不理想,嫁接是否该引入一个折中(如短 TTL 的失败态缓存 + 指数退避),还是这属于超出本次任务范围的独立改进项,应该记为后续 task 而非塞进这次嫁接?

6. **测试策略**:A 的 AC 测试大量打 `worker_gateway_for_test` 副本而非生产路径。嫁接后新服务器的测试该如何设计,才能像 B 的风格一样锚定"出货代码 + 可观测契约"(状态码/header/文件不存在等),而不是重蹈"打副本"的覆辙?

## 交付形式
- 一份辩论备忘,列出你对上述 6 点(以及你发现的其它分歧点)的倾向、理由、反方论据、你认为最强的反对意见是什么。
- 明确标注:你认为哪些决策**必须**在冻结设计里锁死(会影响实施范围/工时),哪些可以留给实施阶段自行判断。
- 不要写 tasks.md 或验收测试代码——那不是你的执笔权限。
- 完成后把备忘写入 `research/ab-experiment-gateway/o1-debate-memo-graft-2026-07-11.md` 并在会话里告知完成。
