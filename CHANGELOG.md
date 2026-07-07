# Changelog

## 0.3.2

- Changed enhance UI default wait behavior to infinite user confirmation.
- Added explicit `-Headless` script opt-in so normal plugin usage always opens the prompt window.
- Added enhance UI startup timing logs for diagnosing slow window launch.
- Kept finite UI timeout support for explicit `uiTimeoutSec` / `--ui-timeout-sec` values.

## 0.3.1

- Added local JSON configuration files for plugin runtime settings.
- Added user-level `~/.ace-tool/config.json` and project-level `<project>/.ace-tool/config.json` support.
- Preserved CLI overrides while letting local config override environment defaults.
- Updated plugin scripts so default parameters no longer mask local config values.
- Kept default search usable without remote credentials by using local structured fallback.
- Added config-backed debug logging and `codexReasoningEffort` support.

## 0.3.0

- Converted ACE Tool into the `ace-tool` Codex plugin.
- Removed MCP as the main runtime path.
- Added `ace-search` and `ace-enhance` skills backed by PowerShell scripts and the Windows CLI.
