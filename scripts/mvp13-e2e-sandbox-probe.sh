#!/usr/bin/env bash
# mvp13 legacy sandbox e2e probe: 已 obsolete
#
# PR2 T4 删除 bwrap 后不再有隔离 /home/agent first-run 探针路径。
# 现在默认模型是 provider HOME/env 重定向 + OAuth symlink；first-run 弹窗不再由
# bwrap 空 HOME 触发。保留本脚本为兼容入口，直接标记 obsolete 成功退出。
#
# 原先的 spawn/sleep/capture-pane 探针主体已移除，避免误跑失效前提。

set -euo pipefail

echo "OBSOLETE: bwrap sandbox first-run probe was removed by PR2 T4."
echo "Default isolation is covered by scripts/mvp13-e2e-sandbox.sh."
exit 0
