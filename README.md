# ace-tool-windows

Windows 平台的 ACE MCP Server（Rust + Win32，stdio 模式），提供 `search_context` 与 `enhance_prompt`。

> 原项目：<https://github.com/eastxiaodong/ace-tool>

## 1. 安装

```powershell
npm i -g ace-tool-windows
```

## 2. 获取 Token / API Key

在 <https://acemcp.heroman.wtf> 生成并复制 `token`，供 `search_context` 使用。

`codex` 模式下的增强请求改为直连 GPT API，需要额外准备：
- `codex api base`（例如 `http://your-gpt-gateway/v1`）
- `codex api key`
- 可选 `model`（默认 `gpt-5.4`）

建议：
- 不要把 token 提交到 Git 仓库。
- 优先通过环境变量或本地 MCP 配置注入。
- 即便使用 `codex` provider，`search_context` 仍依赖远端服务，必须配置 `--base-url` 与 `--token`。

## 3. 模式速览（先选一个）

### 3.1 `remote` 模式
- 适合：不依赖本机 `codex` CLI，开箱可用。
- 依赖：远端 token/credits。
- 启动示例：

```powershell
ace-tool-win --base-url https://acemcp.heroman.wtf/relay/ --token <YOUR_TOKEN> --provider remote
```

### 3.2 `codex` 模式
- 适合：希望增强链路直连 GPT API，减少本地 CLI 拉起与进程管理开销。
- 依赖：可访问的 GPT API 网关、有效 API Key、可用模型名。
- 启动示例：

```powershell
ace-tool-win --base-url https://acemcp.heroman.wtf/relay/ --token <YOUR_TOKEN> --provider codex --codex-api-base http://your-gpt-gateway/v1 --codex-api-key <YOUR_GPT_KEY> --codex-model gpt-5.4
```

关键说明：
- `search_context` 始终走远端，所以即便是 `codex` 模式也必须有可用 token。
- `codex` 模式现在通过 HTTP 直连 GPT API，不再依赖本机 `codex` CLI。
- 交互窗口支持自适应布局；增强结果仍会经过现有 UI 确认流程。

## 4. MCP 配置模板（按模式复制）

### 4.1 `remote` 模式模板

JSON（适用于支持 `mcpServers` 的客户端）：

```json
{
  "mcpServers": {
    "ace-tool-windows": {
      "command": "ace-tool-win",
      "args": [
        "--base-url", "https://acemcp.heroman.wtf/relay/",
        "--token", "<YOUR_TOKEN>",
        "--provider", "remote"
      ]
    }
  }
}
```

TOML（Codex CLI）：

```toml
[mcpServers."ace-tool-windows"]
command = "ace-tool-win"
args = [
  "--base-url", "https://acemcp.heroman.wtf/relay/",
  "--token", "<YOUR_TOKEN>",
  "--provider", "remote"
]
```

### 4.2 `codex` 模式模板

JSON（适用于支持 `mcpServers` 的客户端）：

```json
{
  "mcpServers": {
    "ace-tool-windows": {
      "command": "ace-tool-win",
      "args": [
        "--base-url", "https://acemcp.heroman.wtf/relay/",
        "--token", "<YOUR_TOKEN>",
        "--provider", "codex",
        "--codex-api-base", "http://your-gpt-gateway/v1",
        "--codex-api-key", "<YOUR_GPT_KEY>",
        "--codex-model", "gpt-5.4"
      ]
    }
  }
}
```

TOML（Codex CLI）：

```toml
[mcpServers."ace-tool-windows"]
command = "ace-tool-win"
args = [
  "--base-url", "https://acemcp.heroman.wtf/relay/",
  "--token", "<YOUR_TOKEN>",
  "--provider", "codex",
  "--codex-api-base", "http://your-gpt-gateway/v1",
  "--codex-api-key", "<YOUR_GPT_KEY>",
  "--codex-model", "gpt-5.4"
]
```

## 5. Provider 行为（当前版本）

- 启动时确定 provider：`--provider` / `ACE_TOOL_ENHANCE_PROVIDER`，默认 `remote`。
- 请求参数里的 `provider` 仅作为提示，不用于切换模式。
- 如果请求里的 `provider` 与启动 provider 不一致，会自动忽略，仍以启动 provider 为准。
- 不会再自动从 `codex` 降级到 `remote`。
- `codex` provider 当前通过 `/chat/completions` 直连 GPT API。

## 6. 在 AI CLI 中触发增强

配置好 MCP 后，输入包含 `-enhance` 或 `-enhancer` 的请求即可触发 `enhance_prompt`。

示例：

```text
为当前项目的代码添加详细注释 -enhance
```

补充：
- 用户输入里的触发后缀（`-enhance` / `-enhancer`）会在增强前自动剥离，不会进入最终增强文本。
- 若本次调用的 `prompt` 仅包含触发后缀，服务端会自动回退到 `conversation_history` 提取可用提示，不再直接报错中断。
- 工具入口统一为 `enhance_prompt`，避免同义工具名导致重复触发。
- 服务端会对短时间内完全相同的增强请求做去重（约 180 秒），避免“窗口刚关又被同参再次拉起”。
- 增强文本采用“语义自适应”排版：在不改变语义前提下提升细节和可读性，不强制固定模板。

## 7. 环境变量（按模式）

### 7.1 `remote` 模式

```powershell
$env:ACE_TOOL_ENHANCE_PROVIDER = "remote"
```

### 7.2 `codex` 模式

```powershell
$env:ACE_TOOL_ENHANCE_PROVIDER = "codex"
$env:ACE_TOOL_CODEX_API_BASE = "http://your-gpt-gateway/v1"
$env:ACE_TOOL_CODEX_API_KEY = "<YOUR_GPT_KEY>"
$env:ACE_TOOL_CODEX_MODEL = "gpt-5.4"
```

### 7.3 通用变量

- `ACE_TOOL_ENHANCE_TIMEOUT_SEC=90`（范围 10-600；未显式配置时 `remote` 默认 90 秒，`codex` 默认 180 秒）
- `ACE_TOOL_UI_TIMEOUT_SEC=480`（范围 30-3600）
- `ACE_TOOL_HEADLESS=1`
- `ACE_TOOL_HEADLESS_ACTION=enhanced|end|timeout`
- `ACE_TOOL_DEBUG=1`
- `ACE_TOOL_DEBUG_VERBOSE=1`
- `ACE_TOOL_DEBUG_FILE=<path>`

## 8. 超时规则

- `--enhance-timeout-sec` / `ACE_TOOL_ENHANCE_TIMEOUT_SEC`：增强请求超时，范围 10-600 秒；未显式配置时 `remote` 默认 90 秒，`codex` 默认 180 秒。
- `--ui-timeout-sec` / `ACE_TOOL_UI_TIMEOUT_SEC`：UI 会话超时（默认 480 秒，范围 30-3600）。
- 超出范围的配置会回退到默认值。
- 交互模式下 UI 超时会回退到原始 prompt；headless 模式不显示 UI，由 `ACE_TOOL_HEADLESS_ACTION` 决定动作。
- `codex` provider 的 HTTP 请求会复用上述增强超时。
- `codex` provider 会对 `429 / 502 / 503 / 504` 以及连接超时自动做最多 3 次重试。

## 9. 故障排查

### 9.1 MCP 启动超时 / 连接失败

检查项：
- 必须使用 stdio 模式。
- `command` 路径可执行（绝对路径优先）。
- `--base-url` 与 `--token` 是否正确。
- 开启 `ACE_TOOL_DEBUG=1` 查看日志链路。

### 9.2 窗口不弹出或卡住

检查项：
- 是否启用了 `ACE_TOOL_HEADLESS=1`。
- Windows 权限或安全软件是否拦截窗口创建。
- 调试日志中是否出现 `enhance_prompt: opening ui`。

### 9.3 Token 认证失败

典型返回：401 / 403。

处理：
- 到 <https://acemcp.heroman.wtf> 重新生成 token。
- 更新 MCP 配置后重启客户端。

### 9.4 中文显示异常 / 乱码

当前版本已增加 UTF-8/BOM/GBK 等解码兜底。若仍异常：
- 确认终端与编辑器编码为 UTF-8。
- 打开 `ACE_TOOL_DEBUG=1`，附带日志定位具体节点。

### 9.5 Codex provider 调用失败 / HTTP 返回异常

检查项：
- `--codex-api-base` / `ACE_TOOL_CODEX_API_BASE` 是否正确。
- `--codex-api-key` / `ACE_TOOL_CODEX_API_KEY` 是否有效。
- `--codex-model` / `ACE_TOOL_CODEX_MODEL` 是否为网关支持的模型名。
- 开启 `ACE_TOOL_DEBUG=1` 查看错误类型（timeout/auth/network/rate_limit 等）。

### 9.6 Codex 增强慢 / 高概率超时

检查项：
- GPT 网关本身是否响应慢，或上游模型负载是否过高。
- `--enhance-timeout-sec` 是否过低。
- 返回是否出现 429 / 5xx，确认是否为限流或服务端波动。
- 适当提高 `--enhance-timeout-sec`（例如 240）验证是否为纯耗时问题。

补充：
- 当前版本遇到 `429 / 502 / 503 / 504` 或连接超时会自动重试；若仍失败，通常说明上游服务确实不稳定，而不是本地参数拼错。

## 10. 从源码构建

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
- [ ] MCP 可启动并能成功 `tools/list`
- [ ] `enhance_prompt` 在 `remote` 与 `codex` 均可回包
- [ ] 中文增强回归通过（无 `???`）
- [ ] README 中配置示例可在新机器复现

## License

Apache-2.0
