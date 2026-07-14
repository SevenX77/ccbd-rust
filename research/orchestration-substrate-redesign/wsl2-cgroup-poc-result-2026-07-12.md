# WSL2 真机验证结果 · cgroup 委托 populated 信号(2026-07-12)

**结论:PASS。** WSL2 真机上 `systemd --user` + `Delegate=yes` 委托 cgroup 的 `populated` 信号行为与 Linux 完全一致——底座重构感知层的最后 ~15% 不确定性闭环。

## 环境(用户 Windows/WSL2 真机)
- kernel: `6.18.33.2-microsoft-standard-WSL2`(WSL Ubuntu-24.04)
- PID1 = `systemd`;`systemctl is-system-running` = `running`
- cgroup fstype = `cgroup2fs`(纯 v2 unified)
- `XDG_RUNTIME_DIR=/run/user/0`;`systemctl --user is-system-running` = `running`
- 冒烟 `systemd-run --user --scope -p Delegate=yes --collect true` = `SMOKE_OK`

## 实验
命令:`systemd-run --user --scope -p Delegate=yes --collect /usr/bin/python3 /tmp/cgroup_delegation_poc.py`(与 Linux 基线同款脚本)
- `summary.success` = `true`
- `observed_populated_sequence` = `["1", "1", "0", "0"]` —— 与 Linux 基线逐字一致

## 意义
设计草稿 `design-substrate-redesign-draft-2026-07-12.md` §四 Q3(cgroup 委托 PoC)与 §十一 最弱区#1("WSL2 `--user` + `Delegate=yes` 未实测")据此从"待验"翻为"已验 PASS"。GF4 完成协议的 cgroup populated 兜底传感在 WSL2 环境成立,不必回退到自制降级路径。

验证人:用户真机(operator 交付 handoff `HANDOFF-wsl2-cgroup-poc-verify.md`,用户执行并回报)。一次性脚本已从 /tmp 清理。
