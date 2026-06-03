#!/usr/bin/env bash
# 编译 release 并发布到 bin/clash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE="$ROOT/target/release/clash"
OUT="$ROOT/bin/clash"

info() { printf '\033[1;36m%s\033[0m\n' "$*"; }
ok() { printf '\033[1;32m%s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*"; }
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

build_release() {
    local cargo="$1"

    if "$cargo" build --release; then
        return 0
    fi

    if [[ -n "${RUSTUP_TOOLCHAIN:-}" ]] || ! command -v rustup >/dev/null 2>&1; then
        return 1
    fi

    warn "未配置默认 toolchain，改用 stable 重试"
    RUSTUP_TOOLCHAIN=stable "$cargo" build --release
}

publish_binary() {
    if [[ ! -f "$SOURCE" ]]; then
        fail "未找到构建产物: $SOURCE"
        return 1
    fi

    mkdir -p "$ROOT/bin"
    cp "$SOURCE" "$OUT"
    chmod +x "$OUT"
    ok "已发布: $OUT"
}

main() {
    local cargo
    cargo="$(find_cargo)"

    cd "$ROOT"
    info "编译 release..."
    build_release "$cargo"
    info "发布到 bin/clash"
    publish_binary
}

main "$@"
