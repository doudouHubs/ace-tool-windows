---
name: check-release
description: Run release build for ace-tool-windows (npm run release or cargo build --release), detect warnings/errors, and iteratively fix until 0 warnings/0 errors. Use when user requests release check, clean warnings, or production build verification.
---

# Check Release

## Workflow
1. Locate the release command:
   - If `package.json` has `scripts.release`, use `npm run release`.
   - Otherwise, if `Cargo.toml` exists, use `cargo build --release`.
2. Run the release build and capture all output.
3. Detect warnings/errors and fix them.
4. Re-run release build until warnings = 0 and errors = 0.
5. Report results (command used, counts, fixes applied).

## Preferred Helper Script
Run from the repo root:
- `powershell -ExecutionPolicy Bypass -File C:\Users\X1\.codex\skills\check-release\scripts\check_release.ps1`

The script prints exit code, warning count, error count, and writes a log file `check-release.log` in the repo root.

## Fix Loop Guidance
- Use `rg` to locate warning/error sources quickly.
- If `target/release/*.exe` is locked, close the running process before rebuilding.
- Keep build output clean: 0 warning, 0 error is required for completion.

## Output Expectations
- Summarize: command used, exit code, warning count, error count.
- List files changed and why.
- Confirm final clean build.