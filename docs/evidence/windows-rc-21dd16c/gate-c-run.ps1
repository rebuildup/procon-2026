# Gate C: mock 3-day match through ops/honsen/honsen.ps1 with the release binary.
# Local mock server only. Never talks to a production server.
[CmdletBinding()]
param(
    [string]$Candidate = 'sized:budget=official:budget_ms=2000:convoy_ms=1000:seed=42',
    [int]$Port = 18440,
    [int]$Days = 3
)
$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '.')).Path
$Case = Join-Path $RepoRoot 'docs\evidence\campaign-311\fleet-371\corpus-hard\hard-16-a4-d10-short-s9001.json'
$Summary = Join-Path $RepoRoot "gateC-mock-summary-$Port.json"
$OutLog = Join-Path $RepoRoot "gateC-server-$Port.out.log"
$ErrLog = Join-Path $RepoRoot "gateC-server-$Port.err.log"
$StartLog = Join-Path $RepoRoot "gateC-start-$Port.log"
Remove-Item $Summary, $OutLog, $ErrLog, $StartLog -Force -ErrorAction SilentlyContinue

Get-Process -Name procon -ErrorAction SilentlyContinue | ForEach-Object { throw "procon.exe already running (PID $($_.Id))" }

# .env must point at THIS run's mock port; a stale port makes preflight fail.
@("PROCON_SERVER=http://127.0.0.1:$Port", 'PROCON_TOKEN="local-e2e-token"') |
    Set-Content -LiteralPath (Join-Path $RepoRoot '.env') -Encoding UTF8
$env:PROCON_SERVER = "http://127.0.0.1:$Port"
$env:PROCON_TOKEN = 'local-e2e-token'

$before = @()
if (Test-Path (Join-Path $RepoRoot 'records')) {
    $before = @(Get-ChildItem (Join-Path $RepoRoot 'records') -Directory | ForEach-Object Name)
}

$server = Start-Process -FilePath 'python' -PassThru -NoNewWindow `
    -WorkingDirectory $RepoRoot `
    -ArgumentList @('ops/honsen/mock_server.py', '--case-file', $Case, '--port', "$Port",
        '--days', "$Days", '--day-seconds', '8', '--summary', $Summary) `
    -RedirectStandardOutput $OutLog -RedirectStandardError $ErrLog
Start-Sleep -Seconds 2
if ($server.HasExited) { throw 'mock server exited before the launcher ran' }

$psHost = if ($PSVersionTable.PSEdition -eq 'Core') { 'pwsh' } else { 'powershell' }
$code = 0
try {
    $output = & $psHost -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `
        (Join-Path $RepoRoot 'ops\honsen\honsen.ps1') start -Candidate $Candidate 2>&1 | Out-String
    $code = $LASTEXITCODE
}
finally {
    Set-Content -LiteralPath $StartLog -Value $output -Encoding UTF8
}
Write-Output "---- honsen.ps1 start exit=$code ----"
Write-Output $output

if (-not $server.WaitForExit(60000)) { $server.Kill(); throw 'mock server did not finish within 60s of the launcher' }
$mockExit = $server.ExitCode
$sum = Get-Content $Summary -Raw | ConvertFrom-Json
$expected = @(0..($Days - 1))
$accepted = @($sum.accepted_days)

$runDirs = @()
if (Test-Path (Join-Path $RepoRoot 'records')) {
    $runDirs = @(Get-ChildItem (Join-Path $RepoRoot 'records') -Directory | Sort-Object Name |
        Where-Object { ($before -notcontains $_.Name) -and (Test-Path (Join-Path $_.FullName 'events.jsonl')) })
}
$run = if ($runDirs.Count) { $runDirs[-1] } else { $null }

$failures = @()
if ($code -ne 0) { $failures += "honsen.ps1 start exited $code" }
if ($mockExit -ne 0) { $failures += "mock server exited $mockExit" }
if (($accepted -join ',') -ne ($expected -join ',')) { $failures += "accepted days = $($accepted -join ',')" }
if ($sum.bad_token -ne 0) { $failures += "bad_token = $($sum.bad_token)" }
if (-not $run) { $failures += 'no new record run directory' }

if ($run) {
    foreach ($f in 'manifest.json', 'setting.json', 'events.jsonl', 'state.json', 'coordinator.log') {
        if (-not (Test-Path (Join-Path $run.FullName $f))) { $failures += "record missing $f" }
    }
    $http = Join-Path $RepoRoot ("logs\{0}\http.jsonl" -f $run.Name)
    if (-not (Test-Path $http)) { $failures += 'http.jsonl missing' }
    else {
        $entries = @(Get-Content $http | ForEach-Object { $_ | ConvertFrom-Json })
        $posts = @($entries | Where-Object { $_.method -eq 'POST' -and $_.status -eq 200 -and $_.url -match ":$Port/$" })
        if ($posts.Count -lt $Days) { $failures += "accepted POSTs = $($posts.Count), expected $Days" }
        foreach ($p in $posts) {
            $body = [string]$p.request_body
            if (-not $body.StartsWith('[[') -or -not $body.EndsWith(']]')) { $failures += 'a logged plan is cut off' }
        }
        $gets = @($entries | Where-Object { $_.method -eq 'GET' -and $_.url -match '/setting' })
        if (-not $gets) { $failures += 'no GET /setting exchange logged' }
        $agents = @($entries | Where-Object { $_.method -eq 'POST' -and $_.url -match '/agent' })
        if (-not $agents) { $failures += 'no POST /agent exchange logged' }
    }
    $state = Get-Content (Join-Path $run.FullName 'state.json') -Raw | ConvertFrom-Json
    if ($state.status -ne 'completed') { $failures += "state.json status = $($state.status)" }
    $manifest = Get-Content (Join-Path $run.FullName 'manifest.json') -Raw | ConvertFrom-Json
    $exeHash = (Get-FileHash (Join-Path $RepoRoot 'target\release\procon.exe') -Algorithm SHA256).Hash.ToLower()
    if ($manifest.binary.sha256 -ne $exeHash) { $failures += 'manifest binary.sha256 != built exe' }
    if ($manifest.os -ne 'windows') { $failures += "manifest os = $($manifest.os)" }
    if ($manifest.agent_kinds.kinds.Count -lt 1) { $failures += 'manifest has no agent kinds' }
    if ($manifest.recorder.dropped -ne 0 -or $manifest.recorder.write_errors -ne 0) { $failures += 'recorder lost records' }
    $show = (& (Join-Path $RepoRoot 'target\release\procon.exe') record show $run.FullName | Out-String) | ConvertFrom-Json
    if ($show.unparsed_lines -ne 0) { $failures += "unparsed_lines = $($show.unparsed_lines)" }
    if ($show.missing_seq -ne 0) { $failures += "missing_seq = $($show.missing_seq)" }
    if (@($show.days).Count -ne $Days) { $failures += "record days = $(@($show.days).Count)" }
    foreach ($d in $show.days) {
        if ($d.status -ne 'accepted') { $failures += "day $($d.day) status $($d.status)" }
        if (-not $d.observed) { $failures += "day $($d.day) has no observed state" }
    }
    $leaks = @(Get-ChildItem (Join-Path $RepoRoot 'records'), (Join-Path $RepoRoot 'logs') -Recurse -File -ErrorAction SilentlyContinue |
        Select-String -SimpleMatch 'local-e2e-token')
    if ($leaks) { $failures += 'the token was written to disk' }
}

$psv = $PSVersionTable.PSVersion.ToString()
if ($failures.Count) {
    Write-Output "GATE-C($psv) FAIL: $($failures -join ' | ')"
    exit 1
}
Write-Output "GATE-C($psv) PASS: run=$($run.FullName) accepted=$($accepted -join ',') mockExit=$mockExit sha=$exeHash"
exit 0
