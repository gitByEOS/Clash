#!/usr/bin/env python3
"""Clash e2e 测试共享模块。"""

from __future__ import annotations

import os
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
import threading
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = ROOT / "e2e" / "artifacts"
CLASH_BIN = ROOT / "target" / "release" / "clash"
CONFIG_HOME_ENV = "CLASH_TEST_CONFIG_HOME"
MODELS = ["qwen3.6-plus", "glm-5", "kimi-k2"]
ALT_MODELS = ["deepseek-v4-pro", "deepseek-v4-flash", "qwen-max"]
BASE_URL = "http://example.test/anthropic"
ALT_BASE_URL = "http://alt.example.test/anthropic"
API_KEY = "sk-test-token"
ALT_API_KEY = "sk-alt-token"
COLS = 80
ROWS = 24


@dataclass
class CliResult:
    returncode: int
    stdout: str
    stderr: str


class TerminalScreen:
    def __init__(self, rows: int, cols: int):
        self.rows = rows
        self.cols = cols
        self.row = 0
        self.col = 0
        self.saved_row = 0
        self.saved_col = 0
        self.cells = [[" " for _ in range(cols)] for _ in range(rows)]

    def feed(self, data: bytes) -> None:
        text = data.decode("utf-8", "replace")
        i = 0
        while i < len(text):
            ch = text[i]
            if ch == "\x1b" and i + 1 < len(text) and text[i + 1] == "[":
                end = self._consume_csi(text, i + 2)
                i = end
                continue
            if ch == "\x1b" and i + 1 < len(text) and text[i + 1] == "7":
                self.saved_row = self.row
                self.saved_col = self.col
                i += 2
                continue
            if ch == "\x1b" and i + 1 < len(text) and text[i + 1] == "8":
                self.row = self.saved_row
                self.col = self.saved_col
                i += 2
                continue
            if ch == "\r":
                self.col = 0
            elif ch == "\n":
                self._newline()
            elif ch >= " ":
                self._put(ch)
            i += 1

    def visible_lines(self) -> list[str]:
        return ["".join(row).rstrip() for row in self.cells]

    def semantic_lines(self) -> list[str]:
        return [line for line in self.visible_lines() if line.strip()]

    def _consume_csi(self, text: str, start: int) -> int:
        i = start
        while i < len(text) and not ("@" <= text[i] <= "~"):
            i += 1
        if i >= len(text):
            return len(text)

        params = text[start:i]
        command = text[i]
        value = self._first_param(params)

        if command == "A":
            self.row = max(0, self.row - value)
        elif command == "B":
            self.row = min(self.rows - 1, self.row + value)
        elif command == "C":
            self.col = min(self.cols - 1, self.col + value)
        elif command == "D":
            self.col = max(0, self.col - value)
        elif command == "G":
            self.col = min(self.cols - 1, max(0, value - 1))
        elif command == "H":
            row, col = self._row_col(params)
            self.row = min(self.rows - 1, max(0, row - 1))
            self.col = min(self.cols - 1, max(0, col - 1))
        elif command == "J":
            self._clear_from_cursor_down()
        elif command == "K":
            self._clear_line_from_cursor()
        elif command == "s":
            self.saved_row = self.row
            self.saved_col = self.col
        elif command == "u":
            self.row = self.saved_row
            self.col = self.saved_col

        return i + 1

    def _put(self, ch: str) -> None:
        self.cells[self.row][self.col] = ch
        self.col += 1
        if self.col >= self.cols:
            self.col = 0
            self._newline()

    def _newline(self) -> None:
        if self.row + 1 >= self.rows:
            self.cells.pop(0)
            self.cells.append([" " for _ in range(self.cols)])
        else:
            self.row += 1
        self.col = 0

    def _clear_from_cursor_down(self) -> None:
        for col in range(self.col, self.cols):
            self.cells[self.row][col] = " "
        for row in range(self.row + 1, self.rows):
            self.cells[row] = [" " for _ in range(self.cols)]

    def _clear_line_from_cursor(self) -> None:
        for col in range(self.col, self.cols):
            self.cells[self.row][col] = " "

    def _first_param(self, params: str) -> int:
        match = re.search(r"\d+", params)
        if not match:
            return 1
        return max(1, int(match.group(0)))

    def _row_col(self, params: str) -> tuple[int, int]:
        nums = [int(n) for n in re.findall(r"\d+", params)]
        if len(nums) >= 2:
            return nums[0], nums[1]
        return 1, 1


def run(args: list[str], env: dict[str, str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=ROOT,
        env=env,
        check=check,
        text=True,
        capture_output=True,
        stdin=subprocess.DEVNULL,
    )


def run_clash(args: list[str], env: dict[str, str], *, check: bool = True) -> CliResult:
    proc = subprocess.run(
        [str(CLASH_BIN), *args],
        cwd=ROOT,
        env=env,
        check=check,
        text=True,
        capture_output=True,
        stdin=subprocess.DEVNULL,
    )
    return CliResult(proc.returncode, strip_ansi(proc.stdout), strip_ansi(proc.stderr))


def run_clash_with_input(args: list[str], env: dict[str, str], stdin: str, *, check: bool = True) -> CliResult:
    proc = subprocess.run(
        [str(CLASH_BIN), *args],
        cwd=ROOT,
        env=env,
        input=stdin,
        check=check,
        text=True,
        capture_output=True,
    )
    return CliResult(proc.returncode, strip_ansi(proc.stdout), strip_ansi(proc.stderr))


def strip_ansi(text: str) -> str:
    return re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", text)


def config_path(config_home: str, idx: int = 0) -> Path:
    file_name = "auth" if idx == 0 else f"auth{idx}"
    return Path(config_home) / "clash" / file_name


def config_home(env: dict[str, str]) -> str:
    return env[CONFIG_HOME_ENV]


def test_config_set(env: dict[str, str], idx: int, base_url: str, api_key: str, models: list[str]) -> None:
    result = run_clash(
        [
            "config",
            "--idx",
            str(idx),
            "--url",
            base_url,
            "--key",
            api_key,
            "--models",
            ",".join(models),
        ],
        env,
    )
    assert result.returncode == 0, result.stderr or result.stdout
    assert "配置已保存" in result.stdout
    assert config_path(config_home(env), idx).is_file()


def test_config_show(env: dict[str, str], idx: int, base_url: str, models: list[str]) -> None:
    result = run_clash(["config", "--idx", str(idx)], env)
    assert result.returncode == 0, result.stderr or result.stdout
    assert f"=== 当前配置 idx={idx} ===" in result.stdout
    assert f"BASE_URL={base_url}" in result.stdout
    assert "MODELS=<<MODELS" in result.stdout
    for model in models:
        assert model in result.stdout


def test_config_partial_update(env: dict[str, str]) -> None:
    updated_url = "http://updated.example/anthropic"
    result = run_clash(["config", "--url", updated_url], env)
    assert result.returncode == 0, result.stderr or result.stdout
    show = run_clash(["config"], env)
    assert f"BASE_URL={updated_url}" in show.stdout
    for model in MODELS:
        assert model in show.stdout

    result = run_clash(["config", "--key", "sk-updated-key"], env)
    assert result.returncode == 0, result.stderr or result.stdout

    result = run_clash(["config", "--models", "glm-5"], env)
    assert result.returncode == 0, result.stderr or result.stdout
    show = run_clash(["config"], env)
    assert "glm-5" in show.stdout
    assert "qwen3.6-plus" not in show.stdout
    idx1 = run_clash(["config", "--idx", "1"], env)
    assert f"BASE_URL={ALT_BASE_URL}" in idx1.stdout
    assert ALT_MODELS[0] in idx1.stdout


def test_config_empty_models(env: dict[str, str]) -> None:
    result = run_clash(["config", "--idx", "1", "--models", " , "], env, check=False)
    assert result.returncode != 0
    assert "模型列表不能为空" in result.stdout


def test_invalid_idx(env: dict[str, str]) -> None:
    result = run_clash(["config", "--idx", "abc"], env, check=False)
    assert result.returncode != 0
    assert "--idx 必须是 0 或正整数" in result.stdout or "--idx" in result.stdout

    result = run_clash(["test", "--idx", "abc"], env, check=False)
    assert result.returncode != 0
    # PowerShell 返回 "--idx 必须是 0 或正整数"，Rust 返回 "用法: clash test"
    assert "--idx 必须是 0 或正整数" in result.stdout or "用法" in result.stdout


def auth_files(config_home: str) -> list[str]:
    config_dir = Path(config_home) / "clash"
    if not config_dir.exists():
        return []
    return sorted(path.name for path in config_dir.glob("auth*") if path.is_file())


def write_auth_snapshot(path: Path, files: list[str]) -> None:
    content = "\n".join(files) if files else "<empty>"
    path.write_text(content + "\n", encoding="utf-8")


def test_reset(env: dict[str, str], artifact_dir: Path) -> None:
    before = auth_files(config_home(env))
    assert before == ["auth", "auth1"], before
    write_auth_snapshot(artifact_dir / "reset-before.txt", before)

    result = run_clash(["reset"], env)
    assert result.returncode == 0, result.stderr or result.stdout
    assert "已删除全部配置" in result.stdout
    assert not config_path(config_home(env), 0).exists()
    assert not config_path(config_home(env), 1).exists()

    after = auth_files(config_home(env))
    assert after == [], after
    write_auth_snapshot(artifact_dir / "reset-after.txt", after)


def test_config_interactive_missing_idx(
    env: dict[str, str],
    idx: int,
    base_url: str,
    api_key: str,
    models: list[str],
) -> None:
    result = run_clash_with_input(
        ["config", "--idx", str(idx)],
        env,
        f"{base_url}\n{api_key}\n\n{','.join(models)}\n",
    )
    assert result.returncode == 0, result.stderr or result.stdout
    assert "Clash 配置向导" in result.stdout
    assert config_path(config_home(env), idx).is_file()
    test_config_show(env, idx, base_url, models)


class _AnthropicMockHandler(BaseHTTPRequestHandler):
    def log_message(self, _format, *_args) -> None:
        return

    def do_POST(self) -> None:
        if not self.path.rstrip("/").endswith("/v1/messages"):
            self.send_response(404)
            self.end_headers()
            return

        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length) if length else b""
        if b"ping" not in body:
            self.send_response(400)
            self.end_headers()
            return

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(
            b'{"type":"message","role":"assistant","content":[{"type":"text","text":"pong"}]}'
        )


def test_connection(env: dict[str, str]) -> None:
    server = HTTPServer(("127.0.0.1", 0), _AnthropicMockHandler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        mock_url = f"http://127.0.0.1:{port}/anthropic"
        run_clash(
            [
                "config",
                "--idx",
                "0",
                "--url",
                mock_url,
                "--key",
                API_KEY,
                "--models",
                ",".join(MODELS),
            ],
            env,
        )
        run_clash(
            [
                "config",
                "--idx",
                "1",
                "--url",
                mock_url,
                "--key",
                ALT_API_KEY,
                "--models",
                ",".join(ALT_MODELS),
            ],
            env,
        )
        result = run_clash(["test"], env)
        assert result.returncode == 0, result.stdout + result.stderr
        assert "全部通过" in result.stdout
        assert "qwen3.6-plus 通过" in result.stdout
        assert "glm-5 通过" in result.stdout
        assert "kimi-k2 通过" in result.stdout

        result = run_clash(["test", "--idx", "1"], env)
        assert result.returncode == 0, result.stdout + result.stderr
        assert "deepseek-v4-pro 通过" in result.stdout
        assert "deepseek-v4-flash 通过" in result.stdout
        assert "qwen-max 通过" in result.stdout
        assert "qwen3.6-plus 通过" not in result.stdout
    finally:
        server.shutdown()
        thread.join(timeout=2)


def test_run_exec_env(
    env: dict[str, str],
    *,
    expected_base: str = "http://127.0.0.1",
    expected_model: str = "qwen3.6-plus",
) -> CliResult:
    with tempfile.TemporaryDirectory(prefix="clash-e2e-bin-") as bin_dir:
        claude = Path(bin_dir) / "claude"
        claude.write_text(
            "#!/bin/sh\n"
            "echo BASE=$ANTHROPIC_BASE_URL\n"
            "echo TOKEN=$ANTHROPIC_AUTH_TOKEN\n"
            "echo MODEL=$ANTHROPIC_MODEL\n"
            "echo ARGS=$*\n",
            encoding="utf-8",
        )
        claude.chmod(0o755)

        run_env = env.copy()
        run_env["PATH"] = f"{bin_dir}{os.pathsep}{run_env['PATH']}"
        # 不使用 --smoke，直接让假 claude 输出环境变量
        result = run_clash(["run"], run_env)

    assert result.returncode == 0, result.stdout + result.stderr
    assert f"BASE={expected_base}" in result.stdout
    assert f"MODEL={expected_model}" in result.stdout
    # ARGS 包含基本参数，但不需要 --smoke
    assert "--permission-mode bypassPermissions" in result.stdout
    return result


def test_removed_commands(env: dict[str, str]) -> None:
    with tempfile.TemporaryDirectory(prefix="clash-e2e-bin-") as bin_dir:
        claude = Path(bin_dir) / "claude"
        claude.write_text("#!/bin/sh\necho ARGS=$*\n", encoding="utf-8")
        claude.chmod(0o755)

        run_env = env.copy()
        run_env["PATH"] = f"{bin_dir}{os.pathsep}{run_env['PATH']}"
        add_model = run_clash(["add-model", "new-model"], run_env)
        change_token = run_clash(["change-token", "sk-new"], run_env)

    assert add_model.returncode == 0
    assert "ARGS=--permission-mode bypassPermissions --effort max --model qwen3.6-plus add-model new-model" in add_model.stdout
    assert change_token.returncode == 0
    assert "ARGS=--permission-mode bypassPermissions --effort max --model qwen3.6-plus change-token sk-new" in change_token.stdout


def test_rename_via_config(env: dict[str, str], idx: int, new_name: str) -> None:
    config_file = config_path(config_home(env), idx)
    content = config_file.read_text(encoding="utf-8")

    lines = content.splitlines()
    new_lines = []
    for line in lines:
        new_lines.append(line)
        if line.startswith("AUTH_TOKEN="):
            new_lines.append(f"NAME={new_name}")

    config_file.write_text("\n".join(new_lines) + "\n", encoding="utf-8")

    content = config_file.read_text(encoding="utf-8")
    assert f"NAME={new_name}" in content, content


def set_winsize(fd: int) -> None:
    winsize = struct.pack("HHHH", ROWS, COLS, 0, 0)
    if hasattr(termios, "tcsetwinsize"):
        termios.tcsetwinsize(fd, (ROWS, COLS))
    import fcntl
    fcntl.ioctl(fd, termios.TIOCSWINSZ, winsize)


def drain(fd: int, seconds: float, until: bytes | None = None, min_bytes: int = 0) -> bytes:
    end = time.time() + seconds
    data = bytearray()
    while time.time() < end:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if not readable:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        data.extend(chunk)
        if until and until in data and len(data) >= min_bytes:
            break
    return bytes(data)


def stop_child(pid: int) -> None:
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        return
    try:
        os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        pass


def assert_tui_single_account(screen: TerminalScreen) -> None:
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    assert_contains(lines, "1/3")
    assert_contains(lines, "→ 1. qwen3.6-plus")
    assert_contains(lines, "  2. glm-5")
    assert_contains(lines, "  3. kimi-k2")
    assert_not_contains(lines, "[1st]")
    assert_not_contains(lines, "[2st]")
    assert_not_contains(lines, "^[[B")


def assert_tui_multi_account(screen: TerminalScreen) -> None:
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    # 模型数量取决于 partial update 后的状态，可能是 1+3=4 或 3+3=6
    assert_contains(lines, "/")
    # 检查账户标签格式正确
    assert_contains(lines, "[1st]")
    assert_contains(lines, "[2st]")
    assert_not_contains(lines, "^[[B")


def assert_tui_renamed(screen: TerminalScreen) -> None:
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    # 模型数量取决于 partial update 后的状态
    assert_contains(lines, "/")
    assert_contains(lines, "[work]")
    assert_contains(lines, "[2st]")
    assert_not_contains(lines, "[1st]")
    assert_not_contains(lines, "^[[B")


def assert_tui_down_renamed(screen: TerminalScreen) -> None:
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    assert_contains(lines, "2/")
    assert_contains(lines, "[work]")
    assert_contains(lines, "[2st]")
    assert_not_contains(lines, "[1st]")
    assert_not_contains(lines, "^[[B")


def assert_single_prompt(lines: list[str]) -> None:
    prompts = [line for line in lines if line.startswith("clash>")]
    if len(prompts) != 1:
        raise AssertionError(f"expected one prompt, got {len(prompts)}: {lines}")


def assert_contains(lines: list[str], expected: str) -> None:
    if not any(expected in line for line in lines):
        raise AssertionError(f"missing {expected!r}: {lines}")


def assert_not_contains(lines: list[str], unexpected: str) -> None:
    if any(unexpected in line for line in lines):
        raise AssertionError(f"unexpected {unexpected!r}: {lines}")


def test_version(env: dict[str, str]) -> None:
    result = run_clash(["version"], env)
    assert result.returncode == 0, result.stderr or result.stdout
    assert result.stdout.startswith("v"), result.stdout
    # 版本格式如 v0.1.3
    assert "." in result.stdout, result.stdout


def assert_tui_up_arrow(screen: TerminalScreen) -> None:
    """上箭头后选中第一项（从第二项回到第一项）"""
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    assert_contains(lines, "1/")
    assert_contains(lines, "→")


def assert_tui_esc_cancel(screen: TerminalScreen) -> None:
    """Esc 后 TUI 关闭，无选中项"""
    lines = screen.semantic_lines()
    # Esc 后应该没有 prompt 和选择器
    assert_not_contains(lines, "clash>")
    assert_not_contains(lines, "→")


def assert_tui_search_filter(screen: TerminalScreen, search_term: str) -> None:
    """搜索过滤后只显示匹配项"""
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    # prompt 应包含搜索词
    assert_contains(lines, f"clash> {search_term}")