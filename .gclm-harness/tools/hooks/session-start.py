#!/usr/bin/env python3
"""Keel SessionStart hook.

Injects project context (must/) + active task state + next action
on session start, clear, and compact.

Input:  JSON from Claude Code stdin  {"session_id", "cwd", "hook_event_name"}
Output: JSON with hookSpecificOutput.additionalContext  (<keel-context> block)
"""

import os
import re
import sys

sys.path.insert(0, os.path.dirname(__file__))

from keel_common import (
    find_harness_dir,
    find_active_task,
    get_task_info,
    read_hook_input,
    output_hook_response,
)


def compute_next_action(task_info: dict) -> str:
    """Derive next action from task status + design status."""
    status = task_info["status"]
    design = task_info["design_status"]

    if status == "planning":
        if design == "approved":
            return "design 已 approved，运行 task.py start 进入实现"
        elif design == "draft":
            return "design 还没 approved，先完成 Gate 1（写 design.md 并等待 approved）"
        else:
            return "写 design.md 并等待 approved"
    elif status == "in_progress":
        if design == "approved":
            return "按 design.md 推进策略执行，完成所有 step 后进 Gate 3 验收"
        elif design == "draft":
            return "⚠️ design 未 approved，先 review 再继续"
        else:
            return "按 design.md 推进策略执行"
    return ""


def read_must_file(harness_dir, filename: str, fallback_msg: str) -> str:
    """Read a must/ file, return content or fallback message if empty/missing."""
    path = harness_dir / "must" / filename
    if path.exists() and path.stat().st_size > 0:
        return path.read_text().strip()
    return fallback_msg


def needs_cold_start(harness_dir) -> bool:
    """Check if knowledge layer is still template (cold-start not yet run).

    Returns True if must/ files contain template placeholders like {xxx}.
    """
    must_dir = harness_dir / "must"
    if not must_dir.exists():
        return True
    for md in must_dir.glob("*.md"):
        content = md.read_text()
        if re.search(r"\{[a-zA-Z]", content):
            return True
    return False


def main():
    hook_input = read_hook_input()
    cwd = hook_input.get("cwd", os.getcwd())
    event = hook_input.get("hook_event_name", "SessionStart")

    harness_dir = find_harness_dir(cwd)
    if not harness_dir:
        # Not a Keel project — silent exit
        sys.exit(0)

    # ── must/ files ──
    project = read_must_file(
        harness_dir,
        "project-basics.md",
        "(项目基础信息待补充 — 运行 cold-start SOP 生成)",
    )
    pitfalls = read_must_file(
        harness_dir,
        "pitfalls.md",
        "(常见坑待补充 — 运行 cold-start SOP 生成)",
    )

    # ── active task ──
    task_dir = find_active_task(harness_dir)
    if task_dir:
        info = get_task_info(task_dir)
        next_action = compute_next_action(info)
        task_block = (
            f"Status: {info['status']}\n"
            f"Task: {info['title']}\n"
            f"Type: {info['type']} ({info['complexity']})\n"
            f"SOP: {info['sop']}\n"
            f"Design: {info['design_status']}\n"
            f"\nNext action: {next_action}"
        )
    else:
        task_block = "No active task.\n判断用户意图，路由到对应 SOP。"

    # ── cold-start detection ──
    cold_start_block = ""
    if needs_cold_start(harness_dir):
        cold_start_block = (
            "\n<cold-start-reminder>\n"
            "⚠️ 知识层尚未初始化（must/ 文件仍为模板）。\n"
            "请先运行 cold-start SOP 生成项目知识层骨架，再开始其他任务。\n"
            "操作：直接说「帮我运行 cold-start SOP」即可。\n"
            "</cold-start-reminder>\n"
        )

    # ── assemble context ──
    context = (
        "<keel-context>\n"
        "You are in a Keel-managed project. Read and follow the context below.\n"
        f"{cold_start_block}"
        "\n<project>\n"
        f"{project}\n"
        "</project>\n"
        "\n<pitfalls>\n"
        f"{pitfalls}\n"
        "</pitfalls>\n"
        "\n<current-task>\n"
        f"{task_block}\n"
        "</current-task>\n"
        "\n<reading-guide>\n"
        "需要时按需读取：\n"
        "- 开始新功能 → 读 sop/feature.md\n"
        "- 修 bug → 读 sop/issue.md\n"
        "- 重构 → 读 sop/refactor.md\n"
        "- 查开发计划 → 读 guides/roadmap.md\n"
        "- 查编码规范 → 读 reference/conventions.md\n"
        "</reading-guide>\n"
        "</keel-context>"
    )

    output_hook_response(event, context)


if __name__ == "__main__":
    main()
