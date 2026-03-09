# ace-tool-windows

Windows 平台的 ACE MCP Server（Rust + Win32，stdio 模式），提供 `search_context` 与 `enhance_prompt`。

> 原项目：<https://github.com/eastxiaodong/ace-tool>

## 1. 安装

```powershell
npm i -g ace-tool-windows
```

## 2. 获取 Token

在 <https://acemcp.heroman.wtf> 生成并复制 `token`。

建议：
- 不要把 token 提交到 Git 仓库。
- 优先通过环境变量或本地 MCP 配置注入。
- 即便使用 `codex` provider，`search_context` 仍依赖远端服务，必须配置 token。

## 3. 模式速览（先选一个）

### 3.1 `remote` 模式
- 适合：不依赖本机 `codex` CLI，开箱可用。
- 依赖：远端 token/credits。
- 启动示例：

```powershell
ace-tool-win --base-url https://acemcp.heroman.wtf/relay/ --token <YOUR_TOKEN> --provider remote
```

### 3.2 `codex` 模式
- 适合：希望本地走 Codex 增强，避免远端增强额度影响。
- 依赖：本机可执行 `codex` CLI（`codex --version` 可用）。
- 启动示例：

```powershell
ace-tool-win --base-url https://acemcp.heroman.wtf/relay/ --token <YOUR_TOKEN> --provider codex --codex-cmd codex --codex-reasoning-effort low
```

关键说明：
- `search_context` 始终走远端，所以即便是 `codex` 模式也必须有可用 token。
- `codex_cmd` 推荐先用 `codex`（PATH 方案，跨设备更稳），确实找不到再写绝对路径。
- 交互窗口支持自适应布局；Codex 首轮超时会自动重试一次 `reasoning_effort=none`。

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
        "--codex-cmd", "codex",
        "--codex-reasoning-effort", "low"
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
  "--codex-cmd", "codex",
  "--codex-reasoning-effort", "low"
]
```

## 5. Provider 行为（当前版本）

- 启动时确定 provider：`--provider` / `ACE_TOOL_ENHANCE_PROVIDER`，默认 `remote`。
- 请求参数里的 `provider` 仅作为提示，不用于切换模式。
- 如果请求里的 `provider` 与启动 provider 不一致，会自动忽略，仍以启动 provider 为准。
- 不会再自动从 `codex` 降级到 `remote`。

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
$env:ACE_TOOL_CODEX_CMD = "codex"
$env:ACE_TOOL_CODEX_REASONING_EFFORT = "low"
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
- `codex` provider 在首轮超时时会自动重试一次（`reasoning_effort=none`），用于降低“首次增强卡住”的概率。

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

### 9.5 Codex provider 无法执行 / 退出码非 0

检查项：
- `--codex-cmd` 是否指向可执行文件，PATH 是否包含 `codex`。
- 使用相同命令在终端直接执行一次，确认 CLI 可运行。
- 开启 `ACE_TOOL_DEBUG=1` 查看退出码与错误类型（timeout/auth/network 等）。

### 9.6 Codex 首次增强慢 / 高概率超时

检查项：
- 日志是否长时间停在 `mcp: mcp-router starting`（已默认禁用内部 mcp-router；若仍出现，确认当前进程已重启到新版本）。
- 将 `--codex-reasoning-effort` 临时降到 `none` 或 `low` 观察改善幅度。
- 适当提高 `--enhance-timeout-sec`（例如 240）验证是否为纯耗时问题。

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
