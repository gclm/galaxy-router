# Galaxy Router Makefile

.PHONY: all build frontend-build dev dev-stop \
        test fmt fmt-check clippy clean check \
        release release-version \
        brew-deploy brew-restart \
        docker docker-run help

CARGO_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
VERSION ?= $(CARGO_VERSION)
COMMIT  := $(shell git rev-parse --short HEAD 2>/dev/null || echo 'unknown')
APP     := galaxy-router
DIST    := build/dist
BREW_PREFIX ?= $(shell brew --prefix 2>/dev/null || echo /opt/homebrew)
BREW_SERVICE ?= gclm/tap/$(APP)
BREW_BIN ?= $(BREW_PREFIX)/opt/$(APP)/bin/$(APP)
BREW_BUILD_META ?= $(COMMIT)
BREW_RESTART ?= 1

# 默认目标
all: build

# ===== 开发 =====

frontend-build:
	cd frontend && pnpm install && pnpm build

build: frontend-build
	cargo build

dev:
	@echo "启动开发环境..."
	@trap 'kill 0; exit 0' INT TERM; \
	cargo run & \
	cd frontend && npx vite; \
	wait

dev-stop:
	@lsof -ti :8080 2>/dev/null | xargs kill 2>/dev/null; \
	lsof -ti :5173 2>/dev/null | xargs kill 2>/dev/null; \
	lsof -ti :5174 2>/dev/null | xargs kill 2>/dev/null; \
	echo "开发环境已停止"

# ===== 测试 / 检查 =====

test:
	cargo test

# 快速单元测试（不含 HTTP 层）
test-unit:
	cargo test --lib

# 管理后台 API 回归
test-api:
	cargo test --test api

# 代理转发回归（Step 3 实现）
test-proxy:
	cargo test --test proxy

# 完整回归（单元 + API + 代理）
test-regression:
	cargo test --test api --test proxy

# 接口冒烟（黑盒端到端：启动 mock 上游 + 真实 binary，HTTP 测核心链路）
test-smoke: build
	python3 scripts/api_smoke.py

# 完整验证（单元 + API + 代理 + 接口冒烟）：单测后跑接口自动化
test-all: test test-smoke

# CI 完整流程
test-ci: clippy test-regression

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy -- -D warnings

clean:
	cargo clean
	rm -rf data/galaxy.db
	rm -rf /tmp/galaxy_test*
	rm -rf $(DIST)

check: clippy test

# ===== 发布构建 =====

release:
	cargo build --release

# 发布新版本：同步 Cargo.toml / Cargo.lock / frontend/package.json，提交并推送 tag
# 用法: make release-version VERSION=0.0.3
release-version: check
	@set -e; \
	if [ "$(VERSION)" = "$(CARGO_VERSION)" ]; then \
	  echo "请传入新版本号，例如: make release-version VERSION=0.0.3" >&2; exit 1; \
	fi; \
	if ! echo "$(VERSION)" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-.+][0-9A-Za-z.-]+)?$$'; then \
	  echo "版本号格式不合法: $(VERSION)" >&2; exit 1; \
	fi; \
	if [ -n "$$(git status --porcelain)" ]; then \
	  echo "工作区不干净，请先提交或暂存当前修改:" >&2; \
	  git status --short >&2; \
	  exit 1; \
	fi; \
	tag="v$(VERSION)"; \
	if git rev-parse "$$tag" >/dev/null 2>&1; then \
	  echo "tag 已存在: $$tag" >&2; exit 1; \
	fi; \
	perl -0pi -e 's/^version = "[^"]+"/version = "$(VERSION)"/m' Cargo.toml; \
	cargo check; \
	tmp=$$(mktemp); \
	jq '.version = "$(VERSION)"' frontend/package.json > "$$tmp"; \
	mv "$$tmp" frontend/package.json; \
	git add Cargo.toml Cargo.lock frontend/package.json; \
	if ! git diff --cached --name-only | grep -qx 'Cargo.lock'; then \
	  echo "ERROR: Cargo.lock 未随版本号更新，请手动执行 cargo update -p $(APP) 或 cargo check" >&2; \
	  exit 1; \
	fi; \
	git commit -m "chore: 发布 v$(VERSION)"; \
	git tag -a "$$tag" -m "v$(VERSION)"; \
	git push origin HEAD; \
	git push origin "$$tag"; \
	echo "已发布版本: $$tag"

# ===== Homebrew 本地部署 =====

brew-deploy: frontend-build
	@set -e; \
	echo "构建本地测试版本: $(VERSION)+$(BREW_BUILD_META)"; \
	GALAXY_BUILD_META="$(BREW_BUILD_META)" cargo build --release; \
	src="target/release/$(APP)"; \
	dst="$(BREW_BIN)"; \
	if [ ! -x "$$src" ]; then \
	  echo "release 二进制不存在: $$src" >&2; exit 1; \
	fi; \
	if [ ! -d "$$(dirname "$$dst")" ]; then \
	  echo "Homebrew 部署目录不存在: $$(dirname "$$dst")" >&2; \
	  echo "可用 BREW_BIN=/path/to/$(APP) 覆盖目标路径" >&2; \
	  exit 1; \
	fi; \
	if [ -f "$$dst" ]; then \
	  backup="$$dst.bak.$(shell date -u +%Y%m%dT%H%M%SZ)"; \
	  cp "$$dst" "$$backup"; \
	  echo "已备份当前二进制: $$backup"; \
	fi; \
	install -m 755 "$$src" "$$dst"; \
	echo "已部署到: $$dst"; \
	"$$dst" --version; \
	if [ "$(BREW_RESTART)" = "1" ]; then \
	  $(MAKE) brew-restart; \
	else \
	  echo "跳过服务重启: BREW_RESTART=$(BREW_RESTART)"; \
	fi

brew-restart:
	@set -e; \
	if ! command -v brew >/dev/null 2>&1; then \
	  echo "未找到 brew，无法重启 Homebrew 服务" >&2; exit 1; \
	fi; \
	echo "重启 Homebrew 服务: $(BREW_SERVICE)"; \
	brew services restart "$(BREW_SERVICE)"; \
	echo "服务状态:"; \
	brew services info "$(BREW_SERVICE)" || true

# ===== Docker =====

docker:
	docker build -t galaxy-router:latest .

docker-run:
	docker run -p 8080:8080 -v $(PWD)/data:/app/data galaxy-router:latest

# ===== 帮助 =====

help:
	@echo "Galaxy Router - AI 协议互转代理网关"
	@echo ""
	@echo "用法: make [target]"
	@echo ""
	@echo "开发:"
	@echo "  build              开发构建"
	@echo "  dev                启动开发环境（前后端同时运行）"
	@echo "  dev-stop           停止残留的开发进程"
	@echo ""
	@echo "测试 / 检查:"
	@echo "  test               运行全部测试"
	@echo "  test-unit          单元测试（不含 HTTP 层）"
	@echo "  test-api           管理后台 API 回归"
	@echo "  test-proxy         代理转发回归"
	@echo "  test-regression    完整回归（API + 代理）"
	@echo "  test-ci            CI 完整流程（lint + 回归）"
	@echo "  fmt                格式化代码"
	@echo "  clippy             代码检查"
	@echo "  check              完整检查（lint+测试）"
	@echo ""
	@echo "发布:"
	@echo "  release            本机 release 构建"
	@echo "  release-version VERSION=x.y.z"
	@echo "                     同步 Cargo/前端版本并推送 git tag"
	@echo "  brew-deploy        构建并覆盖 Homebrew 部署二进制"
	@echo "                     可覆盖 BREW_BIN / BREW_SERVICE / BREW_RESTART"
	@echo ""
	@echo "运维:"
	@echo "  brew-restart       重启 Homebrew 服务"
	@echo "  docker             Docker 构建"
	@echo "  docker-run         Docker 运行"
	@echo "  clean              清理构建产物"
