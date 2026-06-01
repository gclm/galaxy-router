#!/usr/bin/env python3
"""Octopus 导出数据 → Galaxy Proxy 备份格式 转换脚本"""

import json
import sys
from datetime import datetime, timezone

TYPE_MAP = {
    0: "openai_chat",
    1: "openai_response",
    2: "anthropic",
    3: "gemini",
    4: "openai_embedding",
    5: "openai_images",
}

# Octopus settings key → Galaxy settings key（只转换两边共有的）
SETTINGS_MAP = {
    "proxy_url": "proxy_url",
    "cors_allow_origins": "cors_allow_origins",
}


def convert(input_path: str, output_path: str):
    with open(input_path, encoding="utf-8") as f:
        src = json.load(f)

    # channel_id(int) → upstream api key objects
    channel_keys_map: dict[int, list[dict]] = {}
    for ck in src.get("channel_keys", []):
        if ck.get("enabled", True):
            channel_keys_map.setdefault(ck["channel_id"], []).append({
                "key": ck["channel_key"],
                "note": ck.get("name", "") or ck.get("remark", "") or "",
                "enabled": True,
            })

    # channel_id(int) → channel dict
    channel_map: dict[int, dict] = {}
    channels = []
    for ch in src.get("channels", []):
        channel_map[ch["id"]] = ch

        # 合并 model 和 custom_model
        models = []
        for m in ch.get("model", "").split(","):
            if m.strip():
                models.append(m.strip())
        for m in ch.get("custom_model", "").split(","):
            if m.strip() and m.strip() not in models:
                models.append(m.strip())

        endpoints = []
        for ep in ch.get("endpoints", []):
            ep_type = ep["type"]
            if isinstance(ep_type, int):
                ep_type = TYPE_MAP.get(ep_type, "openai_chat")
            endpoints.append({"type": ep_type, "base_url": ep["base_url"]})

        # custom_header 格式转换
        custom_headers = []
        for h in ch.get("custom_header", []):
            if isinstance(h, dict) and h.get("key") and h.get("value"):
                custom_headers.append({"key": h["key"], "value": h["value"]})

        channels.append({
            "id": str(ch["id"]),
            "name": ch["name"],
            "api_keys": channel_keys_map.get(ch["id"], []),
            "endpoints": endpoints,
            "models": models,
            "rate_limit_rpm": None,
            "rate_limit_tpm": None,
            "failure_threshold": 3,
            "blacklist_minutes": 10,
            "concurrency": 10,
            "custom_headers": custom_headers,
            "enabled": ch.get("enabled", True),
            "created_at": datetime.now(timezone.utc).isoformat(),
            "updated_at": datetime.now(timezone.utc).isoformat(),
        })

    # groups + group_items → Galaxy group format
    group_item_map: dict[int, list] = {}
    for gi in src.get("group_items", []):
        group_item_map.setdefault(gi["group_id"], []).append(gi)

    groups = []
    for g in src.get("groups", []):
        items = []
        for gi in group_item_map.get(g["id"], []):
            ch = channel_map.get(gi["channel_id"])
            if not ch:
                continue
            items.append({
                "channel_name": ch["name"],
                "model_name": gi["model_name"],
                "priority": gi.get("priority", 1),
                "weight": gi.get("weight", 1),
            })
        groups.append({
            "name": g["name"],
            "match_regex": g.get("match_regex") or None,
            "retry_enabled": True,
            "max_retries": 3,
            "first_token_timeout_secs": g.get("first_token_time_out", 30),
            "enabled": True,
            "items": items,
        })

    # api_keys
    api_keys = []
    for ak in src.get("api_keys", []):
        api_keys.append({
            "name": ak["name"],
            "api_key": ak["api_key"],
            "enabled": ak.get("enabled", True),
        })

    # settings（只转换两边共有的 key）
    settings = []
    for s in src.get("settings", []):
        if s["key"] in SETTINGS_MAP:
            settings.append({
                "key": SETTINGS_MAP[s["key"]],
                "value": s["value"],
            })

    backup = {
        "format": "galaxy-router-backup",
        "version": 1,
        "exported_at": datetime.now(timezone.utc).isoformat(),
        "app_version": "0.1.0",
        "data": {
            "channels": channels,
            "groups": groups,
            "api_keys": api_keys,
            "settings": settings,
        },
    }

    with open(output_path, "w", encoding="utf-8") as f:
        json.dump(backup, f, indent=2, ensure_ascii=False)

    print(f"转换完成:")
    print(f"  渠道:   {len(channels)}")
    print(f"  分组:   {len(groups)}")
    print(f"  API Key: {len(api_keys)}")
    print(f"  设置:   {len(settings)}")
    print(f"  输出:   {output_path}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("用法: python3 scripts/convert-octopus.py <octopus-export.json> [output.json]")
        sys.exit(1)

    input_path = sys.argv[1]
    output_path = sys.argv[2] if len(sys.argv) > 2 else "data/octopus-converted.json"
    convert(input_path, output_path)
