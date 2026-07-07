param(
  [Parameter(Mandatory = $true)]
  [string]$ProjectRoot,

  [Parameter(Mandatory = $true)]
  [string]$Query,

  [ValidateSet("text", "json")]
  [string]$Format = "text",

  [ValidateSet("remote", "local")]
  [string]$SearchProvider,

  [ValidateSet("gpt", "local_fallback_only")]
  [string]$LocalSummaryMode,

  [ValidateSet("off", "broad_only")]
  [string]$LocalRerankMode,

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
  "search",
  "--project-root", $ProjectRoot,
  "--query", $Query,
  "--format", $Format
)

# 只有调用方显式传入脚本参数时才转成 CLI 参数。
# 这样项目级/用户级 config.json 能真正接管默认值，避免脚本默认值把本地配置覆盖掉。
if ($PSBoundParameters.ContainsKey("SearchProvider")) {
  $arguments += @("--search-provider", $SearchProvider)
}
if ($PSBoundParameters.ContainsKey("LocalSummaryMode")) {
  $arguments += @("--local-summary-mode", $LocalSummaryMode)
}
if ($PSBoundParameters.ContainsKey("LocalRerankMode")) {
  $arguments += @("--local-rerank-mode", $LocalRerankMode)
}

& $AceToolExe @arguments
if ($LASTEXITCODE -ne 0) {
  throw "ace-tool search failed with exit code $LASTEXITCODE"
}
