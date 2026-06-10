#!/usr/bin/env python3
"""Keel task lifecycle management.

Usage:
    task.py create "<title>" [type] [complexity]
    task.py start <task-dir>
    task.py finish <task-dir>
    task.py archive <task-dir>
    task.py list [status]
    task.py approve-design <task-dir>
    task.py check-design <task-dir>
    task.py current [--json]
"""

import sys
import os
import yaml
from datetime import datetime
from pathlib import Path
import re
import json
import shutil


def get_tasks_dir() -> Path:
    """Get the tasks directory path."""
    return Path(".gclm-harness/tasks")


def extract_frontmatter(file_path: Path) -> dict:
    """Extract YAML frontmatter from markdown file."""
    content = file_path.read_text()
    match = re.match(r'^---\n(.*?)\n---', content, re.DOTALL)
    if match:
        try:
            return yaml.safe_load(match.group(1))
        except Exception:
            return {}
    return {}


def create(title: str, task_type: str = "feature", complexity: str = "moderate"):
    """Create a new task directory."""
    date = datetime.now().strftime("%Y-%m-%d")
    # Slug: pinyin for Chinese, lowercase, hyphens, strip non-alnum
    try:
        from pypinyin import lazy_pinyin
        slug = "-".join(lazy_pinyin(title))
    except ImportError:
        slug = title
    slug = slug.lower().replace(" ", "-")
    slug = re.sub(r'[^a-z0-9一-鿿-]', '', slug)
    slug = slug[:40].rstrip("-")
    task_dir = get_tasks_dir() / f"{date}-{slug}"
    task_dir.mkdir(parents=True, exist_ok=True)

    # Create task.yaml
    task_yaml = {
        "type": task_type,
        "complexity": complexity,
        "sop": f"sop/{task_type}.md",
        "status": "planning",
        "feature": f"{date}-{slug}",
        "title": title,
        "created": date,
        "completed": None,
        "notes": ""
    }
    with open(task_dir / "task.yaml", "w") as f:
        yaml.dump(task_yaml, f, default_flow_style=False, allow_unicode=True, sort_keys=False)

    # Copy design template
    design_template = Path(".gclm-harness/templates/task/design.md")
    if design_template.exists():
        design_content = design_template.read_text()
        design_content = design_content.replace("{{feature}}", f"{date}-{slug}")
        design_content = design_content.replace("{{title}}", title)
        (task_dir / "design.md").write_text(design_content)
    else:
        # Create minimal design.md
        design_content = f"""---
doc_type: {task_type}-design
status: draft
feature: {date}-{slug}
summary: {title}
---

# {title}

## 1. 决策与约束

### 用户目标
-

### 核心行为
-

### 成功标准
-

### 明确不做
-

## 2. 名词层 + 编排层

### 现状
-

### 变化
-

## 3. 验收契约

-

## 4. 挂载点清单

-
"""
        (task_dir / "design.md").write_text(design_content)

    print(f"Created task: {task_dir}")
    return task_dir


def start(task_dir: str):
    """Start a task (set status to in_progress)."""
    yaml_path = Path(task_dir) / "task.yaml"
    if not yaml_path.exists():
        print(f"Error: {yaml_path} not found")
        sys.exit(1)
    with open(yaml_path) as f:
        data = yaml.safe_load(f)
    data["status"] = "in_progress"
    with open(yaml_path, "w") as f:
        yaml.dump(data, f, default_flow_style=False, allow_unicode=True, sort_keys=False)
    print(f"Started task: {task_dir}")


def finish(task_dir: str):
    """Finish a task (set status to completed)."""
    yaml_path = Path(task_dir) / "task.yaml"
    if not yaml_path.exists():
        print(f"Error: {yaml_path} not found")
        sys.exit(1)
    with open(yaml_path) as f:
        data = yaml.safe_load(f)
    data["status"] = "completed"
    data["completed"] = datetime.now().strftime("%Y-%m-%d")
    with open(yaml_path, "w") as f:
        yaml.dump(data, f, default_flow_style=False, allow_unicode=True, sort_keys=False)
    print(f"Finished task: {task_dir}")


def archive(task_dir: str):
    """Archive a completed task: set status, move to tasks/archive/{YYYY-MM}/."""
    yaml_path = Path(task_dir) / "task.yaml"
    if not yaml_path.exists():
        print(f"Error: {yaml_path} not found")
        sys.exit(1)

    with open(yaml_path) as f:
        data = yaml.safe_load(f)

    # Ensure completed
    data["status"] = "completed"
    if not data.get("completed"):
        data["completed"] = datetime.now().strftime("%Y-%m-%d")
    with open(yaml_path, "w") as f:
        yaml.dump(data, f, default_flow_style=False, allow_unicode=True, sort_keys=False)

    # Move to archive/{YYYY-MM}/
    month = datetime.now().strftime("%Y-%m")
    archive_dir = get_tasks_dir() / "archive" / month
    archive_dir.mkdir(parents=True, exist_ok=True)

    task_name = Path(task_dir).name
    dest = archive_dir / task_name
    if dest.exists():
        print(f"Error: archive target already exists: {dest}")
        sys.exit(1)

    shutil.move(str(task_dir), str(dest))
    print(f"Archived task: {task_dir} → {dest}")


def list_tasks(status: str = None):
    """List all tasks."""
    tasks_dir = get_tasks_dir()
    if not tasks_dir.exists():
        print("No tasks directory")
        return

    count = 0
    for task_dir in sorted(tasks_dir.iterdir()):
        if task_dir.is_dir() and (task_dir / "task.yaml").exists():
            with open(task_dir / "task.yaml") as f:
                data = yaml.safe_load(f)
            if status is None or data.get("status") == status:
                print(f"  {data['feature']}: {data['title']} [{data['status']}]")
                count += 1

    if count == 0:
        print("No tasks found")


def approve_design(task_dir: str):
    """Approve design.md (set status to approved)."""
    design_path = Path(task_dir) / "design.md"
    if not design_path.exists():
        print(f"Error: {design_path} not found")
        sys.exit(1)
    content = design_path.read_text()
    match = re.match(r'^---\n(.*?)\n---', content, re.DOTALL)
    if not match:
        print(f"Error: No frontmatter in {design_path}")
        sys.exit(1)
    fm = yaml.safe_load(match.group(1))
    if fm.get("status") == "approved":
        print(f"Already approved: {design_path}")
        return
    fm["status"] = "approved"
    new_fm = yaml.dump(fm, default_flow_style=False, allow_unicode=True, sort_keys=False).strip()
    new_content = f"---\n{new_fm}\n---{content[match.end():]}"
    design_path.write_text(new_content)
    print(f"Approved design: {design_path}")


def check_design_approved(task_dir: str) -> bool:
    """Check if design.md is approved."""
    design_path = Path(task_dir) / "design.md"
    if not design_path.exists():
        return False
    fm = extract_frontmatter(design_path)
    return fm.get("status") == "approved"


def get_current_task() -> str | None:
    """Get the current active task (most recent planning or in_progress).

    Skips tasks/archive/ subdirectory.
    """
    tasks_dir = get_tasks_dir()
    if not tasks_dir.exists():
        return None

    for task_dir in sorted(tasks_dir.iterdir(), reverse=True):
        if not task_dir.is_dir() or task_dir.name == "archive":
            continue
        if not (task_dir / "task.yaml").exists():
            continue
        with open(task_dir / "task.yaml") as f:
            data = yaml.safe_load(f)
        if data.get("status") in ("planning", "in_progress"):
            return str(task_dir)
    return None


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    cmd = sys.argv[1]
    if cmd == "create":
        title = sys.argv[2] if len(sys.argv) > 2 else "Untitled"
        task_type = sys.argv[3] if len(sys.argv) > 3 else "feature"
        complexity = sys.argv[4] if len(sys.argv) > 4 else "moderate"
        create(title, task_type, complexity)
    elif cmd == "start":
        if len(sys.argv) < 3:
            print("Usage: task.py start <task-dir>")
            sys.exit(1)
        start(sys.argv[2])
    elif cmd == "finish":
        if len(sys.argv) < 3:
            print("Usage: task.py finish <task-dir>")
            sys.exit(1)
        finish(sys.argv[2])
    elif cmd == "archive":
        if len(sys.argv) < 3:
            print("Usage: task.py archive <task-dir>")
            sys.exit(1)
        archive(sys.argv[2])
    elif cmd == "list":
        status = sys.argv[2] if len(sys.argv) > 2 else None
        list_tasks(status)
    elif cmd == "approve-design":
        if len(sys.argv) < 3:
            print("Usage: task.py approve-design <task-dir>")
            sys.exit(1)
        approve_design(sys.argv[2])
    elif cmd == "check-design":
        if len(sys.argv) < 3:
            print("Usage: task.py check-design <task-dir>")
            sys.exit(1)
        approved = check_design_approved(sys.argv[2])
        print("approved" if approved else "not-approved")
    elif cmd == "current":
        current = get_current_task()
        if not current:
            if "--json" in sys.argv:
                print(json.dumps({}))
            else:
                print("No active task")
        elif "--json" in sys.argv:
            yaml_path = Path(current) / "task.yaml"
            with open(yaml_path) as f:
                data = yaml.safe_load(f)
            design_fm = extract_frontmatter(Path(current) / "design.md")
            result = {
                "path": current,
                "title": data.get("title", ""),
                "status": data.get("status", ""),
                "type": data.get("type", ""),
                "complexity": data.get("complexity", ""),
                "design_status": design_fm.get("status", "none"),
            }
            print(json.dumps(result, ensure_ascii=False))
        else:
            print(current)
    else:
        print(f"Unknown command: {cmd}")
        print(__doc__)
        sys.exit(1)
