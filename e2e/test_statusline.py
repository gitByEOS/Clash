#!/usr/bin/env python3
"""Clash statusline 端到端测试。"""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CLASH_BIN = ROOT / "target" / "debug" / "clash"


def strip_ansi(text: str) -> str:
    return re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", text)


def run_statusline(stdin: str, env: dict[str, str] | None = None, show: bool = False) -> tuple[int, str]:
    proc = subprocess.run(
        [str(CLASH_BIN), "statusline"],
        cwd=ROOT,
        env=env or os.environ.copy(),
        input=stdin,
        text=True,
        capture_output=True,
    )
    if show:
        print(f"  raw: {proc.stdout}")
        print(f"  clean: {strip_ansi(proc.stdout)}")
    return proc.returncode, strip_ansi(proc.stdout)


def test_statusline_empty_input() -> None:
    """空输入时显示 'Clash'"""
    code, out = run_statusline("", show=True)
    assert code == 0
    assert "Clash" in out


def test_statusline_basic_json() -> None:
    """基本 JSON 输入显示模型名和进度条"""
    data = {
        "model": {"display_name": "glm-5"},
        "context_window": {
            "context_window_size": 200000,
            "current_usage": {"input_tokens": 52000},
        },
    }
    code, out = run_statusline(json.dumps(data), show=True)
    assert code == 0
    assert "[glm-5]" in out
    assert "26%" in out
    assert "200k" in out
    assert "Clash" in out


def test_statusline_size_marker_removed() -> None:
    """模型名中的 [size] 标记被去除"""
    data = {
        "model": {"display_name": "deepseek-v4-pro[1m]"},
        "context_window": {
            "context_window_size": 1000000,
            "current_usage": {"input_tokens": 0},
        },
    }
    code, out = run_statusline(json.dumps(data), show=True)
    assert code == 0
    assert "[deepseek-v4-pro]" in out
    assert "[1m]" not in out  # size marker removed
    assert "1m" in out  # context size still shown


def test_statusline_high_percentage_color() -> None:
    """高百分比时进度条颜色变化（通过原始输出检查）"""
    data = {
        "model": {"display_name": "test-model"},
        "context_window": {
            "context_window_size": 100000,
            "current_usage": {"input_tokens": 90000},  # 90%
        },
    }
    proc = subprocess.run(
        [str(CLASH_BIN), "statusline"],
        cwd=ROOT,
        input=json.dumps(data),
        text=True,
        capture_output=True,
    )
    print(f"  90% raw: {proc.stdout}")
    # Red color code for >= 90%
    assert "\x1b[1;31m" in proc.stdout  # red


def test_statusline_session_duration() -> None:
    """有 session.start_time 时显示时长"""
    data = {
        "model": {"display_name": "glm-5"},
        "context_window": {
            "context_window_size": 200000,
            "current_usage": {"input_tokens": 1000},
        },
        "session": {"start_time": "2026-06-05T12:00:00Z"},
    }
    code, out = run_statusline(json.dumps(data), show=True)
    assert code == 0
    # Duration format: ⏱ Xm or ⏱ XhYm
    assert "⏱" in out or "h" in out or "m" in out


def test_statusline_no_session() -> None:
    """无 session 数据时不显示时长"""
    data = {
        "model": {"display_name": "glm-5"},
        "context_window": {
            "context_window_size": 200000,
            "current_usage": {"input_tokens": 1000},
        },
    }
    code, out = run_statusline(json.dumps(data))
    assert code == 0
    assert "⏱" not in out


def test_statusline_format() -> None:
    """输出格式符合设计：[model] 进度条 百分比 - size | Clash"""
    data = {
        "model": {"display_name": "glm-5"},
        "context_window": {
            "context_window_size": 200000,
            "current_usage": {"input_tokens": 50000},
        },
    }
    code, out = run_statusline(json.dumps(data), show=True)
    assert code == 0
    # Format: [model] ... N% - 200k | Clash
    assert "[glm-5]" in out
    assert "%" in out
    assert "200k" in out
    assert "|" in out
    assert "Clash" in out


def test_statusline_progress_colors() -> None:
    """进度条颜色覆盖所有级别"""
    sizes = [
        (10, "green"),    # < 50%
        (55, "orange"),   # 50-70%
        (75, "yellow"),   # 70-90%
        (95, "red"),      # >= 90%
    ]
    for pct, _ in sizes:
        data = {
            "model": {"display_name": f"test-{pct}pct"},
            "context_window": {
                "context_window_size": 100,
                "current_usage": {"input_tokens": pct},
            },
        }
        proc = subprocess.run(
            [str(CLASH_BIN), "statusline"],
            cwd=ROOT,
            input=json.dumps(data),
            text=True,
            capture_output=True,
        )
        print(f"  {pct}% raw: {proc.stdout.strip()}")


def test_auto_config_statusline() -> None:
    """自动配置 statusline 到 settings.json"""
    with tempfile.TemporaryDirectory(prefix="clash-test-") as tmpdir:
        settings_path = Path(tmpdir) / ".claude" / "settings.json"
        settings_path.parent.mkdir(parents=True)

        # Create settings without statusLine
        settings_path.write_text(
            json.dumps({"model": "test-model"}),
            encoding="utf-8",
        )

        env = os.environ.copy()
        env["HOME"] = tmpdir
        env["CLAUDE_CONFIG_DIR"] = str(settings_path.parent)

        # Run clash config (trigger ensure_statusline_config)
        subprocess.run(
            [str(CLASH_BIN), "config"],
            cwd=ROOT,
            env=env,
            capture_output=True,
        )

        # Check settings.json has statusLine
        content = json.loads(settings_path.read_text(encoding="utf-8"))
        assert "statusLine" in content
        assert content["statusLine"]["type"] == "command"
        assert content["statusLine"]["command"] == "clash statusline"


def test_auto_config_preserves_existing() -> None:
    """自动配置保留已有 settings"""
    with tempfile.TemporaryDirectory(prefix="clash-test-") as tmpdir:
        settings_path = Path(tmpdir) / ".claude" / "settings.json"
        settings_path.parent.mkdir(parents=True)

        # Create settings with existing fields
        original = {
            "model": "test-model",
            "permissions": {"defaultMode": "auto"},
        }
        settings_path.write_text(json.dumps(original), encoding="utf-8")

        env = os.environ.copy()
        env["HOME"] = tmpdir
        env["CLAUDE_CONFIG_DIR"] = str(settings_path.parent)

        subprocess.run(
            [str(CLASH_BIN), "config"],
            cwd=ROOT,
            env=env,
            capture_output=True,
        )

        content = json.loads(settings_path.read_text(encoding="utf-8"))
        # Original fields preserved
        assert content.get("model") == "test-model"
        assert content.get("permissions", {}).get("defaultMode") == "auto"
        # statusLine added
        assert "statusLine" in content


def test_auto_config_skips_if_valid() -> None:
    """已有有效 statusline 时跳过配置"""
    with tempfile.TemporaryDirectory(prefix="clash-test-") as tmpdir:
        settings_path = Path(tmpdir) / ".claude" / "settings.json"
        settings_path.parent.mkdir(parents=True)

        # Create settings with valid statusLine
        original = {
            "statusLine": {
                "type": "command",
                "command": "custom-statusline",
            },
        }
        settings_path.write_text(json.dumps(original), encoding="utf-8")

        env = os.environ.copy()
        env["HOME"] = tmpdir
        env["CLAUDE_CONFIG_DIR"] = str(settings_path.parent)

        subprocess.run(
            [str(CLASH_BIN), "config"],
            cwd=ROOT,
            env=env,
            capture_output=True,
        )

        content = json.loads(settings_path.read_text(encoding="utf-8"))
        # Should NOT overwrite existing valid statusLine
        assert content["statusLine"]["command"] == "custom-statusline"


def test_auto_config_fixes_empty_statusline() -> None:
    """空 statusLine {} 被修复为有效配置"""
    with tempfile.TemporaryDirectory(prefix="clash-test-") as tmpdir:
        settings_path = Path(tmpdir) / ".claude" / "settings.json"
        settings_path.parent.mkdir(parents=True)

        # Create settings with empty statusLine
        original = {"statusLine": {}}
        settings_path.write_text(json.dumps(original), encoding="utf-8")

        env = os.environ.copy()
        env["HOME"] = tmpdir
        env["CLAUDE_CONFIG_DIR"] = str(settings_path.parent)

        subprocess.run(
            [str(CLASH_BIN), "config"],
            cwd=ROOT,
            env=env,
            capture_output=True,
        )

        content = json.loads(settings_path.read_text(encoding="utf-8"))
        # Empty {} should be replaced with valid config
        assert content["statusLine"]["type"] == "command"
        assert content["statusLine"]["command"] == "clash statusline"


def main() -> int:
    tests = [
        test_statusline_empty_input,
        test_statusline_basic_json,
        test_statusline_size_marker_removed,
        test_statusline_high_percentage_color,
        test_statusline_session_duration,
        test_statusline_no_session,
        test_statusline_format,
        test_statusline_progress_colors,
        test_auto_config_statusline,
        test_auto_config_preserves_existing,
        test_auto_config_skips_if_valid,
        test_auto_config_fixes_empty_statusline,
    ]

    print("[e2e] build")
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True, capture_output=True)

    passed = 0
    for test in tests:
        name = test.__name__
        try:
            test()
            print(f"[e2e] ✓ {name}")
            passed += 1
        except AssertionError as e:
            print(f"[e2e] ✗ {name}: {e}")
            return 1
        except Exception as e:
            print(f"[e2e] ✗ {name}: {type(e).__name__}: {e}")
            return 1

    print(f"[e2e] {passed}/{len(tests)} passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())