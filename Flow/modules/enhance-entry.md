# 模块：增强入口编排

## 模块职责
接收 `enhance_prompt` 请求，完成 prompt 清洗、provider 锁定、重复调用去重、headless/交互分流，并把增强结果整理为最终返回值。

## 触发条件与输入
- 触发条件：MCP 客户端调用 `tools/call`，工具名为 `enhance_prompt`
- 输入：
  - `prompt`
  - `conversation_history`
  - 可选：`project_root_path`、`provider`

## 核心处理步骤
1. 读取 `prompt` 与 `conversation_history`，清洗 `-enhance` / `-enhancer` 标记。
2. 若清洗后的 prompt 为空，则从 `conversation_history` 反推最近可用的用户输入。
3. 使用 `resolve_provider_kind` 锁定当前启动时配置的 provider；请求里传入的不同 provider 只记录日志，不切换运行模式。
4. 计算基于项目路径、provider、prompt、history 的去重键；180 秒内相同调用直接返回缓存结果。
5. 当 provider 为 `codex` 时，创建 `CodexProvider`，并按是否 headless 走不同分支：
   - headless：先执行一次增强，再按 `ACE_TOOL_HEADLESS_ACTION` 组织最终结果
   - 交互：先打开 Win32 窗口，再由后台线程触发首次增强
6. 处理最终动作：返回增强文本、原始 prompt、`__END_CONVERSATION__`，或在特定分支上做超时回退。

## 输出结果
- 输出最终给 MCP 客户端的文本结果
- 输出最近一次增强结果缓存

## 异常与回退路径
- 条件：清洗后 prompt 与 history 都无法产出有效输入 -> 跳过增强，返回原始清洗结果
- 条件：provider 参数非法 -> 直接返回错误
- 条件：headless 分支增强超时 -> 返回带超时提示的原始 prompt
- 条件：交互窗口整体超时 -> 回退原始 prompt

## 依赖模块
- 上游：启动与配置
- 下游：Codex Provider、本地确认会话
