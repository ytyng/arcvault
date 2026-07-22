#!/usr/bin/env bash
# GitHub Actions の Release ワークフローをトリガーし、完了まで watch する。
# `pnpm release` から呼ばれる。gh CLI (認証済み) が必要。
set -euo pipefail

cd "$(dirname "$0")/.."

# ビルドはリモート main の内容で走るため、ローカルの version とズレないよう
# 「作業ツリーがクリーン」かつ「HEAD == origin/main」を事前に確認する。
if [ -n "$(git status --porcelain)" ]; then
  echo "Error: working tree is not clean. Commit and push first." >&2
  exit 1
fi
git fetch origin main
if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
  echo "Error: local HEAD does not match origin/main. Push (or pull) first." >&2
  exit 1
fi

VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
echo "Triggering release build for v${VERSION} ..."

# workflow_dispatch は run ID を返さないため、トリガー前の最新 run ID を控えておき、
# それと異なる ID (= 今回の run) が現れるまでポーリングして拾う。
PREV_RUN_ID=$(gh run list --workflow=release.yml --branch main --limit 1 \
  --json databaseId --jq '.[0].databaseId // ""')

gh workflow run release.yml --ref main

RUN_ID=""
for _ in $(seq 1 15); do
  sleep 2
  RUN_ID=$(gh run list --workflow=release.yml --branch main --limit 1 \
    --json databaseId --jq '.[0].databaseId // ""')
  if [ -n "${RUN_ID}" ] && [ "${RUN_ID}" != "${PREV_RUN_ID}" ]; then
    break
  fi
  RUN_ID=""
done
if [ -z "${RUN_ID}" ]; then
  echo "Error: could not find the triggered workflow run." >&2
  exit 1
fi
echo "Watching run ${RUN_ID} ..."
gh run watch "${RUN_ID}" --exit-status

echo "Done: https://github.com/ytyng/arcvault/releases/tag/v${VERSION}"
