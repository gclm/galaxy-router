/**
 * 客户端配置生成器 —— 纯函数，无副作用、不依赖 React/DOM。
 * 把用户选的 key / group / 网关地址拼成 Claude Code / Codex 的客户端配置。
 *
 * 字段映射依据官方文档（核实于 2026-07-02）：
 *   - Claude Code: code.claude.com/docs/en/settings + /env-vars
 *   - Codex:       developers.openai.com/codex/config-advanced
 * 详见 .gclm-harness/tasks/2026-07-02-客户端配置生成器/design.md §2.4。
 */

/** Claude Code 配置输入。sonnet/opus/haiku 均为 galaxy 的 group.name（虚拟模型）。 */
export interface ClaudeInput {
  baseUrl: string
  apiKey: string
  sonnet: string
  opus: string
  haiku: string
  hideAttribution?: boolean
  effortMax?: boolean
  disableAutoUpdate?: boolean
}

/** Codex 配置输入。model 为 galaxy 的 group.name。 */
export interface CodexInput {
  baseUrl: string
  apiKey: string
  model: string
}

export interface ConfigFile {
  path: string
  content: string
}

/**
 * 生成 Claude Code 的 ~/.claude/settings.json 对象。
 *
 * - env: ANTHROPIC_BASE_URL / ANTHROPIC_API_KEY / 三档位模型（常驻）
 *        + DISABLE_AUTOUPDATER（disableAutoUpdate 勾选）
 *        + CLAUDE_CODE_EFFORT_LEVEL="max" 与 CLAUDE_CODE_ALWAYS_ENABLE_EFFORT="1"（effortMax 勾选；
 *          后者因 galaxy 走自定义 model ID，不加则 effort 参数不下发）
 * - 顶层 attribution: {commit:"", pr:""}（hideAttribution 勾选；空串隐藏署名）
 */
export function generateClaudeConfig(input: ClaudeInput): Record<string, unknown> {
  const env: Record<string, string> = {
    ANTHROPIC_BASE_URL: input.baseUrl,
    ANTHROPIC_API_KEY: input.apiKey,
    ANTHROPIC_DEFAULT_SONNET_MODEL: input.sonnet,
    ANTHROPIC_DEFAULT_OPUS_MODEL: input.opus,
    ANTHROPIC_DEFAULT_HAIKU_MODEL: input.haiku,
  }

  if (input.disableAutoUpdate) {
    env.DISABLE_AUTOUPDATER = '1'
  }

  if (input.effortMax) {
    env.CLAUDE_CODE_EFFORT_LEVEL = 'max'
    env.CLAUDE_CODE_ALWAYS_ENABLE_EFFORT = '1'
  }

  const config: Record<string, unknown> = { env }

  if (input.hideAttribution) {
    config.attribution = { commit: '', pr: '' }
  }

  return config
}

/** 去掉末尾斜杠后拼 /v1（Codex custom provider 的 base_url 指到 /v1，运行时再拼 /responses）。 */
function joinV1(baseUrl: string): string {
  return baseUrl.replace(/\/+$/, '') + '/v1'
}

/**
 * 生成 Codex 配置：~/.codex/config.toml + ~/.codex/.env（两文件）。
 *
 * custom provider 用 env_key 指向环境变量 GALAXY_API_KEY，key 不入 auth.json（官方推荐）。
 * wire_api = "responses"：最新版 Codex 默认走 Responses API，galaxy 的 /v1/responses 兼容。
 */
export function generateCodexConfig(input: CodexInput): { files: ConfigFile[] } {
  const base = joinV1(input.baseUrl)
  const toml =
    `model_provider = "galaxy"\n` +
    `model = "${input.model}"\n\n` +
    `[model_providers.galaxy]\n` +
    `name = "Galaxy Router"\n` +
    `base_url = "${base}"\n` +
    `env_key = "GALAXY_API_KEY"\n` +
    `wire_api = "responses"`
  const envFile = `GALAXY_API_KEY=${input.apiKey}`
  return {
    files: [
      { path: '~/.codex/config.toml', content: toml },
      { path: '~/.codex/.env', content: envFile },
    ],
  }
}
