param(
  [string]$Repo = (Get-Location).Path,
  [string]$LogPath = $(Join-Path (Get-Location).Path 'check-release.log')
)

$repoPath = (Resolve-Path $Repo).Path
Set-Location $repoPath

$command = $null
if (Test-Path (Join-Path $repoPath 'package.json')) {
  try {
    $pkg = Get-Content -Raw -Path (Join-Path $repoPath 'package.json') | ConvertFrom-Json
    if ($pkg.scripts -and $pkg.scripts.release) {
      $command = 'npm run release'
    }
  } catch {
  }
}

if (-not $command -and (Test-Path (Join-Path $repoPath 'Cargo.toml'))) {
  $command = 'cargo build --release'
}

if (-not $command) {
  Write-Error 'No release command found (package.json scripts.release or Cargo.toml).'
  exit 2
}

Write-Host "Running: $command"

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = 'cmd.exe'
$psi.Arguments = "/c $command"
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true

$proc = New-Object System.Diagnostics.Process
$proc.StartInfo = $psi
$null = $proc.Start()

$out = $proc.StandardOutput.ReadToEnd()
$err = $proc.StandardError.ReadToEnd()
$proc.WaitForExit()

$all = $out + $err

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($LogPath, $all, $utf8NoBom)

$warningCount = ([regex]::Matches($all, '(?im)\bwarning\b')).Count
$errorCount = ([regex]::Matches($all, '(?im)\berror\b|error\[')).Count

Write-Host "ExitCode: $($proc.ExitCode)"
Write-Host "Warnings: $warningCount"
Write-Host "Errors: $errorCount"
Write-Host "Log: $LogPath"

if ($proc.ExitCode -ne 0 -or $warningCount -gt 0 -or $errorCount -gt 0) {
  exit 1
}

exit 0