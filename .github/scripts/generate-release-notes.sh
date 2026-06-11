#!/bin/bash
# 从 conventional commits 生成 Release Notes
# 用法: ./generate-release-notes.sh [from_tag] [to_ref]
# - from_tag: 上一个 tag（省略则自动检测）
# - to_ref: 当前 ref（默认 HEAD）

set -euo pipefail

FROM_TAG="${1:-}"
TO_REF="${2:-HEAD}"

# 自动检测上一个 tag
if [ -z "$FROM_TAG" ]; then
  FROM_TAG=$(git describe --tags --abbrev=0 "$TO_REF^" 2>/dev/null || echo "")
fi

# 获取 tag 消息（如果是 annotated tag）
TAG_NAME="${GITHUB_REF#refs/tags/}"
TAG_MESSAGE=""
if [ -n "$TAG_NAME" ] && [ "$TAG_NAME" != "$GITHUB_REF" ]; then
  TAG_MESSAGE=$(git tag -l --format='%(contents:body)' "$TAG_NAME" 2>/dev/null || echo "")
fi

# 获取 commit 列表
if [ -n "$FROM_TAG" ]; then
  RANGE="${FROM_TAG}..${TO_REF}"
else
  RANGE="${TO_REF}"
fi

COMMITS=$(git log "$RANGE" --pretty=format:"%s" 2>/dev/null || echo "")

# 按类型分类
FEATS=""
FIXES=""
REFACTORS=""
OTHER=""

while IFS= read -r commit; do
  [ -z "$commit" ] && continue
  # 提取 scope（如果有）
  msg="${commit}"

  case "$msg" in
    feat[\(\:]*)
      # 去掉 feat: 或 feat(xxx): 前缀
      body="${msg#feat}"
      body="${body#(*}"
      body="${body#)*: }"
      body="${body#: }"
      FEATS="${FEATS}- ${body}\n"
      ;;
    fix[\(\:]*)
      body="${msg#fix}"
      body="${body#(*}"
      body="${body#)*: }"
      body="${body#: }"
      FIXES="${FIXES}- ${body}\n"
      ;;
    refactor[\(\:]*)
      body="${msg#refactor}"
      body="${body#(*}"
      body="${body#)*: }"
      body="${body#: }"
      REFACTORS="${REFACTORS}- ${body}\n"
      ;;
    docs:*|chore:*|ci:*|style:*|test:*)
      # 跳过不重要的 commit
      ;;
    *)
      OTHER="${OTHER}- ${msg}\n"
      ;;
  esac
done <<< "$COMMITS"

# 输出 Release Notes
echo ""

# 如果有 tag 消息，优先展示
if [ -n "$TAG_MESSAGE" ]; then
  echo "$TAG_MESSAGE"
  echo ""
  echo "---"
  echo ""
fi

# 分类展示 commits
if [ -n "$FEATS" ]; then
  echo "## ✨ New Features"
  echo ""
  echo -e "$FEATS"
  echo ""
fi

if [ -n "$FIXES" ]; then
  echo "## 🐛 Bug Fixes"
  echo ""
  echo -e "$FIXES"
  echo ""
fi

if [ -n "$REFACTORS" ]; then
  echo "## 🔧 Refactoring"
  echo ""
  echo -e "$REFACTORS"
  echo ""
fi

if [ -n "$OTHER" ]; then
  echo "## 📦 Other Changes"
  echo ""
  echo -e "$OTHER"
  echo ""
fi

# Footer: 部署说明
VERSION="${TAG_NAME#v}"
REPO_LOWER=$(echo "${GITHUB_REPOSITORY:-user/repo}" | tr '[:upper:]' '[:lower:]')

echo "---"
echo ""
echo "## 🐳 Docker"
echo ""
echo '```bash'
echo "docker pull ghcr.io/${REPO_LOWER}:${VERSION}"
echo "docker pull ghcr.io/${REPO_LOWER}:latest"
echo '```'
echo ""
echo "## 📥 下载"
echo ""
echo "在下方 Assets 中选择对应平台的压缩包，解压后直接运行："
echo ""
echo '```bash'
echo "./galaxy-router --config config.toml"
echo '```'
echo ""
echo "## 📚 文档"
echo ""
echo "- [README](https://github.com/${GITHUB_REPOSITORY:-user/repo})"
echo "- [配置说明](https://github.com/${GITHUB_REPOSITORY:-user/repo}/blob/main/config.toml)"
