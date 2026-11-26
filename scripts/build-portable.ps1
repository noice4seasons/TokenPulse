# TokenPulse Windows 11 x64 Portable Release Builder
$ErrorActionPreference = 'Stop'
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot

try {
    Write-Output "[1/4] Building Release tokenpulse.exe..."
    cargo build --release -p tokenpulse

    $exePath = Join-Path $repoRoot "target\release\tokenpulse.exe"
    $distRoot = Join-Path $repoRoot "dist"
    $stageDir = Join-Path $distRoot "TokenPulse-win11-x64-portable"
    $zipPath = Join-Path $distRoot "TokenPulse-win11-x64-portable.zip"

    if (Test-Path $stageDir) { Remove-Item -Recurse -Force $stageDir }
    if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
    New-Item -ItemType Directory -Force -Path $stageDir | Out-Null

    Write-Output "[2/4] Staging portable files..."
    Copy-Item -Force $exePath (Join-Path $stageDir "tokenpulse.exe")
    Copy-Item -Force (Join-Path $repoRoot "LICENSE") (Join-Path $stageDir "LICENSE")
    Copy-Item -Force (Join-Path $repoRoot "THIRD_PARTY_NOTICES.md") (Join-Path $stageDir "THIRD_PARTY_NOTICES.md")
    Copy-Item -Force (Join-Path $repoRoot "docs\security-and-privacy.md") (Join-Path $stageDir "SECURITY_AND_PRIVACY.md")

    $readmeTxt = @"
TokenPulse (Windows 11 x64 Portable)
====================================

Quick Start:
1. Double-click tokenpulse.exe (no admin privileges required).
2. A TokenPulse pulse-ring icon appears in the Windows system tray, along with the Direct2D compact floating overlay.
3. Click the tray icon or right-click -> "Open Detail Dashboard" to view full trends, 5-tier heatmap, model breakdown, and settings.
   Closing the detail window immediately destroys its dedicated WebView2 browser process.

Security & Privacy:
- 100% local read-only monitoring for OpenAI Codex & Google Antigravity logs.
- Zero outbound network crates, zero telemetry, zero extra model tokens.
- Never reads auth.json, OAuth tokens, or prompt/response bodies.
- See SECURITY_AND_PRIVACY.md and THIRD_PARTY_NOTICES.md for full details.

Note:
- Requires Windows 11 x64. The detail panel reuses the built-in Microsoft Edge WebView2 Runtime; if absent, the tray and Direct2D overlay continue operating normally without silent downloads.
"@
    Set-Content -Path (Join-Path $stageDir "README.txt") -Value $readmeTxt -Encoding UTF8

    Write-Output "[3/4] Compressing portable ZIP..."
    Compress-Archive -Path (Join-Path $stageDir "*") -DestinationPath $zipPath -CompressionLevel Optimal

    Write-Output "[4/4] Verifying package budget (< 10 MB) & SHA-256..."
    $exeInfo = Get-Item (Join-Path $stageDir "tokenpulse.exe")
    $zipInfo = Get-Item $zipPath
    $hash = (Get-FileHash -Algorithm SHA256 $zipPath).Hash
    Set-Content -Path "$zipPath.sha256" -Value "$hash  TokenPulse-win11-x64-portable.zip" -Encoding ASCII

    Write-Output ("EXE size : {0:N2} MB ({1} bytes)" -f ($exeInfo.Length / 1MB), $exeInfo.Length)
    Write-Output ("ZIP size : {0:N2} MB ({1} bytes)" -f ($zipInfo.Length / 1MB), $zipInfo.Length)
    Write-Output ("SHA-256  : $hash")

    if ($zipInfo.Length -ge 10MB) {
        Write-Error "Portable ZIP exceeds 10 MB hard limit!"
    }
    Write-Output "PORTABLE_BUILD_OK"
} finally {
    Pop-Location
}
