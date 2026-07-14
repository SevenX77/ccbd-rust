# HANDOFF · WSL2 真机验证 cgroup 委托 populated 信号

> 交给谁:在 **Windows / WSL2** 里的你(或 WSL2 里的一个 agent)。
> 完成后:把「第 5 步 报告模板」填好回报即可。全程 ~5 分钟,不改任何生产代码。

## 一句话:要验什么

在 **WSL2 真机**上确认:用 `systemd-run --user --scope -p Delegate=yes` 拉起的委托 cgroup,其子 cgroup 的 `cgroup.events` → `populated` 字段能否**准确翻转**——子进程活着时 `populated=1`,子进程退出后即使父进程还活着也翻到 `populated=0`。

这是 ah 编排底座重构里「完成检测靠内核 cgroup 信号、不靠读屏幕文本」的地基假设。**Linux 已验通**(populated 序列 `1 → 1 → 0 → 0`,success=true)。**WSL2 是唯一没验过的环境**,也是整个感知层设计里最后 ~15% 的不确定性。你验掉它,这块就闭环。

## 为什么这事在 WSL2 上很可能挂(先知道踩点在哪)

WSL2 和普通 Linux 差在这几处,任一处不满足,PoC 就跑不起来——**第 0 步预检就是逐个排查它们**:

1. **WSL2 默认可能没开 systemd**。老版本 WSL2 的 PID 1 不是 systemd。没有 systemd,`systemd-run --user` 直接没戏。需要 `/etc/wsl.conf` 里 `[boot]\nsystemd=true` 然后 `wsl --shutdown` 重启过。
2. **`systemd --user` 用户会话可能没起来**。`--user` 需要你的用户 manager(`user@<uid>.service`)+ `XDG_RUNTIME_DIR` + 用户 D-Bus 都在。WSL2 里非登录 shell 常常缺这个——**这是最可能的失败点**。
3. **cgroup v2 unified 层级**。脚本假设 `/sys/fs/cgroup` 是纯 v2(cgroup2fs)。WSL2 内核较新一般是,但要确认不是 hybrid/v1。
4. **委托是否被接受**。即便前三关过,WSL2 的 systemd 也可能拒绝 `Delegate=yes` 或委托层级行为不一致。

## 第 0 步:环境预检(先跑,逐行看结果)

```bash
echo "== 1. 是不是 WSL2 =="; uname -r    # 期望含 'microsoft-standard-WSL2'
echo "== 2. PID1 是不是 systemd =="; cat /proc/1/comm   # 期望: systemd
echo "== 3. systemd 整体状态 =="; systemctl is-system-running   # running / degraded 都行,offline/不存在=没开
echo "== 4. cgroup 是不是纯 v2 =="; stat -fc %T /sys/fs/cgroup   # 期望: cgroup2fs
echo "== 5. 用户会话/运行时目录 =="; echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"; systemctl --user is-system-running 2>&1
echo "== 6. --user + Delegate 能不能拉起来(关键冒烟) =="; \
  systemd-run --user --scope -p Delegate=yes --collect true 2>&1 && echo "SMOKE_OK" || echo "SMOKE_FAIL"
```

判读:
- 第 6 行打印 `SMOKE_OK` → 环境 OK,进第 1 步。
- 第 6 行 `SMOKE_FAIL` 或前面某行明显不对 → **先别跑主实验**,把这 6 行的完整输出贴进报告的「预检」段,那本身就是结论(WSL2 环境不支持,需降级路径)。常见修法:
  - 第 3 行 offline/systemd 不在 → 编辑 `/etc/wsl.conf` 加 `[boot]` + `systemd=true`,PowerShell 里 `wsl --shutdown`,重进 WSL 再试。
  - 第 5 行 `XDG_RUNTIME_DIR` 为空 / 第 6 行报 user bus 相关错 → 试 `loginctl enable-linger $USER` 后重开一个 **登录** shell(`ssh localhost` 或 `machinectl shell`),或 `export XDG_RUNTIME_DIR=/run/user/$(id -u)` 再试第 6 行。

## 第 1 步:落地 PoC 脚本

把下面整段存成 `cgroup_delegation_poc.py`(和 Linux 上 c2 跑的是**同一份**,便于逐字对比):

```python
#!/usr/bin/env python3
import argparse
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%S%z")


def proc_cgroup(pid="self"):
    text = Path(f"/proc/{pid}/cgroup").read_text().strip()
    for line in text.splitlines():
        parts = line.split(":", 2)
        if len(parts) == 3 and parts[0] == "0":
            return parts[2]
    raise RuntimeError(f"no unified cgroup entry in /proc/{pid}/cgroup: {text!r}")


def cgroup_path(relative):
    return Path("/sys/fs/cgroup") / relative.lstrip("/")


def read_events(path):
    values = {}
    for line in (path / "cgroup.events").read_text().splitlines():
        key, value = line.split()
        values[key] = value
    return values


def snapshot(label, path, payload_pid=None):
    entry = {
        "time": now(),
        "label": label,
        "path": str(path),
        "events": read_events(path),
        "payload_procs": (path / "cgroup.procs").read_text().splitlines(),
        "parent_pid": os.getpid(),
        "parent_cgroup": proc_cgroup("self"),
    }
    if payload_pid is not None:
        proc_file = Path(f"/proc/{payload_pid}/cgroup")
        entry["payload_pid"] = payload_pid
        entry["payload_cgroup"] = proc_cgroup(payload_pid) if proc_file.exists() else "exited"
    print(json.dumps(entry, sort_keys=True), flush=True)
    return entry


def wait_for_populated(path, wanted, timeout_s, payload_pid=None):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        last = snapshot(f"poll-populated-{wanted}", path, payload_pid)
        if last["events"].get("populated") == wanted:
            return last
        time.sleep(0.05)
    raise TimeoutError(f"payload populated did not become {wanted}; last={last}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--payload-sleep", type=float, default=1.0)
    parser.add_argument("--parent-hold", type=float, default=0.5)
    args = parser.parse_args()

    parent_rel = proc_cgroup("self")
    parent_path = cgroup_path(parent_rel)
    payload_path = parent_path / f"payload-{os.getpid()}"

    print(json.dumps({
        "time": now(),
        "label": "parent-start",
        "parent_pid": os.getpid(),
        "parent_cgroup": parent_rel,
        "parent_path": str(parent_path),
        "parent_comm": Path("/proc/self/comm").read_text().strip(),
    }, sort_keys=True), flush=True)

    payload_path.mkdir()
    try:
        snapshot("after-payload-cgroup-mkdir", payload_path)

        payload = subprocess.Popen([
            "/bin/sh",
            "-lc",
            f"printf 'payload-shell-start pid=%s\\n' $$; sleep {args.payload_sleep}",
        ])
        (payload_path / "cgroup.procs").write_text(f"{payload.pid}\n")

        after_move = snapshot("after-payload-pid-moved", payload_path, payload.pid)
        reached_one = wait_for_populated(payload_path, "1", 2.0, payload.pid)

        rc = payload.wait(timeout=args.payload_sleep + 3.0)
        print(json.dumps({
            "time": now(),
            "label": "payload-exited-parent-still-running",
            "payload_pid": payload.pid,
            "payload_returncode": rc,
            "parent_pid": os.getpid(),
            "parent_cgroup": proc_cgroup("self"),
        }, sort_keys=True), flush=True)

        reached_zero = wait_for_populated(payload_path, "0", 3.0, payload.pid)
        time.sleep(args.parent_hold)
        final = snapshot("final-parent-still-in-parent-scope", payload_path, payload.pid)

        print(json.dumps({
            "time": now(),
            "label": "summary",
            "success": (
                after_move["parent_cgroup"] == parent_rel
                and reached_one["events"].get("populated") == "1"
                and reached_zero["events"].get("populated") == "0"
                and final["parent_cgroup"] == parent_rel
            ),
            "observed_populated_sequence": [
                after_move["events"].get("populated"),
                reached_one["events"].get("populated"),
                reached_zero["events"].get("populated"),
                final["events"].get("populated"),
            ],
            "parent_cgroup": parent_rel,
            "payload_path": str(payload_path),
        }, sort_keys=True), flush=True)
    finally:
        try:
            payload_path.rmdir()
        except OSError:
            pass


if __name__ == "__main__":
    if shutil.which("systemd-run") is None:
        print("systemd-run not found", file=sys.stderr)
        sys.exit(127)
    main()
```

## 第 2 步:跑实验

```bash
timeout 30 systemd-run --user --scope -p Delegate=yes --collect \
  /usr/bin/python3 "$(pwd)/cgroup_delegation_poc.py"
```

(python3 路径不对就换成 `which python3` 的结果。)

## 第 3 步:判读 PASS / FAIL

看最后一行 `summary` JSON:

- **PASS**:`"success": true` 且 `"observed_populated_sequence": ["1", "1", "0", "0"]`
  → 和 Linux 一致,WSL2 也支持,这块 15% 不确定性闭环。
- **FAIL-A**:命令直接报错(scope 拉不起来 / user bus / Delegate 被拒)→ 属第 0 步那类环境问题,把报错原文贴报告。
- **FAIL-B**:scope 起来了但 `mkdir` 报权限错(委托没真正生效,子 cgroup 建不了)→ 记下报错。
- **FAIL-C**:序列不是 `1,1,0,0`(比如退出后 `populated` 卡在 `1`、或有抖动 `1,0,1,0`)→ 这是**最有价值的负面结果**,说明 WSL2 上信号不可靠,把完整 JSON 输出贴回来。

## 第 4 步(仅当第 3 步 FAIL 才做):验降级路径

主路径不行的话,验一下兜底方案还能不能用——即不看 `populated`,改**枚举 `cgroup.procs` + 排除父进程 PID**:

```bash
# 在 scope 内手动建子 cgroup、丢一个短命进程、轮询 cgroup.procs 是否清空
# (如果上面 mkdir 都失败,这步也免了,直接报告"委托不可用")
```
能枚举出「子进程在→cgroup.procs 非空,子进程退→清空」也算降级可用,报告里注明。

## 第 5 步:报告模板(填好回报即可)

```
### WSL2 cgroup PoC 验证结果

预检(第0步 6 行输出):
  1 uname: <...>
  2 pid1 : <...>
  3 is-system-running: <...>
  4 cgroup fstype: <...>
  5 XDG_RUNTIME_DIR / user is-system-running: <...>
  6 smoke: SMOKE_OK / SMOKE_FAIL(+报错)

主实验:
  跑没跑起来: 是/否(否则贴报错)
  summary.success: true/false
  observed_populated_sequence: [ , , , ]

结论(勾一个):
  [ ] PASS —— WSL2 populated 信号准确,与 Linux 一致,闭环
  [ ] FAIL —— 环境不支持/信号不可靠(属 FAIL-A/B/C 哪类 + 现象一句话)

降级路径(仅 FAIL 时填): 可用 / 不可用 / 未测
```

回报后我把结论并进设计草稿 §四 Q3 / §十一 最弱区#1,该环境这块就从「待验」翻成定论。
