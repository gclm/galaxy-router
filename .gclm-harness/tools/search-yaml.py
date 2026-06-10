#!/usr/bin/env python3
"""Search YAML frontmatter in markdown files.

Usage:
    search-yaml.py <directory> [--filter key=value] [--query text]

Examples:
    # Search all approved designs
    search-yaml.py .gclm-harness/tasks --filter status=approved

    # Search decisions by category
    search-yaml.py .gclm-harness/memory/decisions --filter category=constraint

    # Search by content
    search-yaml.py .gclm-harness --query "oauth"

    # Combined
    search-yaml.py .gclm-harness/tasks --filter status=in_progress --query "auth"
"""

import sys
import yaml
from pathlib import Path
import re


def extract_frontmatter(file_path: Path) -> dict:
    """Extract YAML frontmatter from markdown file."""
    try:
        content = file_path.read_text()
    except Exception:
        return {}
    match = re.match(r'^---\n(.*?)\n---', content, re.DOTALL)
    if match:
        try:
            return yaml.safe_load(match.group(1)) or {}
        except Exception:
            return {}
    return {}


def parse_filter(filter_str: str) -> tuple:
    """Parse filter string like 'key=value' or 'key~=value'."""
    if "~=" in filter_str:
        key, value = filter_str.split("~=", 1)
        return key, value, "partial"
    elif "=" in filter_str:
        key, value = filter_str.split("=", 1)
        return key, value, "exact"
    else:
        return filter_str, None, "exists"


def matches_filter(frontmatter: dict, key: str, value: str, match_type: str) -> bool:
    """Check if frontmatter matches filter."""
    if match_type == "exists":
        return key in frontmatter
    elif match_type == "exact":
        return key in frontmatter and str(frontmatter[key]) == value
    elif match_type == "partial":
        return key in frontmatter and value in str(frontmatter[key])
    return False


def search(directory: str, filters: list = None, query: str = None, json_output: bool = False):
    """Search files by frontmatter filters."""
    import json

    dir_path = Path(directory)
    if not dir_path.exists():
        print(f"Error: {directory} not found")
        sys.exit(1)

    results = []

    for md_file in sorted(dir_path.rglob("*.md")):
        fm = extract_frontmatter(md_file)
        if not fm:
            continue

        # Apply filters
        match = True
        if filters:
            for key, value, match_type in filters:
                if not matches_filter(fm, key, value, match_type):
                    match = False
                    break

        # Apply query
        if query and match:
            try:
                content = md_file.read_text().lower()
                if query.lower() not in content:
                    match = False
            except Exception:
                match = False

        if match:
            results.append({"path": str(md_file), "frontmatter": fm})

    # Output
    if json_output:
        print(json.dumps(results, indent=2, ensure_ascii=False))
    else:
        if not results:
            print("No results found")
            return

        for r in results:
            print(r["path"])
            for key, value in r["frontmatter"].items():
                print(f"  {key}: {value}")
            print()

        print(f"Found {len(results)} file(s)")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    directory = sys.argv[1]
    filters = []
    query = None
    json_output = "--json" in sys.argv

    i = 2
    while i < len(sys.argv):
        if sys.argv[i] == "--filter" and i + 1 < len(sys.argv):
            filters.append(parse_filter(sys.argv[i + 1]))
            i += 2
        elif sys.argv[i] == "--query" and i + 1 < len(sys.argv):
            query = sys.argv[i + 1]
            i += 2
        elif sys.argv[i] == "--json":
            i += 1
        else:
            i += 1

    search(directory, filters, query, json_output)
