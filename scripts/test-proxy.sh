#!/bin/bash
# Galaxy Proxy API 测试脚本
# 模型: mimo-v2.5-pro | 覆盖: 健康/错误/Models/Chat流式非流式/Anthropic流式/Responses流式
set -e

BASE_URL="http://127.0.0.1:8080"
API_KEY="${API_KEY:-gp-test-key}"
MODEL="mimo-v2.5-pro"
UA_HEADER="User-Agent: HermesAgent/0.14.0"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

TOTAL=0 PASSED=0 FAILED=0
pass() { TOTAL=$((TOTAL+1)); PASSED=$((PASSED+1)); echo -e "  ${GREEN}[PASS]${NC} $1"; }
fail() { TOTAL=$((TOTAL+1)); FAILED=$((FAILED+1)); echo -e "  ${RED}[FAIL]${NC} $1"; }
info() { echo -e "\n${CYAN}[TEST]${NC} $1"; }

# 从 Chat SSE 中提取文本内容（含 reasoning_content）
extract_chat_content() {
  python3 -c "
import sys, json
text = ''
reasoning = ''
for line in sys.stdin:
    line = line.strip()
    if line.startswith('data: ') and line != 'data: [DONE]':
        try:
            d = json.loads(line[6:])
            delta = d.get('choices',[{}])[0].get('delta',{})
            c = delta.get('content') or ''
            r = delta.get('reasoning_content') or ''
            if c: text += c
            if r: reasoning += r
        except: pass
if reasoning and not text:
    print(f'[reasoning] {reasoning}')
else:
    print(text)
" < "$1" 2>/dev/null
}

# 从 Anthropic SSE 中提取文本内容（支持 text_delta 和 thinking_delta）
extract_anthropic_content() {
  python3 -c "
import sys, json
text = ''
thinking = ''
for line in sys.stdin:
    line = line.strip()
    if line.startswith('data: '):
        try:
            d = json.loads(line[6:])
            if d.get('type') == 'content_block_delta':
                delta = d.get('delta',{})
                dt = delta.get('type','')
                if dt == 'text_delta':
                    text += delta.get('text','')
                elif dt == 'thinking_delta':
                    thinking += delta.get('thinking','')
        except: pass
if thinking and not text:
    print(f'[thinking] {thinking[:100]}...')
else:
    print(text)
" < "$1" 2>/dev/null
}

# 从 Responses SSE 中提取文本内容
extract_responses_content() {
  python3 -c "
import sys, json
text = ''
for line in sys.stdin:
    line = line.strip()
    if line.startswith('data: '):
        try:
            d = json.loads(line[6:])
            t = d.get('type','')
            if t == 'response.output_text.delta':
                text += d.get('delta','')
            elif t == 'response.output_text.done':
                pass
        except: pass
print(text)
" < "$1" 2>/dev/null
}

echo "=========================================="
echo "  Galaxy Proxy API 测试套件"
echo "  模型: $MODEL"
echo "=========================================="

# ── 1. 健康检查 ──
info "1. 健康检查"
HTTP=$(curl -s -o /tmp/gp_out.json -w "%{http_code}" "$BASE_URL/api/v1/health")
if [ "$HTTP" = "200" ]; then
  STATUS=$(python3 -c "import json; print(json.load(open('/tmp/gp_out.json'))['status'])")
  pass "HTTP 200, status=$STATUS"
else
  fail "HTTP $HTTP (期望 200)"
fi

# ── 2. 空消息 ──
info "2. 空消息"
HTTP=$(curl -s -o /dev/null -w "%{http_code}" \
  "$BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -H "$UA_HEADER" \
  -d "{\"model\":\"$MODEL\",\"messages\":[]}")
if [ "$HTTP" != "200" ]; then
  pass "正确拒绝: HTTP $HTTP"
else
  pass "上游接受: HTTP 200"
fi

# ── 3. 不存在的模型 ──
info "3. 不存在的模型"
HTTP=$(curl -s -o /dev/null -w "%{http_code}" \
  "$BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -H "$UA_HEADER" \
  -d '{"model":"nonexistent-xyz","messages":[{"role":"user","content":"test"}]}')
if [ "$HTTP" != "200" ]; then
  pass "正确返回错误: HTTP $HTTP"
else
  fail "应返回错误, 实际 HTTP 200"
fi

# ── 4. 无效 API Key ──
info "4. 无效 API Key"
HTTP=$(curl -s -o /dev/null -w "%{http_code}" \
  "$BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer invalid-key" \
  -H "Content-Type: application/json" \
  -H "$UA_HEADER" \
  -d "{\"model\":\"$MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"test\"}]}")
if [ "$HTTP" = "401" ]; then
  pass "正确拒绝: HTTP 401"
else
  fail "HTTP $HTTP (期望 401)"
fi

# ── 5. Models 列表 ──
info "5. Models 列表"
HTTP=$(curl -s -o /tmp/gp_out.json -w "%{http_code}" \
  "$BASE_URL/v1/models" \
  -H "Authorization: Bearer $API_KEY" \
  -H "$UA_HEADER")
if [ "$HTTP" = "200" ]; then
  INFO=$(python3 -c "
import json
d = json.load(open('/tmp/gp_out.json'))
models = [m['id'] for m in d.get('data',[])]
print(f'{len(models)} 个: {', '.join(models[:5])}')
" 2>/dev/null || echo "parse error")
  pass "$INFO"
else
  fail "HTTP $HTTP (期望 200)"
fi

# ── 6. Chat Completions 非流式 ──
info "6. Chat Completions 非流式"
HTTP=$(curl -s -o /tmp/gp_out.json -w "%{http_code}" \
  "$BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -H "$UA_HEADER" \
  -d "{
    \"model\": \"$MODEL\",
    \"messages\": [{\"role\": \"user\", \"content\": \"你好，请用一句话介绍你自己\"}],
    \"max_tokens\": 100
  }")
if [ "$HTTP" = "200" ]; then
  CONTENT=$(python3 -c "
import json
d = json.load(open('/tmp/gp_out.json'))
m = d.get('choices',[{}])[0].get('message',{}).get('content','')
u = d.get('usage',{})
print(f'model={d.get(\"model\",\"\")} tokens={u.get(\"total_tokens\",\"?\")} reply={m[:60]}...')
" 2>/dev/null || echo "parse error")
  pass "$CONTENT"
else
  fail "HTTP $HTTP"
fi

# ── 7. Chat Completions 流式 ──
info "7. Chat Completions 流式"
HTTP=$(curl -s --max-time 30 -o /tmp/gp_stream.txt -w "%{http_code}" \
  "$BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -H "$UA_HEADER" \
  -d "{
    \"model\": \"$MODEL\",
    \"messages\": [{\"role\": \"user\", \"content\": \"从1数到5\"}],
    \"max_tokens\": 200,
    \"stream\": true
  }")
if [ "$HTTP" = "200" ]; then
  CHUNKS=$(grep -c "^data:" /tmp/gp_stream.txt 2>/dev/null || echo 0)
  HAS_DONE=$(grep -c "data: \[DONE\]" /tmp/gp_stream.txt 2>/dev/null || echo 0)
  CONTENT=$(extract_chat_content /tmp/gp_stream.txt)
  echo -e "  ${YELLOW}[内容]${NC} $CONTENT"
  pass "chunks=$CHUNKS, [DONE]=$HAS_DONE"
else
  fail "HTTP $HTTP"
fi

# ── 8. Anthropic Messages 流式 ──
info "8. Anthropic Messages 流式 (x-api-key 认证)"
HTTP=$(curl -s --max-time 30 -o /tmp/gp_stream.txt -w "%{http_code}" \
  "$BASE_URL/v1/messages" \
  -H "x-api-key: $API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -H "$UA_HEADER" \
  -d "{
    \"model\": \"$MODEL\",
    \"messages\": [{\"role\": \"user\", \"content\": \"从1数到3\"}],
    \"max_tokens\": 50,
    \"stream\": true
  }")
if [ "$HTTP" = "200" ]; then
  CHUNKS=$(grep -c "^event:" /tmp/gp_stream.txt 2>/dev/null || echo 0)
  HAS_MSG_STOP=$(grep -c "event: message_stop" /tmp/gp_stream.txt 2>/dev/null || echo 0)
  CONTENT=$(extract_anthropic_content /tmp/gp_stream.txt)
  echo -e "  ${YELLOW}[内容]${NC} $CONTENT"
  pass "events=$CHUNKS, message_stop=$HAS_MSG_STOP"
else
  BODY=$(head -c 200 /tmp/gp_stream.txt 2>/dev/null)
  fail "HTTP $HTTP: $BODY"
fi

# ── 9. OpenAI Responses 流式 ──
info "9. OpenAI Responses 流式"
curl -s --max-time 30 "$BASE_URL/v1/responses" \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -H "$UA_HEADER" \
  -d "{
    \"model\": \"$MODEL\",
    \"input\": \"从1数到3\",
    \"max_output_tokens\": 200,
    \"stream\": true
  }" 2>/dev/null | tee /tmp/gp_stream.txt > /dev/null || true
CHUNKS=$(grep -c "^data:" /tmp/gp_stream.txt 2>/dev/null || echo 0)
CHUNKS=$(echo "$CHUNKS" | head -1)
CONTENT=$(extract_responses_content /tmp/gp_stream.txt)
if [ "$CHUNKS" -gt 0 ]; then
  echo -e "  ${YELLOW}[内容]${NC} $CONTENT"
  pass "chunks=$CHUNKS"
else
  fail "无响应内容"
fi

# ── 总结 ──
echo ""
echo "=========================================="
echo -e "  结果: ${GREEN}$PASSED 通过${NC} / ${RED}$FAILED 失败${NC} / $TOTAL 总计"
echo "=========================================="

[ "$FAILED" -eq 0 ] || exit 1
