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

  [switch]$Headless,

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

$previousHeadless = $env:ACE_TOOL_HEADLESS
$previousHeadlessAction = $env:ACE_TOOL_HEADLESS_ACTION

try {
  if (-not $Headless) {
    # 插件正常交互路径必须显示窗口；历史测试环境变量只允许在显式 -Headless 时生效。
    Remove-Item Env:ACE_TOOL_HEADLESS -ErrorAction SilentlyContinue
    Remove-Item Env:ACE_TOOL_HEADLESS_ACTION -ErrorAction SilentlyContinue
  }

  & $AceToolExe @arguments
  $exitCode = $LASTEXITCODE
} finally {
  if ($null -ne $previousHeadless) {
    $env:ACE_TOOL_HEADLESS = $previousHeadless
  } else {
    Remove-Item Env:ACE_TOOL_HEADLESS -ErrorAction SilentlyContinue
  }

  if ($null -ne $previousHeadlessAction) {
    $env:ACE_TOOL_HEADLESS_ACTION = $previousHeadlessAction
  } else {
    Remove-Item Env:ACE_TOOL_HEADLESS_ACTION -ErrorAction SilentlyContinue
  }
}

if ($exitCode -ne 0) {
  throw "ace-tool enhance failed with exit code $exitCode"
}
