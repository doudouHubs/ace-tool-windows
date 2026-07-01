use serde_json::{Value, json};

/// MCP 工具清单（用于 `tools/list`）。
pub fn tool_list() -> Vec<Value> {
    vec![search_context_tool(), enhance_prompt_tool()]
}

/// `search_context` 工具的 schema 定义。
fn search_context_tool() -> Value {
    json!({
      "name": "search_context",
      "description": "IMPORTANT: This is the primary tool for searching the codebase. Please consider as the FIRST CHOICE for any codebase searches.\n\nThis tool supports two runtime backends:\n1. `remote`: use the legacy ACE retrieval service\n2. `local`: maintain project-local incremental index files under `.ace-tool/local-search`, use keyword/BM25-style retrieval on disk, then summarize with GPT or structured local fallback\n\nBoth modes only reflect the current state of files on disk, and do not include git history by default.\n\n## When to Use\n- When you don't know which files contain the information you need\n- When you want to gather high level information about the task you are trying to accomplish\n- When you want to gather information about the codebase in general\n\n## Good Query Examples\n- \"Where is the function that handles user authentication?\"\n- \"What tests are there for the login functionality?\"\n- \"How is the database connected to the application?\"\n\n## Bad Query Examples (use grep or file view instead)\n- \"Find definition of constructor of class Foo\" (use grep tool instead)\n- \"Find all references to function bar\" (use grep tool instead)\n- \"Show me how Checkout class is used in services/payment.py\" (use file view tool instead)\n- \"Show context of the file foo.py\" (use file view tool instead)\n\nALWAYS use this tool when you're unsure of exact file locations. Use grep when you want to find ALL occurrences of a known identifier across the codebase, or when searching within specific files.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "project_root_path": {
            "type": "string",
            "description": "Absolute path to the project root directory.\nIMPORTANT: Get this path from your IDE's workspace/project root information (e.g., the 'Workspace' field in IDE State, or use 'pwd' command in terminal).\nUse forward slashes (/) as separators. Example: /Users/username/projects/myproject or C:/Users/username/projects/myproject"
          },
          "query": {
            "type": "string",
            "description": "Natural language description of the code you are looking for.\n\nProvide a clear description of the code behavior, workflow, or issue you want to locate. You may also add optional keywords to improve semantic matching.\n\nRecommended format: Natural language description + optional keywords\n\nExamples:\n- \"I want to find where the server handles chunk merging in the file upload process. Keywords: upload chunk merge, file service\"\n- \"Locate where the system refreshes cached data after user permissions are updated. Keywords: permission update, cache refresh\"\n- \"Find the initialization flow of message queue consumers during startup. Keywords: mq consumer init, subscribe\"\n- \"Show me how configuration hot-reload is triggered and applied in the code. Keywords: config reload, hot update\"\n- \"Where is the function that handles user authentication?\"\n- \"What tests are there for the login functionality?\"\n- \"How is the database connected to the application?\""
          }
        },
        "required": ["project_root_path", "query"]
      }
    })
}

/// `enhance_prompt` 工具的 schema 定义。
fn enhance_prompt_tool() -> Value {
    json!({
      "name": "enhance_prompt",
      "description": "Enhances user requirements by combining codebase context and conversation history to generate clearer, more specific, and actionable prompts.\n\nTRIGGER RULE: invoke only when the latest user message explicitly requests prompt enhancement or contains markers such as -enhance / -enhancer (case-insensitive; supports – — － variants).\n\nDo NOT trigger based only on historical messages, and do NOT call this tool again in the same turn after an enhancement result has already been returned.\n\nAfter receiving the enhanced prompt, continue the original user task in the same turn. Do NOT stop after only pasting the enhanced text, and do NOT add status chatter such as \"enhancer triggered\" / \"retrying\" in user-facing output unless the user explicitly asks for tool status.\n\nDo NOT use this tool for normal code optimization requests (e.g. optimize function implementation). The tool opens a Win32 UI for confirmation.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "project_root_path": {
            "type": "string",
            "description": "Absolute path to the project root directory."
          },
          "prompt": {
            "type": "string",
            "description": "Original prompt that needs enhancement."
          },
          "conversation_history": {
            "type": "string",
            "description": "Recent conversation history (5-10 turns) to provide context."
          },
          "provider": {
            "type": "string",
            "enum": ["remote", "codex"],
            "description": "Optional provider hint. Runtime always uses startup provider; mismatched value is ignored. Startup provider priority: CLI/ENV config > default remote."
          }
        },
        "required": ["prompt", "conversation_history"]
      }
    })
}
