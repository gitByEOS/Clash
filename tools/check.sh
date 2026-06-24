#!/usr/bin/env bash
# 发版前统一检查入口

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

info() { printf '\033[1;36m%s\033[0m\n' "$*"; }
ok() { printf '\033[1;32m%s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m%s\033[0m\n' "$*" >&2; }

run_step() {
    local title="$1"
    shift

    local log
    log="$(mktemp "${TMPDIR:-/tmp}/clash-check.XXXXXX")"
    info "$title"
    if "$@" >"$log" 2>&1; then
        rm -f "$log"
        return 0
    fi

    fail "$title 失败"
    printf '%s\n' "---- ${title} 输出 ----" >&2
    sed -n '1,200p' "$log" >&2
    rm -f "$log"
    return 1
}

find_cargo() {
    if command -v cargo >/dev/null 2>&1; then
        command -v cargo
        return 0
    fi

    if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
        printf '%s\n' "$HOME/.cargo/bin/cargo"
        return 0
    fi

    fail "未找到 cargo，请先安装 Rust"
    return 1
}

main() {
    local cargo
    cargo="$(find_cargo)"

    cd "$ROOT"

    run_step "格式化 Rust 代码" "$cargo" fmt
    run_step "运行单元测试" "$cargo" test
    run_step "运行 Clippy" "$cargo" clippy --all-targets -- -D warnings
    run_step "构建 release" "$cargo" build --release

    ok "全部检查通过"
}

main "$@"
