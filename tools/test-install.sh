#!/usr/bin/env bash
# 用本地 bin 目录模拟远程安装

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_DIR="$(mktemp -d)"
SCRIPT_DIR="$(mktemp -d)"

info() { printf '\033[1;36m%s\033[0m\n' "$*"; }
ok() { printf '\033[1;32m%s\033[0m\n' "$*"; }

info "检查 install.sh 语法"
bash -n "$ROOT/install.sh"

info "模拟远程安装"
cp "$ROOT/install.sh" "$SCRIPT_DIR/install.sh"
CLASH_INSTALL_BASE_URL="file://$ROOT" CLASH_INSTALL_DIR="$INSTALL_DIR" bash "$SCRIPT_DIR/install.sh"

info "验证安装产物"
"$INSTALL_DIR/clash" version
file "$INSTALL_DIR/clash"

ok "安装验证通过: $INSTALL_DIR/clash"
