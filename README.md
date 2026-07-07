# ace-tool-windows

Windows 平台的 ACE Codex Plugin（Rust + PowerShell scripts），提供代码库检索与提示词增强。

> 原项目：<https://github.com/eastxiaodong/ace-tool>

## 1. 安装

```powershell
npm i -g ace-tool-windows
```

作为 Codex Plugin 使用时，插件名为 `ace-tool`。Codex 通过 `skills/` 触发 `scripts/`，脚本再调用 `bin/ace-tool-win.exe`，不再需要配置 MCP Server。

## 2. 获取 Token / API Key

在 <https://acemcp.heroman.wtf> 生成并复制 `token`，供 `remote` 检索或增强使用。

`codex` 模式下的增强请求改为直连 GPT API，需要额外准备：
- `codex api base`（例如 `http://your-gpt-gateway/v1`）
- `codex api key`
- 可选 `model`（默认 `gpt-5.4`）

建议：
- 不要把 token 提交到 Git 仓库。
- 优先写入用户级本地配置文件 `~/.ace-tool/config.json`。
- 如果使用 `search` 的 `remote` 模式，必须配置 `--base-url` 与 `--token`。
- 如果使用 `search` 的 `local` 模式且启用 GPT 总结/重排，需要可访问的 GPT API 网关与有效 API Key。

## 3. 模式速览

### 3.1 检索模式

- `remote`：沿用 ACE 远端检索服务。
- `local`：在项目根 `.ace-tool/local-search/` 中维护增量索引、chunk 分片和查询缓存，先做本地关键词召回，再用 GPT 总结或本地结构化降级输出。
- 默认：`local` + `local_fallback_only` + `rerank off`，无 token 也能返回结构化本地检索结果。

远端检索示例：

```powershell
ace-tool-win search --project-root <PROJECT_ROOT> --query "Where is auth implemented?" --base-url https://acemcp.heroman.wtf/relay/ --token <YOUR_TOKEN> --search-provider remote
```

本地检索示例：

```powershell
ace-tool-win search --project-root <PROJECT_ROOT> --query "Find local search implementation" --search-provider local --codex-api-base http://your-gpt-gateway/v1 --codex-api-key <YOUR_GPT_KEY> --codex-model gpt-5.4
```

本地检索产物会写入项目根目录：

```text
.ace-tool/local-search/
  meta.json
  files-manifest.json
  chunks/
  query-cache/
  rerank-cache/
```

说明：
- 本地检索默认不再依赖 `/embeddings`。
- 当前实现采用增量索引 + 关键词 BM25 风格召回；对宽查询会额外调用 `/chat/completions` 做 GPT rerank，再走总结。
- 若总结失败，可回退为结构化本地结果；也可显式配置为只走本地结构化结果。
- 本地索引会自动检测 `manifest/chunks/meta` 是否损坏；若发现缺块、坏 JSON 或旧版本索引，会自动清理并全量自愈重建。
- `.ace-tool` 持久化按项目管理；若运行时无法识别项目根，会拒绝写入磁盘状态，而不是回退写到用户目录。

### 3.2 增强模式

`remote` provider 依赖 ACE token，`codex` provider 通过 HTTP 直连 GPT API。

```powershell
ace-tool-win enhance --project-root <PROJECT_ROOT> --prompt "重构这个模块 -enhance" --provider codex --codex-api-base http://your-gpt-gateway/v1 --codex-api-key <YOUR_GPT_KEY> --codex-model gpt-5.4
```

关键说明：
- `search` 可以单独配置 `remote/local`，与增强 provider 解耦。
- `codex` 模式通过 HTTP 直连 GPT API，不依赖本机 `codex` CLI。
- 交互窗口支持自适应布局；增强结果仍会经过现有 UI 确认流程。

## 4. Codex Plugin 使用

插件提供两个 skill：
- `ace-search`：搜索代码库上下文。
- `ace-enhance`：在最新用户请求显式要求增强，或包含 `-enhance` / `-enhancer` 时增强提示词。

脚本入口：

```powershell
scripts\ace-search.ps1 -ProjectRoot <PROJECT_ROOT> -Query "Where is auth implemented?"
scripts\ace-enhance.ps1 -ProjectRoot <PROJECT_ROOT> -Prompt "重构这个模块 -enhance" -ConversationHistory "User: ..."
```

`ace-search.ps1` 默认使用 `local` provider、`local_fallback_only` 总结和关闭 rerank，因此没有远端 token 也能返回结构化本地检索结果。

CLI 入口：

```powershell
ace-tool-win search --project-root <PROJECT_ROOT> --query "Where is auth implemented?"
ace-tool-win enhance --project-root <PROJECT_ROOT> --prompt "重构这个模块 -enhance" --conversation-history "User: ..."
```

## 5. Provider 行为

- 启动时确定 provider：`--provider` / `ACE_TOOL_ENHANCE_PROVIDER`，默认 `remote`。
- 请求参数里的 `provider` 仅作为提示，不用于切换模式。
- 如果请求里的 `provider` 与启动 provider 不一致，会自动忽略，仍以启动 provider 为准。
- 不会自动从 `codex` 降级到 `remote`。
- `codex` provider 当前通过 `/chat/completions` 直连 GPT API。

## 6. 在 Codex 中触发增强

安装插件后，输入包含 `-enhance` 或 `-enhancer` 的请求即可触发 `ace-enhance`。

示例：

```text
为当前项目的代码添加详细注释 -enhance
```

补充：
- 用户输入里的触发后缀（`-enhance` / `-enhancer`）会在增强前自动剥离，不会进入最终增强文本。
- 若本次调用的 `prompt` 仅包含触发后缀，服务端会自动回退到 `conversation_history` 提取可用提示，不直接报错中断。
- 工具入口统一为 `ace-enhance`，避免同义入口导致重复触发。
- 服务端会对短时间内完全相同的增强请求做去重（约 180 秒）。
- 增强文本采用“语义自适应”排版：在不改变语义前提下提升细节和可读性，不强制固定模板。

## 7. 本地配置文件

插件模式不再使用 MCP Server 配置。运行时按以下优先级读取参数：

```text
CLI 参数 > 项目级 <PROJECT_ROOT>/.ace-tool/config.json > 用户级 ~/.ace-tool/config.json > 环境变量 > 内置默认值
```

推荐把跨项目密钥写到用户级配置：

```powershell
New-Item -ItemType Directory -Force "$HOME\.ace-tool" | Out-Null
@'
{
  "baseUrl": "https://acemcp.heroman.wtf/relay/",
  "token": "<YOUR_TOKEN>",
  "enhanceProvider": "codex",
  "codexApiBase": "http://your-gpt-gateway/v1",
  "codexApiKey": "<YOUR_GPT_KEY>",
  "codexModel": "gpt-5.4",
  "codexReasoningEffort": "low",
  "searchProvider": "local",
  "debug": true,
  "debugVerbose": true,
  "localSummaryMode": "local_fallback_only",
  "localRerankMode": "off",
  "searchTimeoutSec": 50,
  "enhanceTimeoutSec": 180,
  "uiTimeoutSec": 480
}
'@ | Set-Content -Path "$HOME\.ace-tool\config.json" -Encoding utf8
```

项目级配置写在当前仓库：

```text
<PROJECT_ROOT>/.ace-tool/config.json
```

项目级配置会覆盖用户级配置，适合给不同代码库设置不同检索策略。当前仓库的 `.gitignore` 已忽略 `.ace-tool/`，但你在其他项目里也要确认不要把 token/API key 提交出去，别把钥匙挂门口上。

支持字段：
- `baseUrl` / `token`：`remote` 检索和增强服务配置。
- `searchProvider`：`remote|local`，默认 `local`。
- `enhanceProvider`：`remote|codex`，默认 `remote`。
- `codexApiBase` / `codexApiKey` / `codexModel`：`codex` provider 和 GPT 总结/重排配置。
- `codexReasoningEffort`：`low|medium|high`，默认 `low`，用于 `codex` 增强和本地 GPT summary 请求。
- `debug` / `debugVerbose` / `debugFile`：调试日志开关、详细日志开关和日志文件路径。
- `localSummaryMode`：`gpt|local_fallback_only`，默认 `local_fallback_only`。
- `localIndexRebuild`：`auto|force_full`，默认 `auto`。
- `localRerankMode`：`off|broad_only`，默认 `off`。
- `localRerankPoolSize`：默认 `12`，范围 `4-32`。
- `localRerankTimeoutSec`：默认 `10`，范围 `3-120`。
- `localRerankModel`：未配置时跟随 `codexModel`。
- `searchTimeoutSec`：默认 `50`，范围 `10-300`。
- `enhanceTimeoutSec`：范围 `10-600`；未显式配置时 `remote` 默认 `90`，`codex` 至少 `180`。
- `uiTimeoutSec`：默认 `480`，范围 `30-3600`。
- `maxLinesPerBlob`：默认 `800`，范围 `50-5000`。
- `textExtensions`：可索引后缀列表，例如 `["rs", ".ts", ".md"]`。
- `excludePatterns`：索引排除模式列表。
- `enableLog`：是否启用项目级 `.ace-tool/ace-tool.log`。

字段也兼容 snake_case，例如 `codex_api_base`、`search_provider`。

## 8. 环境变量

### 8.1 `remote` 模式

```powershell
$env:ACE_TOOL_BASE_URL = "https://acemcp.heroman.wtf/relay/"
$env:ACE_TOOL_TOKEN = "<YOUR_TOKEN>"
$env:ACE_TOOL_ENHANCE_PROVIDER = "remote"
$env:ACE_TOOL_SEARCH_PROVIDER = "remote"
```

### 8.2 `codex` 模式

```powershell
$env:ACE_TOOL_ENHANCE_PROVIDER = "codex"
$env:ACE_TOOL_CODEX_API_BASE = "http://your-gpt-gateway/v1"
$env:ACE_TOOL_CODEX_API_KEY = "<YOUR_GPT_KEY>"
$env:ACE_TOOL_CODEX_MODEL = "gpt-5.4"
$env:ACE_TOOL_CODEX_REASONING_EFFORT = "low"
```

### 8.3 通用变量

- `ACE_TOOL_ENHANCE_TIMEOUT_SEC=90`（范围 10-600；未显式配置时 `remote` 默认 90 秒，`codex` 默认 180 秒）
- `ACE_TOOL_SEARCH_TIMEOUT_SEC=50`（范围 10-300；控制 `search` 整体超时）
- `ACE_TOOL_UI_TIMEOUT_SEC=480`（范围 30-3600）
- `ACE_TOOL_HEADLESS=1`
- `ACE_TOOL_HEADLESS_ACTION=enhanced|end|timeout`
- `ACE_TOOL_DEBUG=1`
- `ACE_TOOL_DEBUG_VERBOSE=1`
- `ACE_TOOL_DEBUG_FILE=<path>`
- `ACE_TOOL_SEARCH_PROVIDER=remote|local`
- `ACE_TOOL_LOCAL_SUMMARY_MODE=gpt|local_fallback_only`
- `ACE_TOOL_LOCAL_INDEX_REBUILD=auto|force_full`
- `ACE_TOOL_LOCAL_RERANK_MODE=off|broad_only`
- `ACE_TOOL_LOCAL_RERANK_POOL_SIZE=12`
- `ACE_TOOL_LOCAL_RERANK_TIMEOUT_SEC=10`
- `ACE_TOOL_LOCAL_RERANK_MODEL=gpt-5.4-mini`
- `ACE_TOOL_BATCH_SIZE=10`
- `ACE_TOOL_MAX_LINES_PER_BLOB=800`
- `ACE_TOOL_TEXT_EXTENSIONS=rs,ts,md`
- `ACE_TOOL_EXCLUDE_PATTERNS=node_modules,target,.git`
- `ACE_TOOL_ENABLE_LOG=1`

## 9. 超时规则

- `ACE_TOOL_SEARCH_TIMEOUT_SEC`：`search` 总超时，范围 10-300 秒，默认 50 秒。
- `ACE_TOOL_LOCAL_SUMMARY_MODE`：`local` 检索答案总结模式，默认 `local_fallback_only`；设为 `gpt` 时请求 GPT API 总结。
- `ACE_TOOL_LOCAL_INDEX_REBUILD`：`local` 检索索引刷新策略，默认 `auto`；设为 `force_full` 时每次强制全量重建本地索引。
- `ACE_TOOL_LOCAL_RERANK_MODE`：本地检索语义重排模式，默认 `off`；设为 `broad_only` 时仅宽查询触发 GPT rerank。
- `ACE_TOOL_LOCAL_RERANK_POOL_SIZE`：进入 rerank 前参与重排的候选池大小，默认 `12`，范围 `4-32`。
- `ACE_TOOL_LOCAL_RERANK_TIMEOUT_SEC`：单次 rerank 请求预算，默认 `10` 秒；仍受 `ACE_TOOL_SEARCH_TIMEOUT_SEC` 总预算约束。
- `ACE_TOOL_LOCAL_RERANK_MODEL`：rerank 专用模型名；未配置时跟随 `ACE_TOOL_CODEX_MODEL`。
- `local` 模式下会自动清理过期或损坏的查询缓存；默认缓存保留 7 天，并限制在最近 200 条以内。
- `--enhance-timeout-sec` / `ACE_TOOL_ENHANCE_TIMEOUT_SEC`：增强请求超时，范围 10-600 秒；未显式配置时 `remote` 默认 90 秒，`codex` 默认 180 秒。
- `--ui-timeout-sec` / `ACE_TOOL_UI_TIMEOUT_SEC`：UI 会话超时（默认 480 秒，范围 30-3600）。
- 超出范围的配置会回退到默认值。
- 交互模式下 UI 超时会回退到原始 prompt；headless 模式不显示 UI，由 `ACE_TOOL_HEADLESS_ACTION` 决定动作。
- `codex` provider 的 HTTP 请求会复用上述增强超时。
- `codex` provider 会对 `429 / 502 / 503 / 504` 以及连接超时自动做最多 3 次重试。

## 10. 故障排查

### 10.1 插件脚本启动失败

检查项：
- `bin/ace-tool-win.exe` 是否存在。
- 是否已运行 `npm run build:bin`。
- `--base-url` 与 `--token` 是否正确。
- 开启 `ACE_TOOL_DEBUG=1` 查看日志链路。

### 10.2 窗口不弹出或卡住

检查项：
- 是否启用了 `ACE_TOOL_HEADLESS=1`。
- Windows 权限或安全软件是否拦截窗口创建。
- 调试日志中是否出现 `enhance_prompt: opening ui`。

### 10.3 Token 认证失败

典型返回：401 / 403。

处理：
- 到 <https://acemcp.heroman.wtf> 重新生成 token。
- 更新环境变量或脚本参数后重试。

### 10.4 中文显示异常 / 乱码

当前版本已增加 UTF-8/BOM/GBK 等解码兜底。若仍异常：
- 确认终端与编辑器编码为 UTF-8。
- 打开 `ACE_TOOL_DEBUG=1`，附带日志定位具体节点。

### 10.5 Codex provider 调用失败 / HTTP 返回异常

检查项：
- `--codex-api-base` / `ACE_TOOL_CODEX_API_BASE` 是否正确。
- `--codex-api-key` / `ACE_TOOL_CODEX_API_KEY` 是否有效。
- `--codex-model` / `ACE_TOOL_CODEX_MODEL` 是否为网关支持的模型名。
- 开启 `ACE_TOOL_DEBUG=1` 查看错误类型（timeout/auth/network/rate_limit 等）。

## 11. 从源码构建

```powershell
# release
npm run release

# 构建并拷贝 bin/ace-tool-win.exe
npm run build:bin

# 本地打包验证
npm run pack:local
```

## 11. 发布到 npm

```powershell
npm publish --access public
```

> 若启用 npm 2FA，请使用支持 publish 的 token 或完成二次验证。

## 12. 发布前检查清单

- [ ] `npm run release` 无 error / warning
- [ ] `ace-tool-win search` 在 `remote` 与 `local` 均可回包
- [ ] `ace-tool-win enhance` 在 `remote` 与 `codex` 均可回包
- [ ] `.codex-plugin/plugin.json` 通过插件校验
- [ ] 中文增强回归通过（无 `???`）
- [ ] README 中配置示例可在新机器复现

## License

Apache-2.0
