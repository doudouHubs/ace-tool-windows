param(
  [Parameter(Mandatory = $true)]
  [string]$ProjectRoot,

  [Parameter(Mandatory = $true)]
  [string]$Prompt,

  [string]$ConversationHistory = "",

  [ValidateSet("text", "json")]
  [string]$Format = "text",

  [ValidateSet("remote", "codex")]
  [string]$Provider,

  [int]$EnhanceTimeoutSec,

  [int]$UiTimeoutSec,

  [string]$AceToolExe
)

$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$pluginRoot = Resolve-Path (Join-Path $scriptRoot "..")

if (-not $AceToolExe) {
  $AceToolExe = Join-Path $pluginRoot "bin\ace-tool-win.exe"
}

if (-not (Test-Path -LiteralPath $AceToolExe)) {
  throw "Missing ace-tool executable: $AceToolExe. Run scripts/build-bin.ps1 before using the plugin."
}

$arguments = @(
  "enhance",
  "--project-root", $ProjectRoot,
  "--prompt", $Prompt,
  "--conversation-history", $ConversationHistory,
  "--format", $Format
)

# 脚本参数仅作为临时覆盖；未显式传入时保持 config.json/env/内置默认值的解析链路。
if ($PSBoundParameters.ContainsKey("Provider")) {
  $arguments += @("--provider", $Provider)
}
if ($PSBoundParameters.ContainsKey("EnhanceTimeoutSec")) {
  $arguments += @("--enhance-timeout-sec", [string]$EnhanceTimeoutSec)
}
if ($PSBoundParameters.ContainsKey("UiTimeoutSec")) {
  $arguments += @("--ui-timeout-sec", [string]$UiTimeoutSec)
}

& $AceToolExe @arguments
if ($LASTEXITCODE -ne 0) {
  throw "ace-tool enhance failed with exit code $LASTEXITCODE"
}
