#!/usr/bin/env bash
# 把 GitHub Actions artifacts 拉回本地 bin；传 run 时才触发新构建

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKFLOW="publish-bin.yml"
COMMAND="${1:-fetch}"
BRANCH="${2:-$(git -C "$ROOT" branch --show-current)}"
DOWNLOAD_DIR="$(mktemp -d)"
POLL_SECONDS=10

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
    if [[ "$COMMAND" == "run" ]]; then
        gh run list \
            --workflow "$WORKFLOW" \
            --branch "$BRANCH" \
            --event workflow_dispatch \
            --limit 1 \
            --json databaseId \
            --jq '.[0].databaseId'
        return
    fi

    gh run list \
        --workflow "$WORKFLOW" \
        --limit 1 \
        --json databaseId \
        --jq '.[0].databaseId'
}

local_asset() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os:$arch" in
        Darwin:x86_64) printf 'clash-x86_64-apple-darwin' ;;
        Darwin:arm64|Darwin:aarch64) printf 'clash-aarch64-apple-darwin' ;;
        Linux:x86_64) printf 'clash-x86_64-unknown-linux-gnu' ;;
        Linux:arm64|Linux:aarch64) printf 'clash-aarch64-unknown-linux-gnu' ;;
        *)
            fail "暂不支持当前平台: $os $arch"
            exit 1
            ;;
    esac
}

required_remote_assets() {
    local local_name="$1"
    local asset

    for asset in \
        clash-x86_64-apple-darwin \
        clash-aarch64-apple-darwin \
        clash-x86_64-unknown-linux-gnu \
        clash-aarch64-unknown-linux-gnu \
        clash-x86_64-pc-windows-msvc.exe
    do
        if [[ "$asset" != "$local_name" ]]; then
            printf '%s\n' "$asset"
        fi
    done
}

artifact_exists() {
    local run_id="$1"
    local name="$2"
    local repo

    repo="$(gh repo view --json nameWithOwner --jq '.nameWithOwner')"
    gh api "repos/$repo/actions/runs/$run_id/artifacts" \
        --jq ".artifacts[]?.name" | grep -Fxq "$name"
}

wait_for_remote_assets() {
    local run_id="$1"
    shift
    local asset missing status

    while true; do
        missing=0
        for asset in "$@"; do
            if ! artifact_exists "$run_id" "$asset"; then
                missing=1
                break
            fi
        done

        if [[ "$missing" -eq 0 ]]; then
            return 0
        fi

        status="$(gh run view "$run_id" --json status,conclusion --jq '.status + ":" + (.conclusion // "")')"
        if [[ "$status" == completed:* ]]; then
            fail "远端构建已结束，但 artifacts 不完整: $status"
            return 1
        fi

        info "等待远端 artifacts: $*"
        sleep "$POLL_SECONDS"
    done
}

download_artifacts() {
    local run_id="$1"
    shift
    local asset

    for asset in "$@"; do
        gh run download "$run_id" -n "$asset" -D "$DOWNLOAD_DIR"
    done
    mkdir -p "$ROOT/bin"
    find "$DOWNLOAD_DIR" -type f -name 'clash-*' -exec cp {} "$ROOT/bin/" \;
    find "$ROOT/bin" -type f ! -name '*.exe' -name 'clash-*' -exec chmod +x {} \;
}

main() {
    require_gh
    if [[ "$COMMAND" != "fetch" && "$COMMAND" != "run" ]]; then
        fail "用法: tools/fetch-bin.sh [run] [branch]"
        exit 1
    fi

    local local_name run_id
    local remote_assets=()
    local asset
    local_name="$(local_asset)"

    info "本机构建 ${local_name}"
    "$ROOT/tools/build.sh"

    if [[ "$COMMAND" == "run" ]]; then
        info "触发 ${WORKFLOW}，分支: ${BRANCH}"
        gh workflow run "$WORKFLOW" --ref "$BRANCH" -f publish=false

        info "等待 workflow run 出现"
        sleep 5
    else
        info "不触发 workflow，使用最近一次 ${WORKFLOW} run"
    fi


    run_id="$(latest_run_id)"
    if [[ -z "$run_id" || "$run_id" == "null" ]]; then
        fail "未找到可用的 workflow run"
        exit 1
    fi

    while IFS= read -r asset; do
        remote_assets+=("$asset")
    done < <(required_remote_assets "$local_name")

    info "等待远端 artifacts: ${remote_assets[*]}"
    wait_for_remote_assets "$run_id" "${remote_assets[@]}"

    info "下载 artifacts"
    download_artifacts "$run_id" "${remote_assets[@]}"

    info "本地 bin 产物"
    file "$ROOT"/bin/clash-*
    ok "已更新本地 bin，不会自动 commit 或 push"
}

main "$@"
