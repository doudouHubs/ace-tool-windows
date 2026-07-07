---
name: ace-enhance
description: Enhance a user prompt with ACE Tool when the latest request explicitly asks for prompt enhancement or contains -enhance / -enhancer.
---

# ACE Enhance

Use this skill only when the latest user message explicitly requests prompt enhancement or contains `-enhance` / `-enhancer`.

## Workflow

1. Resolve the target project root from the current workspace or explicit user path.
2. Pass the latest prompt and recent conversation history to `scripts/ace-enhance.ps1`.
3. If ACE returns an enhanced prompt, continue the original task in the same turn instead of stopping at the rewritten text.

## Command

```powershell
$pluginRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
& (Join-Path $pluginRoot "scripts\ace-enhance.ps1") `
  -ProjectRoot "F:\path\to\project" `
  -Prompt "Refactor this module -enhance" `
  -ConversationHistory "User: ..."
```

## Guardrails

- Do not trigger from historical `-enhance` markers; only the latest user request counts.
- Do not use this for ordinary optimization requests unless the user asks for prompt enhancement.
- Use `<project>/.ace-tool/config.json` or `~/.ace-tool/config.json` for provider/token/model settings instead of MCP configuration.
- If the UI is disabled, configure `ACE_TOOL_HEADLESS=1` and `ACE_TOOL_HEADLESS_ACTION`.
