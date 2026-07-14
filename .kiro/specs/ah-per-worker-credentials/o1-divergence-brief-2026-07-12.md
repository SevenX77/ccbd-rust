# o1 发散 brief — Per-Worker Credentials 重做(第一性,多方案)

> 给 o1(设计辩论席)。你的职责:**发散 + 红队,不执笔冻结设计、这一轮也不做最终收敛**。产出多套候选方案交下一步评审收敛。

## 你要解决的真问题(北极星,别跑偏)

**用户只想登录一次**——用他平时自己用的那个 claude CLI 登录一次。之后 ah 栈里 N 个 claude worker 全部**骑在这一次登录上**干活:
- **不逼用户为每个 worker 重复登录**;
- **不能因为多 worker 并发把用户(或彼此)登出**。

隔离/网关/刷新器都只是**手段**;目标是"**单次登录、零重复登录、零互相登出**"的体验。凭据来自用户 Windows 本机正常登录的 claude。反面=现状 ah#18(多 worker 共享一份,谁刷新谁把别人乃至用户 Windows 本机踢下线)。

## 已坐实的二进制事实(基座,别推翻;不够就继续逆向补)

来自 operator 对生产 CLI 的逆向 + 2026-07-12 调研(详见同目录 `requirements.md`「根因」+「补充二进制事实」):

1. **凭据后端可插拔多后端**:macOS Keychain / Windows Credential Manager(`isWindowsCredManagerAvailable`)/ libsecret / `plaintext` 明文文件。**明文文件仅在无系统密钥库时兜底**——headless Linux + WSL2 正是这处境。→ **原生 macOS/Windows 靠单一 OS 密钥库仲裁,天生免疫本类 bug**;只有落明文后端才中招。
2. **Anthropic 用一次性轮换 refresh token(RTR,已确证)**:刷新一次即作废旧 refresh token、下发新的;旧的再用 → `400 invalid_grant` → 触发盗用检测、**连刚刷新成功那个也一起吊销**(级联)。这是 ah#18 根子。
3. **claude 只在收到 401 时被动刷新**(server 权威,非本地定时;startup 刷新弱且常 buggy)。
4. **claude 把 refresh token 缓存在内存、刷新前不重读文件**(强推断,社区 bug 报告佐证,唯一未源码坐实的一条)。→ "所有 worker 共享写同一个文件"**无效**:内存里那份不共享。
5. **写盘=原子改名**(`tmp+rename`,跨设备 `EXDEV` 退化 `copyFile` → WSL2 下会写穿 `/mnt/c` 宿主文件,即缺陷 B)。
6. 凭据文件结构:`claudeAiOauth{ accessToken, refreshToken, expiresAt(ms), refreshTokenExpiresAt(ms,~27天), scopes, subscriptionType, ... }`。access token TTL ~8–19h。

## 你的任务:发散 ≥4 套机制落地的候选方案

**核心不变量(每套方案都必须满足)**:改动后**没有任何 worker 能独立发起一次会轮换令牌血缘的刷新**。

发散要求:
- **挖到第一性,别停在表层**。operator 手上有一个草稿方案(**单一刷新器 + per-worker 保鲜拷贝 + worker 寿命 ≤ token TTL,让 worker 永不撞 401 → 永不刷新**)。**把它当成「要被你打败/超越的一个候选」,不是答案**。用户明确直觉:"这可能还不是最底层"。
- **特别深挖事实 1 指的那条线**:既然原生桌面靠"单一 OS 密钥库后端仲裁"天生免疫,**能不能给 WSL/headless 的 claude 也塞一个仲裁式 keystore 后端**(真 libsecret/gnome-keyring 单实例共享?自建假 keystore 后端伪装成系统密钥库?claude 探测 keystore 的判据具体是什么、能否满足?)——这可能是比"刷新器 daemon"更底层、更对齐 claude 自身抽象的解。**这需要你继续逆向 claude 二进制**把判据钉死。
- 也考虑:credential-helper/外部命令钩子(类似 git credential helper)claude 有没有;keychain/credmgr 后端在 WSL 里能否接上 Windows 侧;等等。
- 每套方案给全:**机制 / 怎么满足不变量 / load-bearing 假设 + 验证 spike(最小实验怎么证) / 复杂度 / 失效模式 / 是否需要进一步逆向哪块二进制**。
- **红队每一套**(含 operator 草稿):最狠的失败场景是什么?WSL2 `/mnt/c` 跨设备、长寿命 worker 自刷新、claude 版本升级改判据……哪些会击穿?

## 铁律 / 边界

- **只发散产出 markdown 文档,不写代码、不改任何 src、不碰活栈**(preModD 止血态在跑,别扰动)。
- 逆向可读生产二进制、读本机 `~/.claude/.credentials.json` 观察结构(**token 明文值绝不写进产出**)、查官方文档/社区 issue。跑命令必带 `timeout`。
- **plan-first**:先回一个发散大纲(你打算探哪几条机制线 + 各需逆向/验证什么),等评审点头再深挖,别一上来闷头写完。
- 这一轮**不做最终收敛、不执笔冻结设计**(那是评审收敛步 + d1 的活)。你的交付=候选方案集 + 每套的红队。
- 产出写到:`.kiro/specs/ah-per-worker-credentials/design-divergence-2026-07-12.md`。

## 交付判据

一份发散文档,含 ≥4 套机制落地、互相独立的候选,每套按上面模板写全 + 红队;显式标注"哪套需要哪些进一步的二进制事实才能定论"。**不许只写 operator 草稿的变体**——至少要有一套走 keystore-后端/OS-仲裁那条更底层的线。
