#!/usr/bin/env python3
"""Clash CLI 与 TUI 端到端测试。"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import struct
import termios
import time
from datetime import datetime
from pathlib import Path

import run_test as shared


ROOT = shared.ROOT
ARTIFACT_ROOT = shared.ARTIFACT_ROOT
CLASH_BIN = shared.CLASH_BIN


def main() -> int:
    stamp = datetime.now().strftime("%y%m%d-%H%M%S")
    artifact_dir = ARTIFACT_ROOT / stamp
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_home = artifact_dir / "config-home"
    config_home.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env["CLASH_SKIP_AUTO_TEST"] = "1"
    env[shared.CONFIG_HOME_ENV] = str(config_home)
    env["XDG_CONFIG_HOME"] = str(config_home)
    log("build")
    build(env)

    results: list[str] = []

    log("test version")
    shared.test_version(env)
    results.append("- version 输出以 v 开头的版本号")

    log("test config idx0 set")
    shared.test_config_set(env, 0, shared.BASE_URL, shared.API_KEY, shared.MODELS)
    results.append("- config --idx 0 写入 auth")

    log("test config idx0 show")
    shared.test_config_show(env, 0, shared.BASE_URL, shared.MODELS)
    results.append("- config --idx 0 展示 BASE_URL 与模型")

    log("test single account run before idx1")
    single_raw, single_screen = capture_frame(env, ["clash"])
    shared.assert_tui_single_account(single_screen)
    results.append(f"- 创建 idx1 前单账户首帧显示 {len(shared.MODELS)} 个模型且无账户标签")

    log("test config idx1 set")
    shared.test_config_set(env, 1, shared.ALT_BASE_URL, shared.ALT_API_KEY, shared.ALT_MODELS)
    results.append("- config --idx 1 写入 auth1")

    log("test config idx1 show")
    shared.test_config_show(env, 1, shared.ALT_BASE_URL, shared.ALT_MODELS)
    results.append("- config --idx 1 展示独立账户")

    log("test config partial update")
    shared.test_config_partial_update(env)
    results.append("- config 支持单独更新 --url / --key / --models")

    log("test config empty models")
    shared.test_config_empty_models(env)
    results.append("- config --models 空列表失败")

    log("test invalid idx")
    shared.test_invalid_idx(env)
    results.append("- 非法 --idx 会失败")

    log("test reset")
    shared.test_reset(env, artifact_dir)
    results.append("- reset 真实删除 config-home 下全部账户配置")

    log("test config interactive after reset")
    shared.test_config_interactive_missing_idx(env, 0, shared.BASE_URL, shared.API_KEY, shared.MODELS)
    shared.test_config_interactive_missing_idx(env, 1, shared.ALT_BASE_URL, shared.ALT_API_KEY, shared.ALT_MODELS)
    results.append("- reset 后缺失 idx 进入引导并写入对应账户")

    log("test connection")
    shared.test_connection(env)
    results.append("- clash test 与 clash test --idx 1 连通测试成功")

    log("test run exec env")
    shared.test_run_exec_env(env)
    results.append("- clash run 按选中账户设置 Claude 环境变量")

    log("test removed commands")
    shared.test_removed_commands(env)
    results.append("- add-model / change-token 已不再作为命令入口")

    log("test multi account run")
    initial_raw, initial_screen = capture_frame(env, ["clash"])
    shared.assert_tui_multi_account(initial_screen)
    results.append(f"- 多账户 run 使用 1st / 2st 标签，共 {len(shared.MODELS) + len(shared.ALT_MODELS)} 个模型")

    log("test rename via config")
    shared.test_rename_via_config(env, 0, "work")
    results.append("- config 设置 NAME=work 后配置文件含 NAME 字段")

    log("test renamed account label")
    renamed_raw, renamed_screen = capture_frame(env, ["clash"])
    shared.assert_tui_renamed(renamed_screen)
    results.append("- 重命名后 TUI 显示 [work] 而非 [1st]")

    log("test tui run subcommand")
    run_raw, run_screen = capture_frame(env, ["clash", "run"])
    shared.assert_tui_renamed(run_screen)
    results.append("- clash run 与 clash 等价")

    log("test tui down arrow")
    down_raw, down_screen = capture_frame(env, ["clash"], keys=[b"\x1b[B"])
    shared.assert_tui_down_renamed(down_screen)
    results.append("- 下箭头后选中第二项且不重复刷屏")

    log("test tui up arrow")
    # 先下箭头选中第二项，再上箭头回到第一项
    up_raw, up_screen = capture_frame(env, ["clash"], keys=[b"\x1b[B", b"\x1b[A"])
    shared.assert_tui_up_arrow(up_screen)
    results.append("- 上箭头后选中第一项")

    log("test tui esc cancel")
    esc_raw, esc_screen = capture_frame(env, ["clash"], keys=[b"\x1b"])
    shared.assert_tui_esc_cancel(esc_screen)
    results.append("- Esc 后 TUI 关闭无选中项")

    log("test tui search filter")
    # 输入关键字过滤模型列表
    search_raw, search_screen = capture_frame(env, ["clash"], keys=[b"k", b"i", b"m"], until=b"kimi")
    shared.assert_tui_search_filter(search_screen, "kim")
    results.append("- 输入 kim 过滤后只显示匹配模型")

    write_artifacts(
        artifact_dir,
        single_raw,
        single_screen,
        initial_raw,
        initial_screen,
        renamed_raw,
        renamed_screen,
        down_raw,
        down_screen,
        up_raw,
        up_screen,
        esc_raw,
        esc_screen,
        search_raw,
        search_screen,
        run_raw,
        run_screen,
    )
    write_report(artifact_dir, results, single_screen, initial_screen, renamed_screen, down_screen, up_screen, esc_screen, search_screen, run_screen)

    print(f"E2E passed: {artifact_dir}")
    return 0


def build(env: dict[str, str]) -> None:
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / "cargo")
    if not Path(cargo).exists():
        raise RuntimeError("未找到 cargo")

    subprocess.run([cargo, "build", "--release"], cwd=ROOT, env=env, check=True, capture_output=True)
    if not CLASH_BIN.exists():
        raise RuntimeError(f"构建后未找到 {CLASH_BIN}")


def capture_frame(
    env: dict[str, str],
    cmd: list[str],
    keys: list[bytes] | None = None,
    until: bytes | None = None,
) -> tuple[bytes, shared.TerminalScreen]:
    keys = keys or []
    until = until or b"qwen-max"
    pid, master = os.forkpty()
    if pid == 0:
        os.chdir(ROOT)
        os.execve(str(CLASH_BIN), cmd, env)

    shared.set_winsize(master)
    raw = bytearray()
    raw.extend(shared.drain(master, 3.0, until=until, min_bytes=700))

    for key in keys:
        os.write(master, key)
        raw.extend(shared.drain(master, 0.5))

    shared.stop_child(pid)
    raw.extend(shared.drain(master, 0.2))
    os.close(master)

    screen = shared.TerminalScreen(shared.ROWS, shared.COLS)
    screen.feed(bytes(raw))
    return bytes(raw), screen


def log(message: str) -> None:
    print(f"[e2e] {message}", flush=True)


def write_artifacts(
    artifact_dir: Path,
    single_raw: bytes,
    single_screen: shared.TerminalScreen,
    initial_raw: bytes,
    initial_screen: shared.TerminalScreen,
    renamed_raw: bytes,
    renamed_screen: shared.TerminalScreen,
    down_raw: bytes,
    down_screen: shared.TerminalScreen,
    up_raw: bytes,
    up_screen: shared.TerminalScreen,
    esc_raw: bytes,
    esc_screen: shared.TerminalScreen,
    search_raw: bytes,
    search_screen: shared.TerminalScreen,
    run_raw: bytes,
    run_screen: shared.TerminalScreen,
) -> None:
    (artifact_dir / "single-account.raw").write_bytes(single_raw)
    (artifact_dir / "initial.raw").write_bytes(initial_raw)
    (artifact_dir / "renamed.raw").write_bytes(renamed_raw)
    (artifact_dir / "after-down.raw").write_bytes(down_raw)
    (artifact_dir / "after-up.raw").write_bytes(up_raw)
    (artifact_dir / "after-esc.raw").write_bytes(esc_raw)
    (artifact_dir / "after-search.raw").write_bytes(search_raw)
    (artifact_dir / "run.raw").write_bytes(run_raw)
    (artifact_dir / "single-account.txt").write_text(render_text(single_screen), encoding="utf-8")
    (artifact_dir / "initial.txt").write_text(render_text(initial_screen), encoding="utf-8")
    (artifact_dir / "renamed.txt").write_text(render_text(renamed_screen), encoding="utf-8")
    (artifact_dir / "after-down.txt").write_text(render_text(down_screen), encoding="utf-8")
    (artifact_dir / "after-up.txt").write_text(render_text(up_screen), encoding="utf-8")
    (artifact_dir / "after-esc.txt").write_text(render_text(esc_screen), encoding="utf-8")
    (artifact_dir / "after-search.txt").write_text(render_text(search_screen), encoding="utf-8")
    (artifact_dir / "run.txt").write_text(render_text(run_screen), encoding="utf-8")
    write_png(artifact_dir / "single-account.png", single_screen)
    write_png(artifact_dir / "initial.png", initial_screen)
    write_png(artifact_dir / "renamed.png", renamed_screen)
    write_png(artifact_dir / "after-down.png", down_screen)
    write_png(artifact_dir / "after-up.png", up_screen)
    write_png(artifact_dir / "after-esc.png", esc_screen)
    write_png(artifact_dir / "after-search.png", search_screen)
    write_png(artifact_dir / "run.png", run_screen)


def render_text(screen: shared.TerminalScreen) -> str:
    return "\n".join(screen.semantic_lines()) + "\n"


def write_report(
    artifact_dir: Path,
    results: list[str],
    single: shared.TerminalScreen,
    initial: shared.TerminalScreen,
    renamed: shared.TerminalScreen,
    down: shared.TerminalScreen,
    up: shared.TerminalScreen,
    esc: shared.TerminalScreen,
    search: shared.TerminalScreen,
    run_screen: shared.TerminalScreen,
) -> None:
    checklist = "\n".join(results)
    report = f"""# Clash E2E

## 覆盖项
{checklist}

## 产物
- `single-account.txt` / `initial.txt` / `renamed.txt` / `after-down.txt` / `after-up.txt` / `after-esc.txt` / `after-search.txt` / `run.txt`
- `single-account.png` / `initial.png` / `renamed.png` / `after-down.png` / `after-up.png` / `after-esc.png` / `after-search.png` / `run.png`
- `single-account.raw` / `initial.raw` / `renamed.raw` / `after-down.raw` / `after-up.raw` / `after-esc.raw` / `after-search.raw` / `run.raw`
- `config-home/` 保留本次测试配置目录
- `reset-before.txt` / `reset-after.txt` 记录 reset 前后账户文件

## 单账户首帧
```text
{render_text(single)}```

## 多账户首帧
```text
{render_text(initial)}```

## 重命名后首帧
```text
{render_text(renamed)}```

## clash run 首帧
```text
{render_text(run_screen)}```

## 下箭头后
```text
{render_text(down)}```

## 上箭头后
```text
{render_text(up)}```

## Esc 后
```text
{render_text(esc)}```

## 搜索过滤后
```text
{render_text(search)}```
"""
    (artifact_dir / "report.md").write_text(report, encoding="utf-8")


def write_png(path: Path, screen: shared.TerminalScreen) -> None:
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
    if line.startswith("clash>"):
        draw.text((10, y), "clash>", fill=(86, 156, 214), font=font)
        draw.text((82, y), line[len("clash>") :], fill=(212, 212, 212), font=font)
        return
    if line.startswith("→"):
        draw.text((10, y), "→", fill=(255, 0, 128), font=font)
        draw.text((30, y), line[1:], fill=(212, 212, 212), font=font)
        return
    draw.text((10, y), line, fill=(212, 212, 212), font=font)


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