#!/usr/bin/env python3
"""galaxy-router 接口冒烟测试（黑盒端到端）。

启动 mock 上游 + galaxy-router 真实 binary（临时 DB），HTTP 测核心链路：
init/login → admin CRUD → proxy 转发（→ mock 上游）→ 缺口加固端点。

用法：
    make build && python3 scripts/api_smoke.py
    或 make test-smoke

互补于 Rust 集成测试（tests/api + tests/proxy，in-process + wiremock）——
本脚本走真实 binary HTTP，验证端到端部署链路。
"""
import http.server
import json
import os
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

GALAXY_BIN = "target/debug/galaxy-router"
GALAXY_PORT = 18090
ADMIN_USER, ADMIN_PASS = "admin", "admin12345"
TIMEOUT = 30


# ── mock 上游（OpenAI Chat Completions；记录收到的请求供协议转换/多模态黑盒断言）──
class MockUpstream(http.server.BaseHTTPRequestHandler):
    # 最近一次请求快照（path/body/auth）；单线程顺序测试，用例间靠 read_last_request() 清空隔离
    last_request = {"path": None, "body": None, "auth": None}

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0) or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            parsed = json.loads(raw) if raw else None
        except Exception:
            parsed = raw.decode(errors="replace")
        MockUpstream.last_request = {
            "path": self.path,
            "body": parsed,
            "auth": self.headers.get("Authorization"),
        }
        if self.path.endswith("/chat/completions"):
            body = json.dumps({
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "smoke-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "mock-ok"},
                    "finish_reason": "stop",
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7},
            }).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, *_):
        pass


def read_last_request():
    """读取并清空 mock 记录的最近一次请求（用例间隔离）。"""
    req = dict(MockUpstream.last_request)
    MockUpstream.last_request = {"path": None, "body": None, "auth": None}
    return req


def start_mock():
    srv = http.server.HTTPServer(("127.0.0.1", 0), MockUpstream)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{port}", srv


# ── HTTP 辅助 ──
def http_req(method, url, body=None, token=None, expect=None):
    if body is not None:
        data = json.dumps(body).encode()
    elif method in ("POST", "PUT", "PATCH"):
        data = b"{}"  # axum 要求这些方法带 Content-Type: application/json
    else:
        data = None
    req = urllib.request.Request(url, data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        resp = urllib.request.urlopen(req, timeout=TIMEOUT)
        code, text = resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        code, text = e.code, e.read().decode()
    payload = json.loads(text) if text else {}
    if expect is not None and code != expect:
        raise AssertionError(f"{method} {url}: 期望 {expect}, 实际 {code}, body={text[:200]}")
    return code, payload


def wait_up(base, seconds=30):
    for _ in range(seconds):
        try:
            urllib.request.urlopen(f"{base}/", timeout=1)
            return True
        except Exception:
            time.sleep(1)
    return False


def main():
    results = []

    def check(name, cond, detail=""):
        results.append((name, bool(cond), detail))

    mock_url, _ = start_mock()

    ts = int(time.time())
    db = f"/tmp/galaxy_smoke_{ts}.db"
    cfg = f"/tmp/galaxy_smoke_{ts}.toml"
    with open("config.toml") as f:
        tpl = f.read()
    cfg_content = (
        tpl.replace('data/galaxy.db', db)
        .replace('file = true', 'file = false')
    )
    with open(cfg, "w") as f:
        f.write(cfg_content)

    if not os.path.exists(GALAXY_BIN):
        print(f"✗ 找不到 {GALAXY_BIN}，请先 make build")
        return 1

    proc = subprocess.Popen(
        [GALAXY_BIN, "--config", cfg, "--port", str(GALAXY_PORT)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    base = f"http://127.0.0.1:{GALAXY_PORT}"

    try:
        check("服务启动", wait_up(base), f"port {GALAXY_PORT}")
        if not results[-1][1]:
            print("✗ galaxy-router 启动失败")
            return 1

        # init
        code, j = http_req("POST", f"{base}/api/v1/init",
                           {"username": ADMIN_USER, "password": ADMIN_PASS, "site_title": "smoke"},
                           expect=201)
        token = j["data"]["token"]
        check("init 创建 admin", code == 201)

        # 重复 init 应 409
        code2, _ = http_req("POST", f"{base}/api/v1/init",
                            {"username": ADMIN_USER, "password": ADMIN_PASS}, expect=409)
        check("init 幂等(409)", code2 == 409)

        # admin 列表（空）
        _, j = http_req("GET", f"{base}/api/v1/admin/channels", token=token, expect=200)
        check("admin channels list(空)", j["data"]["total"] == 0)

        # 创建 channel（指向 mock 上游，注意 endpoints 用 "type" serde rename）
        _, j = http_req("POST", f"{base}/api/v1/admin/channels", token=token, expect=201, body={
            "name": "smoke-mock",
            "api_keys": [{"key": "sk-mock", "enabled": True}],
            "endpoints": [{"type": "openai_chat", "base_url": mock_url, "enabled": True}],
            "models": ["smoke-model"],
            "enabled": True,
        })
        check("create channel", j.get("data", {}).get("id"))
        ch_id = j["data"]["id"]

        # 创建 route
        _, j = http_req("POST", f"{base}/api/v1/admin/routes", token=token, expect=201, body={
            "name": "smoke-model",
            "items": [{"channel_id": ch_id, "model_name": "smoke-model", "priority": 1, "weight": 100}],
            "enabled": True,
        })
        check("create route", j.get("data", {}).get("id"))

        # 创建 api key
        _, j = http_req("POST", f"{base}/api/v1/admin/api-keys", token=token, expect=201, body={
            "name": "smoke-key", "enabled": True,
        })
        check("create api-key", j.get("data", {}).get("api_key"))
        api_key = j["data"]["api_key"]

        # proxy 无 key → 401
        code, _ = http_req("POST", f"{base}/v1/chat/completions",
                           {"model": "smoke-model", "messages": [{"role": "user", "content": "hi"}]},
                           expect=401)
        check("proxy 鉴权(无key 401)", code == 401)

        # proxy → mock 上游
        code, j = http_req("POST", f"{base}/v1/chat/completions",
                           {"model": "smoke-model", "messages": [{"role": "user", "content": "hi"}], "max_tokens": 10},
                           token=api_key, expect=200)
        check("proxy 转发→mock", code == 200 and j.get("choices"))

        # ── 协议转换：Anthropic client(/v1/messages) → openai_chat upstream（conversion 路径）──
        read_last_request()  # 清空，隔离上一个用例的请求
        code, j = http_req("POST", f"{base}/v1/messages", {
            "model": "smoke-model",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 10,
        }, token=api_key, expect=200)
        last = read_last_request()
        conv_resp_ok = (
            j.get("type") == "message"
            and j.get("content") and j["content"][0].get("type") == "text"
            and j.get("stop_reason") == "end_turn"
            and "input_tokens" in j.get("usage", {}) and "output_tokens" in j.get("usage", {})
        )
        # mock 收到的应是 OpenAI Chat（content 字符串 "hi"，证明 Anthropic→OpenAI 转换）
        conv_up_ok = (
            last["body"] is not None
            and last["path"] and last["path"].endswith("/chat/completions")
            and last["body"].get("messages", [{}])[0].get("content") == "hi"
        )
        check("协议转换 anthropic→openai_chat", code == 200 and conv_resp_ok and conv_up_ok)

        # ── 多模态 passthrough：openai_chat content 数组（text + image_url data URL）原样透传 ──
        read_last_request()
        code, j = http_req("POST", f"{base}/v1/chat/completions", {
            "model": "smoke-model",
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "看图"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,aGVsbG8="}},
            ]}],
            "max_tokens": 10,
        }, token=api_key, expect=200)
        last = read_last_request()
        recv = last["body"].get("messages", [{}])[0].get("content") if last["body"] else None
        mm_pt_ok = (
            isinstance(recv, list)
            and any(p.get("type") == "image_url"
                    and p.get("image_url", {}).get("url") == "data:image/png;base64,aGVsbG8="
                    for p in recv)
        )
        check("多模态 passthrough content 数组", code == 200 and mm_pt_ok)

        # ── 多模态 + 协议转换：Anthropic image(base64 source) → openai_chat image_url(data URL) ──
        read_last_request()
        code, j = http_req("POST", f"{base}/v1/messages", {
            "model": "smoke-model",
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "看图"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGVsbG8="}},
            ]}],
            "max_tokens": 10,
        }, token=api_key, expect=200)
        last = read_last_request()
        recv = last["body"].get("messages", [{}])[0].get("content") if last["body"] else None
        mm_conv_ok = (
            isinstance(recv, list)
            and any(p.get("type") == "image_url"
                    and p.get("image_url", {}).get("url") == "data:image/png;base64,aGVsbG8="
                    for p in recv)
        )
        check("多模态+协议转换 image base64→dataurl",
              code == 200 and j.get("type") == "message" and mm_conv_ok)

        # stats（proxy 后应有记录）
        _, j = http_req("GET", f"{base}/api/v1/admin/stats/overview", token=token, expect=200)
        check("stats overview", "latency_p50" in j.get("data", {}))

        # 缺口2：vacuum 端点
        code, _ = http_req("POST", f"{base}/api/v1/admin/system-info/vacuum", token=token, expect=200)
        check("vacuum 端点", code == 200)

        # 缺口1：retention_days 配置（migration 21 seed = 30）
        _, j = http_req("GET", f"{base}/api/v1/admin/settings", token=token, expect=200)
        retention = next((s for s in j["data"] if s.get("key") == "usage.retention_days"), None)
        check("retention_days=30", retention and retention["value"] == "30")

    finally:
        proc.send_signal(signal.SIGINT)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        for f in (db, db + "-wal", db + "-shm", cfg):
            try:
                os.remove(f)
            except OSError:
                pass

    # 汇总
    passed = sum(1 for _, c, _ in results if c)
    print(f"\n{'=' * 52}")
    for name, cond, detail in results:
        print(f"  {'✓' if cond else '✗'} {name} {detail}")
    print(f"{'=' * 52}")
    print(f"接口冒烟：{passed}/{len(results)} 通过")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
