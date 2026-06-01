# FLOW

更新时间：2026-06-01 14:20

## 适用范围与输入来源
- 模式：模块模式
- 目标范围：`enhance_prompt` 在 `codex` provider 下的调用链路
- 输入来源：`README.md`、`src/main.rs`、`src/config.rs`、`src/enhancer/codex_provider.rs`、`src/ui/session.rs`、`src/ui/window.rs`

## 主业务链路
MCP 客户端调用 `enhance_prompt` -> 参数与 prompt 归一化 -> 锁定 `codex` provider -> 调用 GPT `/chat/completions` 增强 -> 进入 Win32 确认会话或 headless 分支 -> 返回增强结果 / 原始 prompt / 结束对话

## 模块索引
- [启动与配置](./modules/startup-and-config.md)
- [增强入口编排](./modules/enhance-entry.md)
- [Codex Provider](./modules/codex-provider.md)
- [本地确认会话](./modules/prompt-session-ui.md)

## 关键依赖与约束
- `config::init_config` 要求 `--base-url` 与 `--token` 必填；`provider` 允许 `remote|codex`，当前链路只展开 `codex`
- `handle_enhance_prompt` 会清洗 `-enhance` / `-enhancer` 标记，并在必要时从 `conversation_history` 反推有效 prompt
- `provider` 以启动配置为准；请求参数中的 `provider` 若与当前配置不一致，会被忽略而不是动态切换
- `CodexProvider` 通过 HTTP 直连 GPT API，当前请求路径固定为 `/chat/completions`
- `codex` 模式依赖独立的 `codex api base`、`codex api key` 与可选 `model` 配置
- 非 headless 模式下先打开 Win32 窗口，再由后台线程执行首次增强；headless 模式下先增强，再按 `ACE_TOOL_HEADLESS_ACTION` 决定最终动作
- 交互窗口超时默认回退原始 prompt；headless 分支超时动作最终仍回退已增强文本

## 待确认事项
- 当前文档未展开 `remote` provider 的调用细节；若后续要做全景流程，需要补 `src/enhancer/enhancer.rs`
- 当前文档未展开 `search_context` 链路；该链路与本次 `codex` 模式主题无直接耦合
