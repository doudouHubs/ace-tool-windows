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

## 3. 本地快速验证

远端增强（`remote`，默认）：

```powershell
ace-tool-win --base-url https://acemcp.heroman.wtf/relay/ --token <YOUR_TOKEN>
```

本地 Codex 增强（需要已安装可执行的 `codex` CLI）：

```powershell
ace-tool-win --base-url https://acemcp.heroman.wtf/relay/ --token <YOUR_TOKEN> --provider codex --codex-cmd codex
```

可选参数：
- `--enable-log`：写入项目 `.ace-tool/ace-tool.log`
- `--provider remote|codex`：默认 `remote`
- `--codex-cmd codex`：Codex 命令，支持绝对路径
- `--codex-reasoning-effort <none|minimal|low|medium|high|xhigh>`：Codex 推理强度，默认 `low`
- `--enhance-timeout-sec 90`：增强调用超时（10-600 秒）；未显式配置时 `remote` 默认 90 秒，`codex` 默认 180 秒

Windows 说明：
- 当前版本不会写死单一路径：`args.codex_cmd` > 启动参数/环境变量 > 默认 `codex`，再按“直接路径 -> PATH 查找 -> Windows 常见位置（Volta/npm）”解析可执行文件。
- 调用 Codex 时默认附加 `-c "mcp_servers.mcp-router.enabled=false"`，避免 Codex 内部 `mcp-router` 启动抖动影响首轮增强。
- Codex 超时时会自动降级重试一次 `reasoning_effort=none`（仅在首轮超时时触发）。
- `--ui-timeout-sec 480`：UI 会话超时（30-3600 秒）
- UI 窗口支持自适应布局（拖拽/缩放窗口时，编辑区与按钮区域会自动重排）。

## 4. 配置为 MCP Server

### 通用 JSON（mcpServers）

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

### Claude Code（JSON）

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

### Codex CLI（TOML）

```toml
[mcpServers."ace-tool-windows"]
command = "ace-tool-win"
args = [
  "--base-url", "https://acemcp.heroman.wtf/relay/",
  "--token", "<YOUR_TOKEN>",
  "--provider", "remote"
]
```

如果使用 `codex` provider，请在 `args` 中追加：
- `--provider`, `codex`
- `--codex-cmd`, `<codex 可执行路径或命令>`

## 5. Provider 选择（remote / codex）

优先级：
1. 启动参数 `--provider` / 环境变量
2. 默认 `remote`
3. `enhance_prompt` 调用参数 `provider` 只做一致性校验，不用于切换

注意：
- 仅 `enhance_prompt` 使用 provider 配置；`search_context` 始终使用远端。
- `provider` 只允许 `remote` / `codex`，非法值会导致启动报错。
- 请求级 `provider` 不允许与启动 provider 不一致；不一致会直接报错并拒绝执行（默认强约束，避免会话中途切到 remote）。
- `codex` provider 需要本地 `codex` CLI 可执行（可用 `--codex-cmd` 指定路径）。
- `codex_cmd` 不是固定绝对路径，默认会按当前机器环境自动解析；跨设备建议先用 `codex`，失败再改绝对路径。

`enhance_prompt` 示例（可选 `provider`）：

```json
{
  "prompt": "为当前项目添加登录功能 -enhance",
  "conversation_history": "User: ...\nAssistant: ...",
  "project_root_path": "F:/GitlabProjects/ace-tool-windows",
  "provider": "codex",
  "codex_cmd": "C:/Users/X1/AppData/Local/Volta/bin/codex"
}
```

说明：
- `codex_cmd` 是 `enhance_prompt` 的可选外部覆盖参数，优先级高于启动参数和环境变量。
- 用户输入里的触发后缀（`-enhance` / `-enhancer`）会在增强前自动剥离，不会进入最终增强文本。

## 6. 在 AI CLI 中触发增强

配置好 MCP 后，输入包含 `-enhance` 或 `-enhancer` 的请求即可触发 `enhance_prompt`。

示例：

```text
为当前项目的代码添加详细注释 -enhance
```

补充：
- 增强文本默认遵循“语义自适应”的排版策略：优先保留原语义并细化细节；可用分段/小标题/列表提升可读性，但不强制固定模板。
- 当前工具名同时支持 `enhance_prompt` 与 `enhancer`（别名），用于提高不同客户端的工具路由命中率。

## 7. 常用环境变量

- `ACE_TOOL_ENHANCE_PROVIDER=remote|codex`
- `ACE_TOOL_CODEX_CMD=codex`
- `ACE_TOOL_CODEX_REASONING_EFFORT=low`
- `ACE_TOOL_ENHANCE_TIMEOUT_SEC=90`（未设置时 `remote` 默认 90 秒，`codex` 默认 180 秒）
- `ACE_TOOL_UI_TIMEOUT_SEC=480`
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
