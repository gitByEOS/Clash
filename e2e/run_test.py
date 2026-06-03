#!/usr/bin/env python3
"""Clash CLI 与 TUI 端到端测试。"""

from __future__ import annotations

import os
import re
import select
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = ROOT / "e2e" / "artifacts"
CLASH_BIN = ROOT / "target" / "debug" / "clash"
MODELS = ["qwen3.6-plus", "glm-5"]
BASE_URL = "http://example.test/anthropic"
API_KEY = "sk-test-token"
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


def main() -> int:
    stamp = datetime.now().strftime("%y%m%d-%H%M%S")
    artifact_dir = ARTIFACT_ROOT / stamp
    artifact_dir.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env["CLASH_SKIP_AUTO_TEST"] = "1"
    with tempfile.TemporaryDirectory(prefix="clash-e2e-") as config_home:
        env["XDG_CONFIG_HOME"] = config_home
        log("build")
        build(env)

        results: list[str] = []

        log("test config set")
        test_config_set(env)
        results.append("- config set 写入成功")

        log("test config show")
        test_config_show(env)
        results.append("- config show 展示 BASE_URL 与模型")

        log("test config partial update")
        test_config_partial_update(env)
        results.append("- config 支持单独更新 --url / --key / --models")

        log("test config empty models")
        test_config_empty_models(env)
        results.append("- config --models 空列表失败")

        log("test reset")
        test_reset(env)
        results.append("- reset 删除配置")

        log("test config show after reset")
        test_config_show_unconfigured(env)
        results.append("- reset 后 config show 提示未配置")

        log("test config set again for tui")
        test_config_set(env)

        log("test connection")
        test_connection(env)
        results.append("- clash test 对 /v1/messages 连通测试成功")

        log("test tui default run")
        initial_raw, initial_screen = capture_frame(env, ["clash"])
        assert_tui_initial(initial_screen)
        results.append("- clash 首帧选中第一项")

        log("test tui run subcommand")
        run_raw, run_screen = capture_frame(env, ["clash", "run"])
        assert_tui_initial(run_screen)
        results.append("- clash run 与 clash 等价")

        log("test tui down arrow")
        down_raw, down_screen = capture_frame(env, ["clash"], keys=[b"\x1b[B"])
        assert_tui_down(down_screen)
        results.append("- 下箭头后选中第二项且不重复刷屏")

        write_artifacts(
            artifact_dir,
            initial_raw,
            initial_screen,
            down_raw,
            down_screen,
            run_raw,
            run_screen,
        )
        write_report(artifact_dir, results, initial_screen, down_screen, run_screen)

    print(f"E2E passed: {artifact_dir}")
    return 0


def build(env: dict[str, str]) -> None:
    if CLASH_BIN.exists():
        return

    cargo = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / "cargo")
    if not Path(cargo).exists():
        raise RuntimeError("未找到 cargo，且 target/debug/clash 不存在")

    run([cargo, "build"], env)
    if not CLASH_BIN.exists():
        raise RuntimeError(f"构建后未找到 {CLASH_BIN}")


def run(args: list[str], env: dict[str, str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=ROOT,
        env=env,
        check=check,
        text=True,
        capture_output=True,
    )


def run_clash(args: list[str], env: dict[str, str], *, check: bool = True) -> CliResult:
    proc = run([str(CLASH_BIN), *args], env, check=check)
    return CliResult(proc.returncode, strip_ansi(proc.stdout), strip_ansi(proc.stderr))


def strip_ansi(text: str) -> str:
    return re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", text)


def log(message: str) -> None:
    print(f"[e2e] {message}", flush=True)


def config_path(config_home: str) -> Path:
    return Path(config_home) / "clash" / "auth"


def test_config_set(env: dict[str, str]) -> None:
    result = run_clash(
        [
            "config",
            "--url",
            BASE_URL,
            "--key",
            API_KEY,
            "--models",
            ",".join(MODELS),
        ],
        env,
    )
    assert result.returncode == 0, result.stderr or result.stdout
    assert "配置已保存" in result.stdout
    assert config_path(env["XDG_CONFIG_HOME"]).is_file()


def test_config_show(env: dict[str, str]) -> None:
    result = run_clash(["config"], env)
    assert result.returncode == 0, result.stderr or result.stdout
    assert f"BASE_URL={BASE_URL}" in result.stdout
    assert "MODELS=<<MODELS" in result.stdout
    for model in MODELS:
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


def test_config_empty_models(env: dict[str, str]) -> None:
    result = run_clash(["config", "--models", " , "], env, check=False)
    assert result.returncode != 0
    assert "模型列表不能为空" in result.stdout


def test_reset(env: dict[str, str]) -> None:
    result = run_clash(["reset"], env)
    assert result.returncode == 0, result.stderr or result.stdout
    assert "已删除配置" in result.stdout
    assert not config_path(env["XDG_CONFIG_HOME"]).exists()


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
                "--url",
                mock_url,
                "--key",
                API_KEY,
            ],
            env,
        )
        result = run_clash(["test"], env)
        assert result.returncode == 0, result.stdout + result.stderr
        assert "全部通过" in result.stdout
        assert "qwen3.6-plus 通过" in result.stdout
        assert "glm-5 通过" in result.stdout
    finally:
        server.shutdown()
        thread.join(timeout=2)


def test_config_show_unconfigured(env: dict[str, str]) -> None:
    result = run_clash(["config"], env, check=False)
    assert result.returncode != 0
    assert "未配置" in result.stdout


def capture_frame(
    env: dict[str, str],
    cmd: list[str],
    keys: list[bytes] | None = None,
) -> tuple[bytes, TerminalScreen]:
    keys = keys or []
    pid, master = os.forkpty()
    if pid == 0:
        os.chdir(ROOT)
        os.execve(str(CLASH_BIN), cmd, env)

    set_winsize(master)
    raw = bytearray()
    raw.extend(drain(master, 3.0, until=b"cursor>", min_bytes=400))

    for key in keys:
        os.write(master, key)
        raw.extend(drain(master, 0.5))

    stop_child(pid)
    raw.extend(drain(master, 0.2))
    os.close(master)

    screen = TerminalScreen(ROWS, COLS)
    screen.feed(bytes(raw))
    return bytes(raw), screen


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


def assert_tui_initial(screen: TerminalScreen) -> None:
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    assert_contains(lines, "1/2")
    assert_contains(lines, "→ model  qwen3.6-plus")
    assert_contains(lines, "  model  glm-5")
    assert_not_contains(lines, "^[[B")


def assert_tui_down(screen: TerminalScreen) -> None:
    lines = screen.semantic_lines()
    assert_single_prompt(lines)
    assert_contains(lines, "2/2")
    assert_contains(lines, "  model  qwen3.6-plus")
    assert_contains(lines, "→ model  glm-5")
    assert_not_contains(lines, "^[[B")


def assert_single_prompt(lines: list[str]) -> None:
    prompts = [line for line in lines if line.startswith("cursor>")]
    if len(prompts) != 1:
        raise AssertionError(f"expected one prompt, got {len(prompts)}: {lines}")


def assert_contains(lines: list[str], expected: str) -> None:
    if not any(expected in line for line in lines):
        raise AssertionError(f"missing {expected!r}: {lines}")


def assert_not_contains(lines: list[str], unexpected: str) -> None:
    if any(unexpected in line for line in lines):
        raise AssertionError(f"unexpected {unexpected!r}: {lines}")


def write_artifacts(
    artifact_dir: Path,
    initial_raw: bytes,
    initial_screen: TerminalScreen,
    down_raw: bytes,
    down_screen: TerminalScreen,
    run_raw: bytes,
    run_screen: TerminalScreen,
) -> None:
    (artifact_dir / "initial.raw").write_bytes(initial_raw)
    (artifact_dir / "after-down.raw").write_bytes(down_raw)
    (artifact_dir / "run.raw").write_bytes(run_raw)
    (artifact_dir / "initial.txt").write_text(render_text(initial_screen), encoding="utf-8")
    (artifact_dir / "after-down.txt").write_text(render_text(down_screen), encoding="utf-8")
    (artifact_dir / "run.txt").write_text(render_text(run_screen), encoding="utf-8")
    write_png(artifact_dir / "initial.png", initial_screen)
    write_png(artifact_dir / "after-down.png", down_screen)
    write_png(artifact_dir / "run.png", run_screen)


def render_text(screen: TerminalScreen) -> str:
    return "\n".join(screen.semantic_lines()) + "\n"


def write_png(path: Path, screen: TerminalScreen) -> None:
    try:
        from PIL import Image, ImageDraw, ImageFont
    except Exception:
        return

    lines = screen.semantic_lines()
    font = load_font(ImageFont)
    width = 900
    line_height = 28
    pad = 10
    height = max(1, len(lines)) * line_height + pad * 2
    image = Image.new("RGB", (width, height), (30, 30, 30))
    draw = ImageDraw.Draw(image)

    for idx, line in enumerate(lines):
        y = pad + idx * line_height
        draw_colored_line(draw, font, line, y)

    image.save(path)


def load_font(image_font):
    for font_path in [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Menlo.ttc",
    ]:
        if Path(font_path).exists():
            try:
                return image_font.truetype(font_path, 20)
            except Exception:
                pass
    return image_font.load_default()


def draw_colored_line(draw, font, line: str, y: int) -> None:
    if line.startswith("cursor>"):
        draw.text((10, y), "cursor>", fill=(86, 156, 214), font=font)
        draw.text((92, y), line[len("cursor>") :], fill=(212, 212, 212), font=font)
        return
    if line.startswith("→"):
        draw.text((10, y), "→", fill=(255, 0, 128), font=font)
        draw.text((30, y), line[1:], fill=(212, 212, 212), font=font)
        return
    draw.text((10, y), line, fill=(212, 212, 212), font=font)


def write_report(
    artifact_dir: Path,
    results: list[str],
    initial: TerminalScreen,
    down: TerminalScreen,
    run_screen: TerminalScreen,
) -> None:
    checklist = "\n".join(results)
    report = f"""# Clash E2E

## 覆盖项
{checklist}

## 产物
- `initial.txt` / `after-down.txt` / `run.txt`
- `initial.png` / `after-down.png` / `run.png`
- `initial.raw` / `after-down.raw` / `run.raw`

## 首帧
```text
{render_text(initial)}```

## clash run 首帧
```text
{render_text(run_screen)}```

## 下箭头后
```text
{render_text(down)}```
"""
    (artifact_dir / "report.md").write_text(report, encoding="utf-8")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as exc:
        print(f"command failed: {exc}", file=sys.stderr)
        if exc.stdout:
            print(exc.stdout, file=sys.stderr)
        if exc.stderr:
            print(exc.stderr, file=sys.stderr)
        raise SystemExit(exc.returncode)
    except AssertionError as exc:
        print(f"assertion failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
