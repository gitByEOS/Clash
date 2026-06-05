#!/usr/bin/env bash
# 安装 clash 到当前用户 PATH

set -euo pipefail

APP_NAME="clash"
INSTALL_DIR="${CLASH_INSTALL_DIR:-"$HOME/.local/bin"}"
TARGET="$INSTALL_DIR/$APP_NAME"
RAW_BASE_URL="${CLASH_INSTALL_BASE_URL:-https://raw.githubusercontent.com/gitByEOS/Clash/master}"

info() { printf '\033[1;36m%s\033[0m\n' "$*"; }
ok() { printf '\033[1;32m%s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*"; }
fail() { printf '\033[1;31m%s\033[0m\n' "$*" >&2; }

script_dir() {
    cd "$(dirname "$0")" >/dev/null 2>&1 && pwd
}

download_file() {
    local url="$1"
    local dest="$2"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$dest"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$dest" "$url"
    else
        fail "缺少 curl 或 wget，无法下载安装文件"
        return 1
    fi
}

local_binary() {
    local root="$1"
    local candidate
    for candidate in "$root/target/release/clash" "$root/target/debug/clash"; do
        if [[ -f "$candidate" ]]; then
            printf '%s' "$candidate"
            return 0
        fi
    done
    return 1
}

remote_binary_name() {
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

install_from_local_project() {
    local source="$1"
    cp "$source" "$TARGET"
}

install_from_remote() {
    local tmp_file binary_name
    binary_name="$(remote_binary_name)"
    tmp_file="$(mktemp)"
    download_file "$RAW_BASE_URL/bin/$binary_name" "$tmp_file"
    cp "$tmp_file" "$TARGET"
    rm -f "$tmp_file"
}

ensure_path_hint() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return 0 ;;
    esac

    warn "$INSTALL_DIR 不在 PATH 中"
    warn "请把下面一行加入你的 shell 配置："
    printf 'export PATH="%s:$PATH"\n' "$INSTALL_DIR"
}

main() {
    mkdir -p "$INSTALL_DIR"

    local local_source=""
    local root=""
    if [[ -n "${BASH_SOURCE[0]:-}" ]]; then
        root="$(script_dir)"
        if [[ -f "$root/Cargo.toml" ]]; then
            local_source="$(local_binary "$root")" || true
            if [[ -z "$local_source" ]]; then
                fail "本地未找到 clash，请先执行: cargo build --release"
                exit 1
            fi
        fi
    fi

    if [[ -n "$local_source" ]]; then
        info "使用本地构建产物安装 clash"
        install_from_local_project "$local_source"
    else
        info "从远程地址安装 clash"
        install_from_remote
    fi

    chmod +x "$TARGET"
    ok "clash 已安装到 $TARGET"
    ensure_path_hint
    ok "运行 clash 开始配置"
}

main "$@"
