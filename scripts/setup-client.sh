#!/usr/bin/env bash
#
# Galaxy Router 客户端配置脚本
# 交互式引导用户配置 Codex CLI / Claude Code / Cursor / Cline / OpenClaw / Hermes
#
set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

DEFAULT_HOST="127.0.0.1"
DEFAULT_PORT="8080"

print_banner() {
    echo ""
    echo -e "${CYAN}${BOLD}  ╔══════════════════════════════════════╗"
    echo -e "  ║     Galaxy Router 客户端配置助手     ║"
    echo -e "  ╚══════════════════════════════════════╝${NC}"
    echo ""
}

print_step() {
    echo -e "\n${BLUE}${BOLD}[$1]${NC} $2"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
}

print_warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
}

# ── Step 1: 连接信息 ──────────────────────────────────

collect_connection_info() {
    print_step "1/3" "配置连接信息"

    echo -ne "  Galaxy Router 地址 [${DEFAULT_HOST}]: "
    read -r host
    host="${host:-$DEFAULT_HOST}"

    echo -ne "  端口 [${DEFAULT_PORT}]: "
    read -r port
    port="${port:-$DEFAULT_PORT}"

    base_url="http://${host}:${port}"
    openai_base_url="${base_url}/v1"

    # 验证连通性
    echo -ne "  检测连通性... "
    if curl -sf --connect-timeout 3 "${base_url}/api/v1/health" > /dev/null 2>&1; then
        print_success "连接成功"
    else
        print_warning "无法连接 ${base_url}，请确认服务已启动"
        echo -ne "  是否继续? [y/N]: "
        read -r continue_anyway
        if [[ ! "${continue_anyway}" =~ ^[yY]$ ]]; then
            exit 0
        fi
    fi
}

# ── Step 2: API Key ───────────────────────────────────

collect_api_key() {
    print_step "2/3" "输入 API Key"

    echo -ne "  API Key (sk-gr-xxxx): "
    read -r api_key

    if [[ -z "${api_key}" ]]; then
        print_error "API Key 不能为空"
        exit 1
    fi

    if [[ ! "${api_key}" == sk-gr-* ]]; then
        print_warning "Galaxy Router 的 API Key 以 sk-gr- 开头，请确认输入正确"
    fi
}

# ── Step 3: 选择工具 ──────────────────────────────────

select_tool() {
    print_step "3/3" "选择要配置的工具"

    echo ""
    echo -e "  ${BOLD}1)${NC} Codex CLI      ${CYAN}(OpenAI 命令行)${NC}"
    echo -e "  ${BOLD}2)${NC} Claude Code    ${CYAN}(Anthropic 命令行)${NC}"
    echo -e "  ${BOLD}3)${NC} Cursor         ${CYAN}(AI 代码编辑器)${NC}"
    echo -e "  ${BOLD}4)${NC} Cline          ${CYAN}(VS Code 插件)${NC}"
    echo -e "  ${BOLD}5)${NC} OpenClaw       ${CYAN}(开源 Claude Code)${NC}"
    echo -e "  ${BOLD}6)${NC} Hermes         ${CYAN}(AI 编程助手)${NC}"
    echo -e "  ${BOLD}7)${NC} ${BOLD}全部${NC}           ${CYAN}(输出所有工具的配置命令)${NC}"
    echo -e "  ${BOLD}0)${NC} 退出"
    echo ""
    echo -ne "  请选择 [0-7]: "
    read -r choice
}

# ── 输出配置 ──────────────────────────────────────────

show_codex() {
    echo ""
    echo -e "${GREEN}${BOLD}─── Codex CLI 配置 ───${NC}"
    echo ""
    echo -e "  ${YELLOW}临时生效（当前终端）：${NC}"
    echo ""
    echo "  export OPENAI_API_KEY=\"${api_key}\""
    echo "  export OPENAI_BASE_URL=\"${openai_base_url}\""
    echo ""
    echo -e "  ${YELLOW}永久生效（写入 shell 配置）：${NC}"
    echo ""
    echo "  # 根据你的 shell 选择其一："
    echo "  echo 'export OPENAI_API_KEY=\"${api_key}\"' >> ~/.bashrc"
    echo "  echo 'export OPENAI_BASE_URL=\"${openai_base_url}\"' >> ~/.bashrc"
    echo "  # 或"
    echo "  echo 'export OPENAI_API_KEY=\"${api_key}\"' >> ~/.zshrc"
    echo "  echo 'export OPENAI_BASE_URL=\"${openai_base_url}\"' >> ~/.zshrc"
    echo ""
}

show_claude_code() {
    echo ""
    echo -e "${GREEN}${BOLD}─── Claude Code 配置 ───${NC}"
    echo ""
    echo -e "  ${YELLOW}临时生效（当前终端）：${NC}"
    echo ""
    echo "  export ANTHROPIC_API_KEY=\"${api_key}\""
    echo "  export ANTHROPIC_BASE_URL=\"${base_url}\""
    echo ""
    echo -e "  ${YELLOW}永久生效（写入 shell 配置）：${NC}"
    echo ""
    echo "  # 根据你的 shell 选择其一："
    echo "  echo 'export ANTHROPIC_API_KEY=\"${api_key}\"' >> ~/.bashrc"
    echo "  echo 'export ANTHROPIC_BASE_URL=\"${base_url}\"' >> ~/.bashrc"
    echo "  # 或"
    echo "  echo 'export ANTHROPIC_API_KEY=\"${api_key}\"' >> ~/.zshrc"
    echo "  echo 'export ANTHROPIC_BASE_URL=\"${base_url}\"' >> ~/.zshrc"
    echo ""
    echo -e "  ${CYAN}提示：ANTHROPIC_BASE_URL 不带 /v1 后缀${NC}"
    echo ""
}

show_cursor() {
    echo ""
    echo -e "${GREEN}${BOLD}─── Cursor 配置 ───${NC}"
    echo ""
    echo "  1. 打开 Cursor → Settings → Models"
    echo "  2. OpenAI API Key 填入：${api_key}"
    echo "  3. OpenAI Base URL 填入：${openai_base_url}"
    echo "  4. 点击 Verify 验证连通性"
    echo "  5. 选择你需要的模型"
    echo ""
}

show_cline() {
    echo ""
    echo -e "${GREEN}${BOLD}─── Cline 配置 ───${NC}"
    echo ""
    echo -e "  ${YELLOW}方式一：OpenAI Compatible${NC}"
    echo "  1. 打开 Cline 侧边栏 → 设置 ⚙"
    echo "  2. API Provider 选择 OpenAI Compatible"
    echo "  3. Base URL 填入：${openai_base_url}"
    echo "  4. API Key 填入：${api_key}"
    echo ""
    echo -e "  ${YELLOW}方式二：Anthropic${NC}"
    echo "  1. API Provider 选择 Anthropic"
    echo "  2. Base URL 填入：${base_url}"
    echo "  3. API Key 填入：${api_key}"
    echo ""
}

show_openclaw() {
    echo ""
    echo -e "${GREEN}${BOLD}─── OpenClaw 配置 ───${NC}"
    echo ""
    echo -e "  ${YELLOW}临时生效（当前终端）：${NC}"
    echo ""
    echo "  export ANTHROPIC_API_KEY=\"${api_key}\""
    echo "  export ANTHROPIC_BASE_URL=\"${base_url}\""
    echo ""
    echo -e "  ${YELLOW}永久生效（写入 shell 配置）：${NC}"
    echo ""
    echo "  echo 'export ANTHROPIC_API_KEY=\"${api_key}\"' >> ~/.zshrc"
    echo "  echo 'export ANTHROPIC_BASE_URL=\"${base_url}\"' >> ~/.zshrc"
    echo ""
}

show_hermes() {
    echo ""
    echo -e "${GREEN}${BOLD}─── Hermes 配置 ───${NC}"
    echo ""
    echo "  1. 打开 Hermes 设置"
    echo "  2. API Provider 选择 OpenAI Compatible 或 Custom"
    echo "  3. Base URL 填入：${openai_base_url}"
    echo "  4. API Key 填入：${api_key}"
    echo "  5. 选择你在 Galaxy Router 中配置的模型"
    echo ""
}

show_all() {
    show_codex
    show_claude_code
    show_cursor
    show_cline
    show_openclaw
    show_hermes
}

# ── 主流程 ────────────────────────────────────────────

main() {
    print_banner

    collect_connection_info
    collect_api_key
    select_tool

    case "${choice}" in
        1) show_codex ;;
        2) show_claude_code ;;
        3) show_cursor ;;
        4) show_cline ;;
        5) show_openclaw ;;
        6) show_hermes ;;
        7) show_all ;;
        0) echo ""; exit 0 ;;
        *) print_error "无效选择"; exit 1 ;;
    esac

    echo -e "${CYAN}详细配置指南: docs/client-setup.md${NC}"
    echo ""
}

main "$@"
