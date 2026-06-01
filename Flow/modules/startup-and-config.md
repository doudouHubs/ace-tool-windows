# 模块：启动与配置

## 模块职责
解析 CLI / 环境变量配置，初始化 MCP Server、Tokio Runtime 与工具分发器，为 `enhance_prompt` 的 `codex` 链路提供稳定运行参数。

## 触发条件与输入
- 触发条件：执行 `ace-tool-win` 进程并进入 `main()`
- 输入：
  - CLI：`--base-url`、`--token`、`--provider`、`--codex-api-base`、`--codex-api-key`、`--codex-model`、`--enhance-timeout-sec`、`--ui-timeout-sec`
  - 环境变量：`ACE_TOOL_ENHANCE_PROVIDER`、`ACE_TOOL_CODEX_API_BASE`、`ACE_TOOL_CODEX_API_KEY`、`ACE_TOOL_CODEX_MODEL`、`ACE_TOOL_ENHANCE_TIMEOUT_SEC`、`ACE_TOOL_UI_TIMEOUT_SEC`

## 核心处理步骤
1. `config::init_config` 解析 CLI 参数并回退到环境变量默认值。
2. 规范化 `base_url`；若传入 `http://`，自动改写为 `https://`。
3. 校验 `provider` 是否为 `remote|codex`，并生成最终 `Config`。
4. `main()` 初始化工具列表、Tokio Runtime、MCP handler 与 logging sender。
5. `McpServer::run()` 进入 stdio 循环，等待 `tools/call` 请求分发到 `handle_enhance_prompt`。

## 输出结果
- 输出可复用的 `Config`
- 输出 MCP 服务端运行循环
- 输出 `enhance_prompt` 可消费的运行时参数

## 异常与回退路径
- 条件：缺少 `--base-url` 或 `--token` -> `init_config` 返回错误，主进程打印错误并退出
- 条件：`provider` 非 `remote|codex` -> 返回配置错误，主进程退出
- 条件：超时参数超出允许范围 -> 回退到内置默认值

## 依赖模块
- 上游：无
- 下游：增强入口编排
