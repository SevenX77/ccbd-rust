# 第三例现场:WSL2/Windows 真机 symlink 登出穿透(2026-07-11,用户报告)

本文件是 ah-per-worker-credentials(模块 D,公开仓 ah#18)的新增现场证据。机制与前两例一致,但环境升级为**用户 Win11/WSL2 真机**(Studio Req1 runbook 栈),且爆炸半径首次越出 ah 栈、殃及用户本人的 Windows claude 登录态。

## 现场证据链(用户原始取证)

1. master 的 pend 返回后尝试开转,直接吃到 `Not logged in · Please run /login`(pane 原文取证)。
2. SSOT 链 `/root/.claude/.credentials.json` → `/mnt/c/Users/test/.claude/.credentials.json`,链尾内容是 `expiresAt: 0` 登出残根——即 **Windows 本体文件被盖掉**。
3. 所有走这条链的 claude 席(master/d1/g1/g2)全部登出;**用户 Windows 上的 claude 同样是登出态**。

## 机制(与前两次一致,已归档 ah#18)

多实例共享交互登录链:某个席位 refresh 轮转竞态落败后,把登出残根(`expiresAt: 0`)写穿了 symlink,所有共享同一 inode 的席位同时失效。见 requirements.md「Existing Grounding」:`link_credentials` symlink 到同一物理文件,不是拷贝。

## 本例新增的严重性升级

- **爆炸半径越栈**:WSL2 里 ah 的 symlink 链尾指向 `/mnt/c` 上 Windows 用户的真实 credentials 文件,竞态残根直接**登出了用户宿主机的 claude**。此前两例只影响 ah 栈内 worker;本例证明共享链在 WSL2 部署形态下会伤害用户本人的工作环境。
- **Windows 原生/WSL2 是最高优先级方向**(用户"全力打通我要用"),该形态下此 bug 从"栈内运维痛点"升级为**用户可感知的产品缺陷**,修复优先级应随之上调。
- Studio Req1 真机验收(v1.3.0-rc)在此形态下运行,该 bug 是真机门的现役阻塞项之一。

## 对 spec 的影响

- P1(token gateway / Plan B)的动机再获一例实证;方案本身不变。
- design 阶段需补一条 WSL2 特有验收:**任何情况下不得写穿到 `/mnt/c` 上的宿主 credentials 文件**——worker 侧根本不该持有指向宿主文件的可写链(Plan B 下 worker 无 `.credentials.json`,自然满足;迁移期/回退路径也要守住)。
- 复发计数:同族第 3 例(2026-06-29 首例、此前 ah#18 归档例、本例)。按「同族 bug 两次=结构病升维」纪律,本 spec 早已定性结构病;第 3 例进一步支持排期提前。
