param(
  [Parameter(Mandatory = $true)]
  [string]$ProjectRoot,

  [Parameter(Mandatory = $true)]
  [string]$Prompt,

  [string]$ConversationHistory = "",

  [ValidateSet("text", "json")]
  [string]$Format = "text",

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

& $AceToolExe enhance --project-root $ProjectRoot --prompt $Prompt --conversation-history $ConversationHistory --format $Format
if ($LASTEXITCODE -ne 0) {
  throw "ace-tool enhance failed with exit code $LASTEXITCODE"
}
