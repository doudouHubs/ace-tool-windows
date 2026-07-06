param(
  [Parameter(Mandatory = $true)]
  [string]$ProjectRoot,

  [Parameter(Mandatory = $true)]
  [string]$Query,

  [ValidateSet("text", "json")]
  [string]$Format = "text",

  [ValidateSet("remote", "local")]
  [string]$SearchProvider = "local",

  [ValidateSet("gpt", "local_fallback_only")]
  [string]$LocalSummaryMode = "local_fallback_only",

  [ValidateSet("off", "broad_only")]
  [string]$LocalRerankMode = "off",

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

& $AceToolExe search `
  --project-root $ProjectRoot `
  --query $Query `
  --format $Format `
  --search-provider $SearchProvider `
  --local-summary-mode $LocalSummaryMode `
  --local-rerank-mode $LocalRerankMode
if ($LASTEXITCODE -ne 0) {
  throw "ace-tool search failed with exit code $LASTEXITCODE"
}
