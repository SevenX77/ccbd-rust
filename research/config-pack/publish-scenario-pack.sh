#!/usr/bin/env bash
# ============================================================================
# publish-scenario-pack.sh — 发布配置包到公开仓 SevenX77/ah-scenario-pack
# ============================================================================
# 规矩(2026-07-10 用户拍板,固化防漂移):
#
#   1. SOURCE OF TRUTH = 本目录的 pack/(research/config-pack/pack/)。
#      公开仓 ah-scenario-pack 只是【发布产物】,不在公开仓直接改内容。
#      外部 PR / 公开仓侧任何独立改动,必须先【回流 dev pack/】再发,否则下次发布被覆盖丢失。
#
#   2. 发布只跑本脚本 —— 固定剪裁,不手工 cp。手工同步正是 v0.5.0→v0.5.1 双向漂移的根因
#      (dev ROLES 漏改矛盾没回流 / 公开仓 examples hotfix 差点被覆盖)。
#
#   3. 权威文件(从 dev pack/ 同步覆盖公开仓):
#        顶层 README/GUIDE/OPERATOR/ROLES/VERIFY + ah.toml.example
#        + .ah/rules/*.md + dual-lane/
#   4. 公开仓自有(脚本不覆盖,公开仓维护):examples/、CHANGELOG.md
#   5. 剔除(绝不发):.claude/、hook_audit.log 等内部运行产物
#
#   6. 三道门必过:漂移检测门(step2) → leak-gate 四闸(step4) → 人工审 + 手动 push(step5)。
#      脚本不自动 push;发布是 operator 独占、不可逆动作,末尾打印命令由人确认执行。
#
# 用法: ./publish-scenario-pack.sh vX.Y.Z
# ============================================================================
set -euo pipefail

VERSION="${1:?用法: $0 vX.Y.Z (例: v0.5.2)}"
DEV_PACK="$(cd "$(dirname "$0")/pack" && pwd)"
WORK="$(mktemp -d)/scenario-pack"
PUBLIC_REPO="SevenX77/ah-scenario-pack"
AUTHORITATIVE_TOP="README.md GUIDE.md OPERATOR.md ROLES.md VERIFY.md ah.toml.example"

echo "== 1. clone 公开仓 → $WORK =="
gh repo clone "$PUBLIC_REPO" "$WORK" -- --quiet

echo "== 2. 漂移检测门:公开仓有没有 dev 没有的改动?=="
DRIFT=0
check_drift() {  # $1 = 相对路径
  local rel="$1"
  [ -f "$WORK/$rel" ] || return 0
  if ! diff -q "$DEV_PACK/$rel" "$WORK/$rel" >/dev/null 2>&1; then
    echo "  ⚠ $rel: dev 与公开仓不一致"
    DRIFT=1
  fi
}
for f in $AUTHORITATIVE_TOP; do check_drift "$f"; done
for f in $(cd "$DEV_PACK/.ah/rules" 2>/dev/null && ls *.md); do check_drift ".ah/rules/$f"; done
if [ "$DRIFT" = 1 ]; then
  echo "  → 逐个确认【dev 领先】(dev 是 source of truth)。"
  echo "  → 若某文件是【公开仓独有 hotfix】,先回流 dev pack/ 再重跑本脚本,否则会被覆盖丢失。"
  echo "  → 全部确认 dev 领先后,本警告可忽略,继续同步。"
fi

echo "== 3. 剪裁同步:dev pack/ 权威内容 → staging =="
for f in $AUTHORITATIVE_TOP; do cp "$DEV_PACK/$f" "$WORK/$f"; done
cp "$DEV_PACK"/.ah/rules/*.md "$WORK/.ah/rules/"
rm -rf "$WORK/dual-lane"; cp -r "$DEV_PACK/dual-lane" "$WORK/dual-lane"
# examples/ 与 CHANGELOG.md 不动(公开仓维护;记得手工把 $VERSION 段加进 CHANGELOG)
# 剔除内部产物
find "$WORK" -path "$WORK/.git" -prune -o \( -name hook_audit.log -o -name .claude \) -exec rm -rf {} + 2>/dev/null || true

echo "== 4. LEAK-GATE 四闸 =="
cd "$WORK"
git add -A
ALLOW=".ah ah.toml.example CHANGELOG.md dual-lane examples GUIDE.md OPERATOR.md README.md ROLES.md VERIFY.md"
for e in $(ls -A | grep -v '^\.git$'); do
  echo "$ALLOW" | grep -qw "$e" || { echo "  ✗ 闸1 非白名单顶层项: $e"; exit 1; }
done
if git grep -inE "\.kiro|research/|ccbd-rust|/home/sevenx" --cached -- ':!CHANGELOG.md' | grep -q .; then
  echo "  ✗ 闸2 内部路径泄漏"; exit 1; fi
if git grep -in "traveluxi.com@gmail" --cached | grep -q .; then
  echo "  ✗ 闸3 gmail 敏感项泄漏"; exit 1; fi
if find . -path ./.git -prune -o \( -name hook_audit.log -o -name .claude \) -print | grep -q .; then
  echo "  ✗ 闸4 内部产物残留"; exit 1; fi
echo "  ✓ leak-gate 四道全过"

echo "== 5. 改动清单(人工审 → 手动发布)=="
git status --short
cat <<NEXT

────────────────────────────────────────────────────────────
发布前确认:CHANGELOG.md 是否已加 $VERSION 段?(脚本不动 CHANGELOG)
确认后 operator 手动执行:
  cd $WORK
  git commit -m "release: $VERSION"
  git push origin main
  git tag -a $VERSION -m "$VERSION"
  git push origin $VERSION
────────────────────────────────────────────────────────────
NEXT
