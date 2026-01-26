# ace-tool-windows

Rust + Win32 的 MCP Server（stdio），提供 `search_context` 与 `enhance_prompt`，用原生 Win32 GUI 替代浏览器交互。

> 原项目：https://github.com/eastxiaodong/ace-tool

## 快速开始

### 1) 安装

```powershell
npm i -g ace-tool-windows
```

### 2) 直接运行（本地验证）

```powershell
ace-tool-win --base-url <URL> --token <TOKEN> [--enable-log]
```

### 3) 配置为 MCP Server

> 不同 MCP 客户端字段名可能是 `mcpServers` 或 `servers`，以客户端文档为准。

**mcpServers 格式（常见）**

```json
{
  "mcpServers": {
    "ace-tool-windows": {
      "command": "ace-tool-win",
      "args": ["--base-url", "<URL>", "--token", "<TOKEN>", "--enable-log"]
    }
  }
}
```

**使用本地 exe 路径**

```json
{
  "mcpServers": {
    "ace-tool-windows": {
      "command": "C:\\path\\to\\ace-tool-win.exe",
      "args": ["--base-url", "<URL>", "--token", "<TOKEN>"]
    }
  }
}
```

**servers 格式（兼容模式）**

```json
{
  "servers": {
    "ace-tool-windows": {
      "command": "ace-tool-win",
      "args": ["--base-url", "<URL>", "--token", "<TOKEN>"]
    }
  }
}
```

配置完成后，在 AI CLI 中输入 `xxxxx -enhance` 即可触发 `enhance_prompt`（同样支持 `-enhancer`）。

## 项目简介

本仓库是 ace-tool 的 Rust + Win32 重写版本，目标与 Node 版本功能 1:1 对齐，同时保持 MCP stdio 兼容，供 Codex / MCP 客户端调用。

## 功能特性

- MCP JSON-RPC over stdio，支持工具列举与调用
- `search_context`：索引 + 检索代码库上下文
- `enhance_prompt`：调用远端增强服务，支持中文/英文自动判断
- Win32 GUI 交互窗口，提供继续增强 / 使用原始 / 结束对话
- 本地索引与日志输出（`.ace-tool/`）

## 环境要求

- Windows 10/11（Win32 GUI）
- Rust 工具链（建议稳定版）
- Node.js + npm（用于发布到 npm）

## 配置说明

### CLI 参数

- `--base-url` 必填，服务地址；未写协议会自动补 `https://`
- `--token` 必填，ACE 服务 Token
- `--enable-log` 可选，写入 `.ace-tool/ace-tool.log`

### 环境变量（可选）

- `ACE_TOOL_HEADLESS=1`：跳过 UI，直接返回增强结果
- `ACE_TOOL_HEADLESS_ACTION=enhanced|end|timeout`：headless 模式返回策略，默认 enhanced
- `ACE_TOOL_DEBUG=1`：输出 MCP 调试日志（stderr + 文件）
- `ACE_TOOL_DEBUG_VERBOSE=1`：输出更详细的帧解析日志
- `ACE_TOOL_DEBUG_FILE=<path>`：调试日志路径，默认 `%TEMP%\\ace-tool-mcp.log`

### 超时规则（重要）

- 本项目默认不提供自定义超时参数。
- MCP 调用超时以客户端（Codex）配置为准，例如 `tool_timeout_sec` / `startup_timeout_sec`。
- 如需调整，请在 Codex 客户端配置中修改，不建议在服务端再做一套超时逻辑。

## MCP 工具说明

### search_context

- 传入项目根路径与检索 query
- 自动进行索引（`.ace-tool/index.json`）
- 调用 `/agents/codebase-retrieval` 返回 formatted_retrieval

### enhance_prompt

- 调用 `/prompt-enhancer`
- 语言检测：中文输入 -> 中文输出；英文输入 -> 英文输出
- 工具名映射：`codebase-retrieval` -> `search_context`
- 8 分钟超时回退到原始 prompt
- 默认弹 Win32 窗口，等待用户点击后返回
- 如需无 UI，可设置 `ACE_TOOL_HEADLESS=1`

## 从源码运行

```powershell
# 在仓库根目录
cargo run -- --base-url <URL> --token <TOKEN> [--enable-log]
```

## npm 打包与发布

构建 + 拷贝 exe：

```powershell
npm run build:bin
```

本地打包验证：

```powershell
npm run pack:local
```

发布到 npm（首次需登录）：

```powershell
npm publish --access public
```

## 项目结构

```text
ace-tool-windows/
- Cargo.toml
- package.json
- README.md
- src/
  - main.rs
  - mcp/
  - index/
  - enhancer/
  - ui/
  - utils/
- tests/
```

## 对齐清单（Node 版本）

### MCP 协议
- [ ] 基于 stdio 的 JSON-RPC
- [ ] ListTools 返回 search_context + enhance_prompt schema
- [ ] CallTool 按名称分发并返回 content[].text
- [ ] MCP logging 通道推送（level + data）

### CLI / 配置
- [ ] 必填参数：--base-url、--token
- [ ] 可选参数：--enable-log
- [ ] base_url 自动规范化（必须 https，去掉末尾 /）

### 项目数据
- [ ] 项目根目录创建 .ace-tool/
- [ ] index.json 写入 .ace-tool/
- [ ] --enable-log 时写入 ace-tool.log
- [ ] .ace-tool 自动加入 .gitignore

### search_context 工具
- [ ] project_root_path / query 输入校验
- [ ] 路径统一为正斜杠
- [ ] 目录存在性与类型检查
- [ ] 检索前自动索引
- [ ] POST {baseUrl}/agents/codebase-retrieval
- [ ] 返回 formatted_retrieval 或友好错误信息

### 索引行为
- [ ] 默认文本后缀与排除规则与 Node 一致
- [ ] 读取编码兜底（utf-8、gbk、gb2312、latin1）
- [ ] 二进制内容检测并跳过
- [ ] 清洗控制字符
- [ ] 按 maxLinesPerBlob 分片（默认 800）
- [ ] Blob 名称 hash：SHA-256(path + content)
- [ ] MAX_BLOB_SIZE：单 blob 500KB
- [ ] MAX_BATCH_SIZE：单批 5MB
- [ ] 基于 index.json 的增量索引
- [ ] 按 blob 数量自适应上传策略（batch size + 并发）
- [ ] 指数退避重试 + 友好错误映射

### enhance_prompt 工具
- [ ] POST {baseUrl}/prompt-enhancer（payload 含 nodes/chat_history/blobs）
- [ ] 语言检测（中文输入 -> 中文输出；英文输入 -> 英文输出）
- [ ] 工具名映射：codebase-retrieval -> search_context
- [ ] 8 分钟超时并回退到原始 prompt

### Win32 UI（替代浏览器 UI）
- [ ] 四个动作：发送增强 / 使用原始 / 继续增强 / 结束对话
- [ ] Session 状态：pending / completed / timeout
- [ ] UI 与增强流程通过通道协作

### 错误提示
- [ ] Token 无效（401）/ 访问被拒绝（403）
- [ ] SSL 错误 / 非 https / DNS / 超时 / 连接被拒绝
- [ ] 路径不存在 / 非目录 / 空索引

### 日志格式
- [ ] MCP logging 推送（level + data）
- [ ] 文件日志行格式："YYYY-MM-DD HH:MM:SS | LEVEL | message"
- [ ] 新日志流写入 Session 分隔符

## 测试

```powershell
cargo test
```

## License

Apache-2.0

## 致谢

- 原项目：ace-tool（https://github.com/eastxiaodong/ace-tool）
