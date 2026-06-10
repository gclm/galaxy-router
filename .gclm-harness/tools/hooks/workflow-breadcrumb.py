#!/usr/bin/env python3
"""Keel UserPromptSubmit hook.

Injects a lightweight breadcrumb (<keel-breadcrumb>) on every user message,
reminding the AI of the current Gate state.

Input:  JSON from Claude Code stdin  {"session_id", "cwd", "hook_event_name"}
Output: JSON with hookSpecificOutput.additionalContext  (<keel-breadcrumb> block)
"""

import os
import sys

sys.path.insert(0, os.path.dirname(__file__))

from keel_common import (
    find_harness_dir,
    find_active_task,
    get_task_info,
    read_hook_input,
    output_hook_response,
)


def compute_gate_reminder(task_info: dict) -> str:
    """One-line Gate reminder based on task + design status."""
    status = task_info["status"]
    design = task_info["design_status"]

    if status == "planning":
        if design == "approved":
            return "design 已 approved，可以 task.py start 进入实现"
        return "design 还没 approved，先完成 Gate 1"
    elif status == "in_progress":
        if design == "approved":
            return "按 design.md 推进策略执行，完成所有 step 后进 Gate 3 验收"
        return "⚠️ design 回到了 draft，先修正再继续"
    return ""


NO_TASK_BREADCRUMB = (
    "<keel-breadcrumb>\n"
    "No active task.\n"
    "- 新功能/需求 → 路由到 sop/feature.md\n"
    "- Bug 修复 → 路由到 sop/issue.md\n"
    "- 重构/优化 → 路由到 sop/refactor.md\n"
    "- 需求不清 → 路由到 sop/brainstorm.md\n"
    "- Trivial 改动 → 直接改，不走 SOP\n"
    "</keel-breadcrumb>"
)


def main():
    hook_input = read_hook_input()
    cwd = hook_input.get("cwd", os.getcwd())
    event = hook_input.get("hook_event_name", "UserPromptSubmit")

    harness_dir = find_harness_dir(cwd)
    if not harness_dir:
        sys.exit(0)

    task_dir = find_active_task(harness_dir)
    if not task_dir:
        output_hook_response(event, NO_TASK_BREADCRUMB)
        return

    info = get_task_info(task_dir)
    reminder = compute_gate_reminder(info)

    breadcrumb = (
        "<keel-breadcrumb>\n"
        f"Task: {info['title']} ({info['status']}) "
        f"| Design: {info['design_status']} "
        f"| Complexity: {info['complexity']}\n"
        f"\nGate 提醒：{reminder}\n"
        "</keel-breadcrumb>"
    )

    output_hook_response(event, breadcrumb)


if __name__ == "__main__":
    main()
