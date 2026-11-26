# TokenPulse Phase 4 Privacy Sentinel & Release Verification Script
$ErrorActionPreference = 'Stop'
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
    Write-Output "=== [1/5] Cargo Workspace Tests ==="
    cargo test --workspace

    Write-Output "`n=== [2/5] Zero-Network Crate Audit (Cargo.lock) ==="
    $forbiddenCrates = 'name = "(reqwest|hyper|ureq|surf|curl|tokio|rustls|native-tls|openssl|tungstenite|surf|awc|isahc)"'
    $netHits = Select-String -Path "Cargo.lock" -Pattern $forbiddenCrates
    if ($netHits) {
        Write-Error ("Forbidden network crates detected in Cargo.lock: " + ($netHits | Out-String))
    }
    Write-Output "PASS: 0 network crates in Cargo.lock"

    Write-Output "`n=== [3/5] Release Binary Size & Static String Sentinel ==="
    $exePath = Join-Path $repoRoot "target\release\tokenpulse.exe"
    if (-not (Test-Path $exePath)) {
        cargo build --release -p tokenpulse
    }
    $exe = Get-Item $exePath
    $sizeMb = [math]::Round($exe.Length / 1MB, 2)
    Write-Output "Release tokenpulse.exe: $sizeMb MB ($($exe.Length) bytes)"
    if ($exe.Length -ge 10MB) {
        Write-Error "Release binary exceeds 10 MB hard limit!"
    }
    # Verify no forbidden ChatGPT cloud endpoint strings exist in binary
    $rawBytes = [System.IO.File]::ReadAllBytes($exePath)
    $ascii = [System.Text.Encoding]::ASCII.GetString($rawBytes)
    foreach ($needle in @("chatgpt.com/backend-api", "rate-limit-reset-credits", "GetCascadeTrajectory")) {
        if ($ascii.Contains($needle)) {
            Write-Error "Forbidden string detected in release binary: $needle"
        }
    }
    Write-Output "PASS: Binary size < 10 MB and 0 forbidden endpoints in binary"

    Write-Output "`n=== [4/5] Privacy Sentinel on Temporary SQLite DB & Export ==="
    $tmpDir = Join-Path $env:TEMP ("tokenpulse-sentinel-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
    $dbPath = Join-Path $tmpDir "sentinel.db"
    $fakeCodex = Join-Path $tmpDir "codex_home"
    $fakeSessions = Join-Path $fakeCodex "sessions"
    $fakeAgBrain = Join-Path $tmpDir "ag_brain"
    $fakeAgLogs = Join-Path $fakeAgBrain "conv-sentinel\.system_generated\logs"
    New-Item -ItemType Directory -Force -Path $fakeSessions, $fakeAgLogs | Out-Null

    $secretPrompt = "SENTINEL_SECRET_USER_PROMPT_DO_NOT_PERSIST_991827"
    $secretAuth = "SENTINEL_SECRET_OAUTH_TOKEN_XYZ_778899"
    Set-Content -Path (Join-Path $fakeCodex "auth.json") -Value "{`"access_token`":`"$secretAuth`"}" -Encoding UTF8

    $codexLines = @(
        '{"type":"session_meta","timestamp":"2026-11-26T10:00:00Z","payload":{"id":"sess-s1","model":"gpt-5.4"}}',
        "{`"type`":`"response_item`",`"timestamp`":`"2026-11-26T10:00:01Z`",`"payload`":{`"type`":`"message`",`"role`":`"user`",`"content`":[{`"type`":`"input_text`",`"text`":`"$secretPrompt`"}]}}",
        '{"type":"token_usage_record","timestamp":"2026-11-26T10:00:02Z","payload":{"turn_id":"t1","response_id":"resp-s1","usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":120,"reasoning_output_tokens":20},"thread_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":120,"reasoning_output_tokens":20},"turn_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"output_tokens":120,"reasoning_output_tokens":20}}}'
    )
    Set-Content -Path (Join-Path $fakeSessions "s1.jsonl") -Value $codexLines -Encoding UTF8

    $agLine = "{`"step_index`":1,`"type`":`"PLANNER_RESPONSE`",`"status`":`"DONE`",`"created_at`":`"2026-11-26T10:01:00Z`",`"model`":`"gemini-4-neon-high`",`"input_tokens`":500,`"cache_read_tokens`":4500,`"output_tokens`":150,`"content`":`"$secretPrompt`",`"thinking`":`"$secretAuth`"}"
    Set-Content -Path (Join-Path $fakeAgLogs "transcript.jsonl") -Value $agLine -Encoding UTF8

    $verifyOutFile = Join-Path $tmpDir "sentinel_verify.json"
    $verifyErrFile = Join-Path $tmpDir "sentinel_verify.err"
    $p1 = Start-Process -FilePath $exePath -ArgumentList @("--verify-all","--codex-home",$fakeCodex,"--antigravity-brain",$fakeAgBrain,"--db",$dbPath) -Wait -PassThru -NoNewWindow -RedirectStandardOutput $verifyOutFile -RedirectStandardError $verifyErrFile
    if ($p1.ExitCode -ne 0) {
        $errTxt = Get-Content $verifyErrFile -Raw -ErrorAction SilentlyContinue
        Write-Error "Sentinel --verify-all failed with exit code $($p1.ExitCode): $errTxt"
    }
    $verifyStr = Get-Content $verifyOutFile -Raw -Encoding UTF8
    foreach ($secret in @($secretPrompt, $secretAuth, "auth.json")) {
        if ($verifyStr.Contains($secret)) {
            Write-Error "PRIVACY LEAK in verify output: $secret"
        }
    }
    foreach ($artifact in Get-ChildItem $tmpDir -Filter "sentinel.db*") {
        $bytes = [System.IO.File]::ReadAllBytes($artifact.FullName)
        $text = [System.Text.Encoding]::UTF8.GetString($bytes)
        foreach ($secret in @($secretPrompt, $secretAuth, "auth.json")) {
            if ($text.Contains($secret)) {
                Write-Error "PRIVACY LEAK in $($artifact.Name): $secret"
            }
        }
    }
    Remove-Item -Recurse -Force $tmpDir
    Write-Output "PASS: Privacy sentinel 0 hits across DB, WAL, SHM, and JSON output"

    Write-Output "`n=== [5/5] Real Local Dual-Provider End-to-End Verification ==="
    $realOutFile = Join-Path $env:TEMP ("tokenpulse-real-verify-" + $PID + ".json")
    $realErrFile = Join-Path $env:TEMP ("tokenpulse-real-verify-" + $PID + ".err")
    $p2 = Start-Process -FilePath $exePath -ArgumentList @("--verify-all") -Wait -PassThru -NoNewWindow -RedirectStandardOutput $realOutFile -RedirectStandardError $realErrFile
    if ($p2.ExitCode -ne 0) {
        $errTxt = Get-Content $realErrFile -Raw -ErrorAction SilentlyContinue
        Write-Error "Real local --verify-all failed (exit=$($p2.ExitCode)): $errTxt"
    }
    $realObj = (Get-Content $realOutFile -Raw -Encoding UTF8) | ConvertFrom-Json
    Remove-Item -Force -ErrorAction SilentlyContinue $realOutFile, $realErrFile
    Write-Output ("PASS: passed=$($realObj.passed) | Codex=$($realObj.codex.round1_all_metrics.total_tokens) tokens (r2=$($realObj.codex.round2.elapsed_ms)ms) | Antigravity=$($realObj.antigravity.metrics.total_tokens) tokens (r2=$($realObj.antigravity.round2.elapsed_ms)ms)")
} finally {
    Pop-Location
}
