#!/usr/bin/env python3
"""Validate YAML files and frontmatter.

Usage:
    validate-yaml.py <file> [--frontmatter]
    validate-yaml.py <directory> [--recursive]

Examples:
    # Validate a YAML file
    validate-yaml.py task.yaml

    # Validate frontmatter in markdown
    validate-yaml.py design.md --frontmatter

    # Validate all YAML files in directory
    validate-yaml.py .gclm-harness/tasks --recursive
"""

import sys
import yaml
from pathlib import Path
import re


def validate_yaml_file(file_path: str) -> tuple:
    """Validate a YAML file. Returns (success, error_message)."""
    try:
        with open(file_path) as f:
            yaml.safe_load(f)
        return True, None
    except yaml.YAMLError as e:
        return False, str(e)
    except FileNotFoundError:
        return False, f"File not found: {file_path}"


def validate_frontmatter(file_path: str) -> tuple:
    """Validate YAML frontmatter in markdown file. Returns (success, error_message)."""
    try:
        content = Path(file_path).read_text()
    except FileNotFoundError:
        return False, f"File not found: {file_path}"

    match = re.match(r'^---\n(.*?)\n---', content, re.DOTALL)
    if not match:
        return False, "No frontmatter found"

    try:
        yaml.safe_load(match.group(1))
        return True, None
    except yaml.YAMLError as e:
        return False, str(e)


def validate_directory(directory: str, recursive: bool = False) -> int:
    """Validate all YAML files in directory. Returns error count."""
    dir_path = Path(directory)
    if not dir_path.exists():
        print(f"Error: {directory} not found")
        return 1

    errors = 0
    count = 0

    # Find YAML files
    if recursive:
        yaml_files = list(dir_path.rglob("*.yaml")) + list(dir_path.rglob("*.yml"))
    else:
        yaml_files = list(dir_path.glob("*.yaml")) + list(dir_path.glob("*.yml"))

    # Find markdown files with frontmatter
    md_files = list(dir_path.rglob("*.md")) if recursive else list(dir_path.glob("*.md"))

    # Validate YAML files
    for yaml_file in yaml_files:
        count += 1
        success, error = validate_yaml_file(str(yaml_file))
        if not success:
            print(f"✗ {yaml_file}: {error}")
            errors += 1
        else:
            print(f"✓ {yaml_file}")

    # Validate markdown frontmatter
    for md_file in md_files:
        content = md_file.read_text()
        if content.startswith("---"):
            count += 1
            success, error = validate_frontmatter(str(md_file))
            if not success:
                print(f"✗ {md_file}: {error}")
                errors += 1
            else:
                print(f"✓ {md_file}")

    print(f"\nValidated {count} file(s), {errors} error(s)")
    return errors


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    target = sys.argv[1]
    frontmatter_only = "--frontmatter" in sys.argv
    recursive = "--recursive" in sys.argv

    target_path = Path(target)

    if target_path.is_dir():
        errors = validate_directory(target, recursive)
        sys.exit(0 if errors == 0 else 1)
    elif target_path.is_file():
        if frontmatter_only:
            success, error = validate_frontmatter(target)
        else:
            success, error = validate_yaml_file(target)

        if success:
            print(f"✓ {target}")
            sys.exit(0)
        else:
            print(f"✗ {target}: {error}")
            sys.exit(1)
    else:
        print(f"Error: {target} not found")
        sys.exit(1)
