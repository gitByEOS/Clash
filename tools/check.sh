#!/usr/bin/env bash
# 发版前统一检查入口

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

info() { printf '\033[1;36m%s\033[0m\n' "$*"; }
ok() { printf '\033[1;32m%s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m%s\033[0m\n' "$*" >&2; }

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

    info "格式化 Rust 代码"
    "$cargo" fmt

    info "运行单元测试"
    "$cargo" test

    info "运行 Clippy"
    "$cargo" clippy --all-targets -- -D warnings

    info "构建 release"
    "$cargo" build --release

    ok "全部检查通过"
}

main "$@"
