#!/usr/bin/env python3
"""PowerShell 版 Clash 端到端测试，复用 Rust e2e 断言。"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from datetime import datetime
from pathlib import Path

import run_test as shared


CLASH_PS1 = shared.ROOT / "bin" / "clash.ps1"
ORIGINAL_RUN_CLASH = shared.run_clash
ORIGINAL_RUN_CLASH_WITH_INPUT = shared.run_clash_with_input
ORIGINAL_CONFIG_HOME_ENV = shared.CONFIG_HOME_ENV


def main() -> int:
    stamp = datetime.now().strftime("%y%m%d-%H%M%S")
    artifact_dir = shared.ARTIFACT_ROOT / f"pwsh-{stamp}"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    config_home = artifact_dir / "config-home"
    config_home.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env["CLASH_TEST_CONFIG_HOME"] = str(config_home)
    env["APPDATA"] = str(config_home)
    env["CLASH_SKIP_AUTO_TEST"] = "1"

    ensure_pwsh()
    shared.run_clash = run_pwsh
    shared.run_clash_with_input = run_pwsh_with_input
    results: list[str] = []

    log("config idx0")
    shared.test_config_set(env, 0, shared.BASE_URL, shared.API_KEY, shared.MODELS)
    shared.test_config_show(env, 0, shared.BASE_URL, shared.MODELS)
    results.append("- config --idx 0 写入并展示 auth")

    log("single run")
    single = shared.test_run_exec_env(env, expected_base=shared.BASE_URL)
    assert "[1st]" not in single.stdout
    _, single_screen = capture_pwsh_frame(env, ["clash"], b"kimi-k2")
    shared.assert_tui_single_account(single_screen)
    results.append("- 单账户 run 不显示账户标签")

    log("config idx1")
    shared.test_config_set(env, 1, shared.ALT_BASE_URL, shared.ALT_API_KEY, shared.ALT_MODELS)
    shared.test_config_show(env, 1, shared.ALT_BASE_URL, shared.ALT_MODELS)
    results.append("- config --idx 1 写入并展示 auth1")

    log("multi run")
    multi = shared.test_run_exec_env(env, expected_base=shared.BASE_URL)
    _, multi_screen = capture_pwsh_frame(env, ["clash"], b"qwen-max")
    shared.assert_tui_multi_account(multi_screen)
    results.append("- 多账户 run 可从聚合列表启动")

    log("rename via config")
    shared.test_rename_via_config(env, 0, "work")
    results.append("- config 设置 NAME=work 后配置文件含 NAME 字段")

    log("renamed account label")
    _, renamed_screen = capture_pwsh_frame(env, ["clash"], b"qwen-max")
    shared.assert_tui_renamed(renamed_screen)
    results.append("- 重命名后 TUI 显示 [work] 而非 [1st]")

    log("test command")
    shared.test_connection(env)
    results.append("- test 与 test --idx 1 连通测试成功")

    log("removed commands")
    shared.test_removed_commands(env)
    results.append("- add-model / change-token 不再作为命令入口")

    log("reset")
    shared.test_reset(env, artifact_dir)
    results.append("- reset 真实删除全部 auth 槽")

    log("account numeric sort")
    shared.test_config_set(env, 2, shared.BASE_URL, shared.API_KEY, shared.MODELS)
    shared.test_config_set(env, 10, shared.ALT_BASE_URL, shared.ALT_API_KEY, shared.ALT_MODELS)
    shared.test_run_exec_env(env, expected_base=shared.BASE_URL)
    results.append("- 账户按数字 idx 排序，auth2 排在 auth10 前")

    write_artifacts(artifact_dir, single.stdout, multi.stdout)
    write_report(artifact_dir, results, single.stdout, multi.stdout)
    print(f"PowerShell E2E passed: {artifact_dir}")
    shared.run_clash = ORIGINAL_RUN_CLASH
    shared.run_clash_with_input = ORIGINAL_RUN_CLASH_WITH_INPUT
    shared.CONFIG_HOME_ENV = ORIGINAL_CONFIG_HOME_ENV
    return 0


def ensure_pwsh() -> None:
    if not shutil.which("pwsh"):
        raise RuntimeError("未找到 pwsh")


def run_pwsh(args: list[str], env: dict[str, str], *, check: bool = True) -> shared.CliResult:
    proc = subprocess.run(
        ["pwsh", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(CLASH_PS1), *args],
        cwd=shared.ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=check,
    )
    return shared.CliResult(proc.returncode, shared.strip_ansi(proc.stdout), shared.strip_ansi(proc.stderr))


def run_pwsh_with_input(args: list[str], env: dict[str, str], stdin: str, *, check: bool = True) -> shared.CliResult:
    proc = subprocess.run(
        ["pwsh", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(CLASH_PS1), *args],
        cwd=shared.ROOT,
        env=env,
        input=stdin,
        text=True,
        capture_output=True,
        check=check,
    )
    return shared.CliResult(proc.returncode, shared.strip_ansi(proc.stdout), shared.strip_ansi(proc.stderr))


def capture_pwsh_frame(
    env: dict[str, str],
    args: list[str],
    until: bytes,
) -> tuple[bytes, shared.TerminalScreen]:
    pwsh_path = shutil.which("pwsh")
    assert pwsh_path

    pid, master = os.forkpty()
    if pid == 0:
        os.chdir(shared.ROOT)
        os.execve(
            pwsh_path,
            ["pwsh", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(CLASH_PS1), *args[1:]],
            env,
        )

    shared.set_winsize(master)
    raw = bytearray()
    raw.extend(shared.drain(master, 3.0, until=until, min_bytes=700))
    shared.stop_child(pid)
    raw.extend(shared.drain(master, 0.2))
    os.close(master)

    screen = shared.TerminalScreen(shared.ROWS, shared.COLS)
    screen.feed(bytes(raw))
    return bytes(raw), screen


def log(message: str) -> None:
    print(f"[pwsh-e2e] {message}", flush=True)


def write_artifacts(artifact_dir: Path, single_stdout: str, multi_stdout: str) -> None:
    (artifact_dir / "single-account.txt").write_text(single_stdout, encoding="utf-8")
    (artifact_dir / "multi-account.txt").write_text(multi_stdout, encoding="utf-8")


def write_report(artifact_dir: Path, results: list[str], single_stdout: str, multi_stdout: str) -> None:
    checklist = "\n".join(results)
    report = f"""# Clash PowerShell E2E

## 覆盖项
{checklist}

## 产物
- `single-account.txt` / `multi-account.txt`
- `config-home/` 保留本次测试配置目录
- `reset-before.txt` / `reset-after.txt` 记录 reset 前后账户文件

## 单账户 run 输出
```text
{single_stdout}```

## 多账户 run 输出
```text
{multi_stdout}```
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
