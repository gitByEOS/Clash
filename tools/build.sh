#!/usr/bin/env bash
# 编译 release 并发布到 bin/<platform>

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE="$ROOT/target/release/clash"

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

binary_name() {
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
            return 1
            ;;
    esac
}

publish_binary() {
    local out
    if [[ ! -f "$SOURCE" ]]; then
        fail "未找到构建产物: $SOURCE"
        return 1
    fi

    out="$ROOT/bin/$(binary_name)"
    mkdir -p "$ROOT/bin"
    cp "$SOURCE" "$out"
    chmod +x "$out"
    ok "已发布: $out"
}

main() {
    local cargo
    cargo="$(find_cargo)"

    cd "$ROOT"
    info "编译 release..."
    build_release "$cargo"
    info "发布到 bin 平台产物"
    publish_binary
}

main "$@"
