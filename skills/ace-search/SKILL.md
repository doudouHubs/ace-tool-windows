---
name: ace-search
description: Search the current codebase with ACE Tool when semantic or broad project context retrieval is useful.
---

# ACE Search

Use this skill when the user needs project-wide context, does not know exact file locations, or asks ACE Tool to search the codebase.

## Workflow

1. Resolve the target project root from the current workspace or explicit user path.
2. Write a focused natural-language query; include important identifiers as keywords when available.
3. Run `scripts/ace-search.ps1` from the plugin root.
4. Use the result as supporting context, then continue the user's actual task.

## Command

```powershell
$pluginRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
& (Join-Path $pluginRoot "scripts\ace-search.ps1") `
  -ProjectRoot "F:\path\to\project" `
  -Query "Where is the authentication flow implemented? Keywords: auth login token"
```

## Guardrails

- The script defaults to local provider with structured fallback so it can work without remote credentials.
- Do not use this for exact identifier grep; use repository search tools instead.
- Treat ACE output as context, not as authoritative proof; verify important claims by reading files.
