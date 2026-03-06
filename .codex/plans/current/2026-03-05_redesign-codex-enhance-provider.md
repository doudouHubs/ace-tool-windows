# 任务计划：重设计 Codex 增强提示词功能（可切换 Remote/Codex）

## 元信息
- 计划 ID：plan-2026-03-05-codex-enhance-redesign
- 创建时间：2026-03-05T16:12:06+08:00
- 状态：执行中
- 预计复杂度：高
- 预计步骤数：7
- MCP 同步：是（clearAllTasks）

---

## 任务目标
在保持现有 `enhance_prompt` 默认行为与 MCP 兼容性的前提下，重构为可配置增强 Provider 架构，支持 `remote` 与 `codex` 两种模式，提升稳定性并消除中文乱码风险。

---

## 背景分析

### 现状（事实）
- 当前 `enhance_prompt` 主流程在 `src/main.rs`，直接依赖 `PromptEnhancer`（远端 API）。
- 当前 UI 会话在 `src/ui/session.rs` + `src/ui/window.rs`，已支持继续增强、发送增强、结束对话、超时回退。
- 当前 MCP schema 在 `src/mcp/schemas.rs`，`enhance_prompt` 必填参数为 `prompt` 与 `conversation_history`。
- 当前 README 已覆盖安装与 MCP 配置，但 provider 设计尚未成体系。

### 关键问题（事实 + 推论）
- 事实：增强来源与主流程强耦合，难以安全扩展到本地 Codex。
- 事实：用户历史反馈出现过中文显示 `???` 与偶发连接不可诊断问题。
- 推论：需要 provider 抽象、统一编码解码、统一日志语义，才能稳定支持双方案。

### 影响范围
- 涉及文件：`src/main.rs`、`src/config.rs`、`src/enhancer/*`、`src/ui/*`、`src/mcp/schemas.rs`、`src/utils/encoding.rs`、`README.md`
- 影响模块：增强调用链、配置解析、UI 会话、MCP 工具描述、运维排障

---

## 历史经验检查
- 项目级 lessons：未发现（`.codex/lessons` 不存在）。
- 全局 lessons 检索：当前环境无可用 `search_nodes` 能力，按本地代码事实规划。

---

## 实现方案

### 技术选型
- Provider 架构：新增 `EnhanceProvider` 抽象，`RemoteProvider` 复用现有远端逻辑，`CodexProvider` 承载本地 CLI 单次增强。
- 兼容策略：默认 `remote`，旧调用零改动可用。
- 编码策略：对外部命令输出统一 UTF-8 优先，补 BOM 清理与多编码兜底。

### 步骤分解

#### 步骤 1：建立增强 Provider 抽象与配置总线
- 状态：已完成 ✓
- 执行时间：2026-03-05T16:25:00+08:00
- AI 评分：84/100
- 目标：隔离增强来源，统一配置入口与优先级
- 涉及文件：
  - `src/enhancer/provider.rs`（新建）
  - `src/config.rs`（修改）
  - `src/main.rs`（修改）
- 验证：`cargo build --release` 通过；默认行为保持 remote

#### 步骤 2：重构远程增强实现为 RemoteProvider
- 状态：已完成 ✓
- 执行时间：2026-03-05T17:04:31+08:00
- AI 评分：90/100
- 目标：将现有远端逻辑挂入 provider 抽象，行为不变
- 涉及文件：
  - `src/enhancer/enhancer.rs`（修改）
  - `src/enhancer/mod.rs`（修改）
- 验证：同输入下返回一致；401/403/网络错误语义一致

#### 步骤 3：实现 CodexProvider 本地单次增强
- 状态：已完成 ✓
- 执行时间：2026-03-05T17:18:31+08:00
- AI 评分：88/100
- 目标：新增 codex 增强来源，支持超时、退出码、输出解码
- 涉及文件：
  - `src/enhancer/codex_provider.rs`（新建）
  - `src/utils/encoding.rs`（修改）
  - `Cargo.toml`（必要时修改）
- 验证：中文输入输出无乱码；异常可回退并有日志

#### 步骤 4：统一 UI 会话与 Provider 异步交互契约
- 状态：已完成 ✓
- 执行时间：2026-03-05T17:23:23+08:00
- AI 评分：86/100
- 目标：保证“先弹窗再等待增强”与交互一致性
- 涉及文件：
  - `src/ui/session.rs`（修改）
  - `src/ui/window.rs`（修改）
  - `src/main.rs`（修改）
- 验证：继续增强/发送增强/结束对话稳定可复现

#### 步骤 5：扩展 MCP schema 与调用参数兼容策略
- 状态：已完成 ✓
- 执行时间：2026-03-05T17:25:14+08:00
- AI 评分：92/100
- 目标：支持可选 provider 参数，保持旧调用兼容
- 涉及文件：
  - `src/mcp/schemas.rs`（修改）
  - `src/main.rs`（修改）
- 验证：`tools/list` 可见可选字段；旧调用成功

#### 步骤 6：补全日志与故障诊断信息
- 状态：已完成 ✓
- 执行时间：2026-03-05T17:28:37+08:00
- AI 评分：90/100
- 目标：定位超时、卡死、乱码与 provider 失败链路
- 涉及文件：
  - `src/main.rs`（修改）
  - `src/ui/window.rs`（修改）
  - `src/logging/mod.rs`（必要时修改）
- 验证：`ACE_TOOL_DEBUG=1` 下可追踪完整链路且无 token 泄露

#### 步骤 7：更新 README 与发布验证流程
- 状态：已完成 ✓
- 执行时间：2026-03-05T20:16:10+08:00
- AI 评分：88/100
- 目标：补齐安装、配置、双 provider 使用、排障、发布文档
- 涉及文件：
  - `README.md`（修改）
  - `package.json`（必要时修改）
- 验证：新机器可按文档配置并成功调用

---

## 风险评估
| 风险 | 可能性 | 影响 | 缓解措施 |
|------|--------|------|----------|
| Codex provider 响应慢导致体验差 | 中 | 中 | 独立 provider 超时、日志打点、UI 先展示 |
| 子进程输出编码异常导致中文乱码 | 高 | 高 | UTF-8 优先 + BOM 清理 + 编码兜底 |
| 重构导致旧 remote 行为漂移 | 中 | 高 | 默认 remote + 回归验证 + 错误映射保持一致 |
| 日志过量或泄露敏感信息 | 低 | 高 | 关键字段脱敏、仅输出必要摘要 |

---

## 验收标准
### 完成条件
- [ ] `enhance_prompt` 支持 provider 切换，默认 remote 不变
- [ ] codex provider 可在本地正常增强，中文不乱码
- [ ] UI 交互稳定：继续增强、发送增强、结束对话符合预期
- [ ] MCP schema 保持兼容，旧调用无破坏
- [ ] release 构建 0 error，尽量 0 warning
- [ ] README 完整覆盖安装、配置、排障、发布

### 验证方法
- 手动回归：remote / codex 各执行 3 轮增强（含中文）
- 协议验证：`tools/list` + `tools/call` 完整链路
- 构建验证：`npm run release` / `cargo build --release`

---

## 执行记录
| 时间 | 操作 | 结果 |
|------|------|------|
| 2026-03-05T16:12:06+08:00 | 创建计划并同步 MCP 任务（7 项） | 完成 |
| 2026-03-05T16:25:00+08:00 | 步骤 1：建立增强 Provider 抽象与配置总线 | 自检通过，MCP 已同步 |




| 2026-03-05T17:28:37+08:00 | 步骤 6：补全日志与故障诊断信息 | 自检通过，MCP 已同步 |
| 2026-03-05T20:16:10+08:00 | 步骤 7：更新 README 与发布验证流程 | 自检通过，MCP 已同步 |

---

## 用户确认
- [ ] 我已审阅并批准此计划

批准后执行：`$do-plan`





