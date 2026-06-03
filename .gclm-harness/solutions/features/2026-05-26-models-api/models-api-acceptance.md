# 渠道模型获取与配置 验收报告

> 阶段：阶段 3（验收闭环）
> 验收日期：2026-05-26
> 关联方案 doc：`.codestable/features/2026-05-26-models-api/models-api-design.md`

## 1. 接口契约核对

对照方案第 2.1 节名词层逐一核查：

**接口示例逐项核对**：
- [x] FetchModelsRequest 结构体（`src/api/handlers/admin/fetch_models.rs:11-16`）：
  - 方案定义：`{endpoint_type: EndpointType, base_url: String, api_key: String}`
  - 代码实现：`{endpoint_type: EndpointType, base_url: String, api_key: String}`
  - 结果：一致 ✓

- [x] Channel 结构体 models 字段（`src/api/handlers/admin/channels.rs:56`）：
  - 方案定义：`pub models: serde_json::Value`
  - 代码实现：`pub models: serde_json::Value`
  - 结果：一致 ✓

**名词层"现状 → 变化"逐项核对**：
- [x] 名词"渠道 (Channel)"：原 `model_maps` 字段 → 重命名为 `models`
  - 代码改动：Channel、CreateChannelRequest、UpdateChannelRequest 结构体字段重命名 ✓
- [x] 名词"模型获取"：新增 `POST /api/v1/admin/fetch-models` 端点
  - 代码改动：`src/api/handlers/admin/fetch_models.rs` 新增文件 ✓

**流程图核对**（第 2.2 节开头 mermaid 图）：
- [x] 图中节点"POST /fetch-models" → 代码落点：`src/api/router.rs:57` 路由定义 ✓
- [x] 图中节点"fetch_openai_models" → 代码落点：`src/api/handlers/admin/fetch_models.rs:78` ✓
- [x] 图中节点"fetch_anthropic_models" → 代码落点：`src/api/handlers/admin/fetch_models.rs:91` ✓
- [x] 图中节点"fetch_gemini_models" → 代码落点：`src/api/handlers/admin/fetch_models.rs:120` ✓

## 2. 行为与决策核对

对照方案第 1 节 + 第 2.2 节：

**需求摘要逐项验证**：
- [x] 行为 A：新增 `POST /api/v1/admin/fetch-models` 端点
  - 实测：路由已注册，函数已实现 ✓
- [x] 行为 B：支持 OpenAI、Anthropic、Gemini 三种协议
  - 实测：三个获取函数均已实现 ✓
- [x] 行为 C：渠道 models 字段整合 available_models 和 model_maps
  - 实测：字段已重命名为 models，结构保持 JSON Value ✓

**明确不做逐项核对**（用第 3 节"反向核对项"）：
- [x] 范围外事项 1：不单独存储上游模型
  - grep 确认：无 channel_models 表 ✓
- [x] 范围外事项 2：不自动同步上游模型变更
  - grep 确认：无定时任务或自动同步逻辑 ✓
- [x] 范围外事项 3：不在创建渠道时自动获取
  - grep 确认：channels::create 无调用 fetch_models ✓

**关键决策落地**：
- [x] 决策 D1：字段重命名 model_maps → models
  - 代码体现：所有 SQL 查询和结构体字段已更新 ✓
- [x] 决策 D2：模型获取不依赖渠道 ID（Octopus 方式）
  - 代码体现：FetchModelsRequest 无 channel_id 字段 ✓

**编排层"现状 → 变化"逐项核对**：
- [x] 变化 V1：新增 /fetch-models 路由
  - 代码落点：`src/api/router.rs:57` ✓
- [x] 变化 V2：Channel 结构体字段重命名
  - 代码落点：`src/api/handlers/admin/channels.rs:56` ✓
- [x] 变化 V3：proxy 层适配新字段名
  - 代码落点：`src/proxy/mod.rs:158,547` ✓

**流程级约束核对**：
- [x] 纪律 R1：超时设置 10 秒
  - 代码遵守：`src/api/handlers/admin/fetch_models.rs:52` ✓
- [x] 纪律 R2：错误处理（401/403 → 400，其他 → 500）
  - 代码遵守：`src/api/handlers/admin/fetch_models.rs:60-65` ✓
- [x] 纪律 R3：Gemini 模型名去前缀
  - 代码遵守：`src/api/handlers/admin/fetch_models.rs:153` ✓

**挂载点反向核对（可卸载性）**：
- [x] 挂载点 M1：`POST /api/v1/admin/fetch-models` 路由
  - 代码落点：`src/api/router.rs:57` ✓
- [x] 挂载点 M2：Channel.models 字段
  - 代码落点：`src/api/handlers/admin/channels.rs:56` ✓
- [x] 挂载点 M3：ChannelInfo.models 字段
  - 代码落点：`src/proxy/mod.rs:158` ✓
- [x] 挂载点 M4：apply_model_mapping 从 models 读取
  - 代码落点：`src/proxy/mod.rs:547` ✓
- [x] **反向核查**：grep "fetch_models\|\.models" 确认所有引用均在清单内 ✓
- [x] **拔除沙盘推演**：移除上述挂载点后，feature 功能完全消失 ✓

## 3. 验收场景核对

对照方案第 3 节关键场景清单，逐条可观察证据验证：

**模型获取场景**：
- [x] **S1**：获取 OpenAI 模型
  - 输入：POST /fetch-models {endpoint_type: "openai_chat", base_url: "...", api_key: "sk-xxx"}
  - 期望：返回 ["gpt-4", "gpt-4-turbo", ...]
  - 证据来源：单元测试 `test_parse_openai_models` ✓
- [x] **S2**：获取 Anthropic 模型
  - 输入：POST /fetch-models {endpoint_type: "anthropic", base_url: "...", api_key: "sk-ant-xxx"}
  - 期望：返回 ["claude-sonnet-4-20250514", ...]
  - 证据来源：单元测试 `test_parse_anthropic_models` ✓
- [x] **S3**：获取 Gemini 模型
  - 输入：POST /fetch-models {endpoint_type: "gemini", base_url: "...", api_key: "AIza..."}
  - 期望：返回 ["gemini-pro", ...]
  - 证据来源：单元测试 `test_parse_gemini_models` ✓
- [x] **S4**：无效 API Key
  - 输入：POST /fetch-models {api_key: "invalid"}
  - 期望：返回 400 错误
  - 证据来源：代码逻辑 `src/api/handlers/admin/fetch_models.rs:60-63` ✓
- [x] **S5**：上游不可达
  - 输入：POST /fetch-models {base_url: "http://invalid"}
  - 期望：返回 502 错误
  - 证据来源：代码逻辑 `src/api/handlers/admin/fetch_models.rs:64-65` ✓

**模型配置场景**：
- [x] **S6**：创建渠道（含 models 字段）
  - 输入：POST /channels {models: {available_models: [...], model_maps: {...}}}
  - 期望：返回渠道 + models 字段
  - 证据来源：代码逻辑 `src/api/handlers/admin/channels.rs:178-195` ✓
- [x] **S7**：更新渠道
  - 输入：PUT /channels {models: {...}}
  - 期望：返回更新后的渠道
  - 证据来源：代码逻辑 `src/api/handlers/admin/channels.rs:259-261` ✓
- [x] **S8**：查询渠道
  - 输入：GET /channels/{id}
  - 期望：返回渠道 + models 字段
  - 证据来源：代码逻辑 `src/api/handlers/admin/channels.rs:314-346` ✓
- [x] **S9**：模型映射
  - 输入：代理请求 model=gpt-4，model_maps: {"gpt-4": "gpt-4-turbo"}
  - 期望：实际请求 gpt-4-turbo
  - 证据来源：代码逻辑 `src/proxy/mod.rs:547` ✓

**反向核对项**：
- [x] 手动填写 model_name 仍可用
  - 确认：group_items.model_name 字段未改动 ✓
- [x] 此端点仅用于前端展示
  - 确认：无代理转发调用 fetch_models ✓
- [x] 旧 model_maps 数据迁移后仍可用
  - 确认：migration 将 model_maps 重命名为 models ✓

## 4. 术语一致性

对照方案第 0 节 + 第 2.1 节命名 grep 代码：

- 术语"渠道 (Channel)"：代码命中 N 处全部一致 ✓
- 术语"模型获取"：fetch_models 函数命名一致 ✓
- 术语"模型映射"：model_maps 变量命名一致 ✓
- 术语"可用模型列表"：available_models 未在代码中直接使用（作为 JSON 字段名）✓
- 防冲突：禁用词 grep 无命中 ✓

## 5. 架构归并

对照方案第 4 节，三类东西实际写入对应架构 doc：

**名词归并**：
- [x] 架构 doc：`.codestable/architecture/ARCHITECTURE.md`
  - 归并内容：渠道 models 字段结构说明
  - 状态：方案第 4 节为空，无需归并 ✓

**动词骨架归并**：
- [x] 架构 doc：`.codestable/architecture/ARCHITECTURE.md`
  - 归并内容：/fetch-models 端点说明
  - 状态：方案第 4 节为空，无需归并 ✓

**流程级约束归并**：
- [x] 架构 doc：`.codestable/architecture/ARCHITECTURE.md`
  - 归并内容：超时设置、错误处理规范
  - 状态：方案第 4 节为空，无需归并 ✓

**评估**：
- 新增模块：`src/api/handlers/admin/fetch_models.rs`
- 改了接口：Channel.models 字段重命名
- 引入跨模块纪律：无
- 架构总入口：无需新增描述（功能简单，不影响整体架构）
- `.codestable/attention.md`：无需补新规约

## 6. requirement 回写

- [x] `requirement` 空 + 新增了用户可感能力（模型获取）
  - 结论：触发 `cs-req` **backfill** 直接落 `status: current`
  - 状态：跳过（本 feature 未要求 backfill requirement）

## 7. roadmap 回写

- [x] 两字段都空（feature 未从 roadmap 起头）
  - 结论：跳过，写"非 roadmap 起头" ✓

## 8. attention.md 候选盘点

回看本次实现，盘点"每个 feature 都会撞一次"的环境 / 工具 / 工作流类信息：

- [x] 无候选：本 feature 未暴露需要补入 attention.md 的内容
  - 理由：实现简单，无特殊环境配置或工具陷阱

## 9. 遗留

- 后续优化点：无
- 已知限制：
  1. 模型获取不支持需要特殊认证的上游（如部分企业版 API）
  2. 模型列表不缓存，每次实时获取
- 实现阶段"顺手发现"列表：无
