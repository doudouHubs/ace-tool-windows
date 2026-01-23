# rust-win32 ace-tool（Win32 GUI + MCP stdio）

本目录是 ace-tool 的 Rust + 纯 Win32 重写版本。目标是与 Node 版本功能 1:1 对齐，同时保持 MCP stdio 兼容，供 Codex 客户端调用。

## 对齐清单（必须与 Node 一致）

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

## 目录结构（规划）

```
rust-win32/
- Cargo.toml
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

## 使用方式

```bash
# 在 rust-win32/ 目录下
cargo run -- --base-url <URL> --token <TOKEN> [--enable-log]
```

## 备注

- 这是 Win32 GUI 应用，非 Windows 平台不支持运行。
- MCP 通信使用 stdio；Codex 需要指向该二进制作为 MCP Server。
- 本重写不会修改现有 Node 版本实现。