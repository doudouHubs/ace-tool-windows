# 模块：本地确认会话

## 模块职责
根据运行模式选择 headless 或 Win32 交互会话，对增强结果做人工确认、继续增强或超时回退处理。

## 触发条件与输入
- 触发条件：增强入口已进入结果确认阶段
- 输入：
  - 当前 prompt 或增强结果
  - `ContinueCallback`
  - `ACE_TOOL_HEADLESS`
  - `ACE_TOOL_HEADLESS_ACTION`
  - `ui_timeout_sec`

## 核心处理步骤
1. `run_prompt_session` 先判断是否启用 headless 模式。
2. headless 模式下直接按 `ACE_TOOL_HEADLESS_ACTION` 决定动作：
   - `enhanced` -> 使用增强文本
   - `end` -> 结束对话
   - `timeout` -> 走超时动作
3. 非 headless 模式下创建 Win32 窗口，初始化编辑框、倒计时、固定窗口状态与按钮事件。
4. 若 `auto_enhance=true`，窗口创建后立即在后台线程触发 `ContinueCallback` 执行增强，并把结果或错误回投到窗口线程。
5. 用户可选择 `发送增强`、`使用原始`、`继续增强`、`结束对话`；交互状态会根据加载态启停控件。
6. 窗口固定状态持久化到 `.ace-tool/pin.json`，下次会话自动复用。

## 输出结果
- 输出 `SessionAction::UseEnhanced`
- 输出 `SessionAction::UseOriginal`
- 输出 `SessionAction::EndConversation`
- 输出 `SessionAction::Timeout`

## 异常与回退路径
- 条件：headless 模式启用 -> 不创建窗口，直接返回 headless 动作
- 条件：后台增强失败 -> 弹出错误提示，用户可继续选择其他动作
- 条件：交互会话超时 -> 返回 `Timeout`，由上游回退原始 prompt

## 依赖模块
- 上游：增强入口编排、Codex Provider
- 下游：无
