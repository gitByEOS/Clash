#!/usr/bin/env bash
# 触发 GitHub Actions 构建，并把 artifacts 拉回本地 bin

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKFLOW="publish-bin.yml"
BRANCH="${1:-$(git -C "$ROOT" branch --show-current)}"
DOWNLOAD_DIR="$(mktemp -d)"

info() { printf '\033[1;36m%s\033[0m\n' "$*"; }
ok() { printf '\033[1;32m%s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m%s\033[0m\n' "$*" >&2; }

require_gh() {
    if ! command -v gh >/dev/null 2>&1; then
        fail "未找到 gh，请先安装 GitHub CLI"
        exit 1
    fi
}

latest_run_id() {
    gh run list \
        --workflow "$WORKFLOW" \
        --branch "$BRANCH" \
        --event workflow_dispatch \
        --limit 1 \
        --json databaseId \
        --jq '.[0].databaseId'
}

download_artifacts() {
    local run_id="$1"

    gh run download "$run_id" -D "$DOWNLOAD_DIR"
    mkdir -p "$ROOT/bin"
    find "$DOWNLOAD_DIR" -type f -name 'clash-*' -exec cp {} "$ROOT/bin/" \;
    chmod +x "$ROOT"/bin/clash-*
}

main() {
    require_gh

    info "触发 ${WORKFLOW}，分支: ${BRANCH}"
    gh workflow run "$WORKFLOW" --ref "$BRANCH" -f publish=false

    info "等待 workflow run 出现"
    sleep 5

    local run_id
    run_id="$(latest_run_id)"
    if [[ -z "$run_id" || "$run_id" == "null" ]]; then
        fail "未找到刚触发的 workflow run"
        exit 1
    fi

    info "等待构建完成: $run_id"
    gh run watch "$run_id" --exit-status

    info "下载 artifacts"
    download_artifacts "$run_id"

    info "本地 bin 产物"
    file "$ROOT"/bin/clash-*
    ok "已更新本地 bin，不会自动 commit 或 push"
}

main "$@"
