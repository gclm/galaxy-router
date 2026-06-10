#!/usr/bin/env python3
"""Shared utilities for Keel hooks."""

import json
import os
import re
import sys
from pathlib import Path


def find_harness_dir(cwd: str) -> Path | None:
    """Find .gclm-harness/ by searching from cwd upward (max 10 levels)."""
    path = Path(cwd).resolve()
    for _ in range(10):
        candidate = path / ".gclm-harness"
        if candidate.is_dir():
            return candidate
        parent = path.parent
        if parent == path:
            break
        path = parent
    return None


def read_frontmatter(path: Path) -> dict:
    """Extract YAML frontmatter from a markdown file."""
    if not path.exists():
        return {}
    content = path.read_text()
    match = re.match(r"^---\n(.*?)\n---", content, re.DOTALL)
    if not match:
        return {}
    try:
        import yaml
        return yaml.safe_load(match.group(1)) or {}
    except Exception:
        return {}


def find_active_task(harness_dir: Path) -> Path | None:
    """Find most recent active task (planning or in_progress).

    Searches tasks/ but NOT tasks/archive/.
    Returns the task directory Path, or None.
    """
    import yaml

    tasks_dir = harness_dir / "tasks"
    if not tasks_dir.exists():
        return None

    active = []
    for task_dir in tasks_dir.iterdir():
        if not task_dir.is_dir():
            continue
        if task_dir.name == "archive":
            continue
        yaml_path = task_dir / "task.yaml"
        if not yaml_path.exists():
            continue
        try:
            with open(yaml_path) as f:
                data = yaml.safe_load(f)
            if data.get("status") in ("planning", "in_progress"):
                active.append((task_dir, str(data.get("created", ""))))
        except Exception:
            continue

    if not active:
        return None
    active.sort(key=lambda x: x[1], reverse=True)
    return active[0][0]


def get_task_info(task_dir: Path) -> dict:
    """Read task.yaml + design.md frontmatter, return structured info."""
    import yaml

    with open(task_dir / "task.yaml") as f:
        task = yaml.safe_load(f)

    design_fm = read_frontmatter(task_dir / "design.md")

    return {
        "path": str(task_dir),
        "title": task.get("title", ""),
        "status": task.get("status", ""),
        "type": task.get("type", ""),
        "complexity": task.get("complexity", ""),
        "sop": task.get("sop", ""),
        "design_status": design_fm.get("status", "none"),
    }


def read_hook_input() -> dict:
    """Read and parse hook JSON from stdin."""
    try:
        return json.loads(sys.stdin.read())
    except Exception:
        return {}


def output_hook_response(event_name: str, additional_context: str):
    """Write hook response JSON to stdout."""
    response = {
        "hookSpecificOutput": {
            "hookEventName": event_name,
            "additionalContext": additional_context,
        }
    }
    json.dump(response, sys.stdout, ensure_ascii=False)
