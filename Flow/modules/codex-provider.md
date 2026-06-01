# 模块：Codex Provider

## 模块职责
把增强请求转换为 GPT `/chat/completions` HTTP 调用，负责请求组装、超时控制、错误映射、响应解析与输出清洗。

## 触发条件与输入
- 触发条件：增强入口已锁定 `provider=codex`
- 输入：
  - 当前 prompt 文本
  - 对话历史（首次增强可带 history；`continue` 分支默认清空 history）
  - `codex_api_base`
  - `codex_api_key`
  - `codex_model`
  - `enhance_timeout_sec`

## 核心处理步骤
1. 构造增强用 `system` / `user` 消息，要求只输出最终增强文本，不附带解释。
2. 组装 `/chat/completions` 请求体，注入 `model` 与低温度参数。
3. 通过 `Authorization: Bearer <api key>` 发起 HTTP 请求，并复用增强超时配置。
4. 根据 HTTP 状态码映射鉴权、限流、服务端错误与网络错误。
5. 解析 `choices[0].message.content`，兼容字符串或分段内容结构。
6. 清洗结果：去掉代码块、标签前缀、寒暄行，并在必要时自动做可读性换行。

## 输出结果
- 输出清洗后的增强文本
- 输出带分类信息的错误文本（鉴权失败、限流、超时、空结果、响应异常等）

## 异常与回退路径
- 条件：API base / key / model 缺失 -> 返回配置错误
- 条件：HTTP 401 / 403 / 429 / 5xx -> 返回对应错误
- 条件：网络超时或连接失败 -> 返回请求失败错误
- 条件：Codex 输出为空 -> 返回空结果错误

## 依赖模块
- 上游：增强入口编排
- 下游：本地确认会话
