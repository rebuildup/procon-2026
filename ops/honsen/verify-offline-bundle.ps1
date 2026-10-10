# Developer-side Windows PowerShell 5.1/7 validation; not required at the venue.
# Prerequisites: Python 3 and a prebuilt Windows/MSVC target/release/procon.exe.
[CmdletBinding()]
param(
    [string]$Exe = '',
    [int]$Port = 0
)

# Windows PowerShell 5.1 started from a shell that inherited PowerShell 7 module paths cannot autoload
# built-in cmdlets such as Get-FileHash. Drop the PowerShell 7 entries (not WindowsPowerShell) for this process.
if ($PSVersionTable.PSEdition -eq 'Desktop' -and $env:PSModulePath) {
    $env:PSModulePath = (($env:PSModulePath -split ';') | Where-Object { $_ -and ($_ -notmatch 'PowerShell' -or $_ -match 'WindowsPowerShell') }) -join ';'
}
$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Exe) { $Exe = Join-Path $RepoRoot 'target\release\procon.exe' }
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$Case = (Resolve-Path -LiteralPath (Join-Path $RepoRoot 'docs\evidence\campaign-311\fleet-371\corpus-hard\hard-16-a4-d10-short-s9001.json')).Path
$Mock = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'mock_server.py')).Path
$Launcher = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'honsen.ps1')).Path
$SmokeRoot = Join-Path $env:TEMP ('procon-honsen-bundle-test-' + [guid]::NewGuid().ToString('N'))
$Bin = Join-Path $SmokeRoot 'bin'
$Zip = Join-Path $SmokeRoot 'release.zip'
$SummaryFile = Join-Path $SmokeRoot 'mock-summary.json'
$ServerOut = Join-Path $SmokeRoot 'server.stdout.log'
$ServerErr = Join-Path $SmokeRoot 'server.stderr.log'
New-Item -ItemType Directory -Force $Bin | Out-Null

# Build only the same flat bin layout documented for the venue. Exercise it
# from a FRESH extraction so no checked-out target/ or source paths are used.
$PackageInput = Join-Path $SmokeRoot 'package-input'
New-Item -ItemType Directory -Force $PackageInput | Out-Null
Copy-Item -LiteralPath $Exe -Destination (Join-Path $PackageInput 'procon.exe')
Copy-Item -LiteralPath $Launcher -Destination (Join-Path $PackageInput 'honsen.ps1')
Compress-Archive -Path (Join-Path $PackageInput '*') -DestinationPath $Zip
Expand-Archive -LiteralPath $Zip -DestinationPath $Bin
$DistExe = Join-Path $Bin 'procon.exe'
$DistScript = Join-Path $Bin 'honsen.ps1'
$ExpectedSha = (Get-FileHash -LiteralPath $Exe -Algorithm SHA256).Hash
$ActualSha = (Get-FileHash -LiteralPath $DistExe -Algorithm SHA256).Hash
if ($ExpectedSha -ne $ActualSha) { throw 'extracted binary SHA256 mismatch' }
& $DistExe version
if ($LASTEXITCODE -ne 0) { throw 'extracted procon.exe version failed' }

if ($Port -eq 0) {
    $Tcp = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $Tcp.Start()
    try { $Port = $Tcp.LocalEndpoint.Port } finally { $Tcp.Stop() }
}
# Wrong env intentionally: .env at the bundle root must override it.
@("PROCON_SERVER=http://127.0.0.1:$Port", 'PROCON_TOKEN=local-e2e-token') | Set-Content -LiteralPath (Join-Path $SmokeRoot '.env') -Encoding UTF8
$env:PROCON_SERVER = 'http://127.0.0.1:65534'
$env:PROCON_TOKEN = 'incorrect-override'
Remove-Item Env:PROCON_SUPPLY_SLOT_WINDOW_MS -ErrorAction SilentlyContinue
Remove-Item Env:PROCON_TELEMETRY -ErrorAction SilentlyContinue
if (Get-Process -Name procon -ErrorAction SilentlyContinue) {
    throw 'another procon.exe is running; refuse to interfere with a live match'
}

$Server = $null
try {
    $PythonArgs = @(
        ('"{0}"' -f $Mock), '--case-file', ('"{0}"' -f $Case),
        '--port', "$Port", '--days', '3', '--day-seconds', '8',
        '--summary', ('"{0}"' -f $SummaryFile)
    )
    $Server = Start-Process -FilePath 'python' -ArgumentList $PythonArgs -PassThru -NoNewWindow -RedirectStandardOutput $ServerOut -RedirectStandardError $ServerErr
    # Cache the process handle now: without it, ExitCode reads as $null after WaitForExit(ms) on PS 5.1.
    $null = $Server.Handle
    Start-Sleep -Seconds 2
    if ($Server.HasExited) { throw 'mock server exited before preflight' }
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $DistScript preflight
    if ($LASTEXITCODE -ne 0) { throw 'extracted honsen.ps1 preflight failed' }
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $DistScript start -Candidate 'fleet:budget=official:budget_ms=1000'
    if ($LASTEXITCODE -ne 0) { throw 'extracted honsen.ps1 start failed' }
    if (-not $Server.WaitForExit(90000)) { throw 'mock server did not finish in 90 seconds' }
    if ($Server.ExitCode -ne 0) { throw "mock server rejected a day: exit $($Server.ExitCode)" }
    $Summary = Get-Content -LiteralPath $SummaryFile -Raw | ConvertFrom-Json
    if ((@($Summary.accepted_days) -join ',') -ne '0,1,2') {
        throw "expected days 0,1,2 accepted, got $(@($Summary.accepted_days) -join ',')"
    }
    if ($Summary.bad_token -ne 0) { throw 'bundle failed to override stale environment token' }
    $Runs = @(Get-ChildItem -LiteralPath (Join-Path $SmokeRoot 'records') -Directory)
    if ($Runs.Count -ne 1) { throw "expected 1 record run in bundle root, got $($Runs.Count)" }
    foreach ($Name in 'manifest.json', 'setting.json', 'events.jsonl', 'state.json', 'coordinator.log') {
        if (-not (Test-Path -LiteralPath (Join-Path $Runs[0].FullName $Name))) {
            throw "missing distributed-run record: $Name"
        }
    }
    $Http = Join-Path $SmokeRoot ("logs\{0}\http.jsonl" -f $Runs[0].Name)
    if (-not (Test-Path -LiteralPath $Http)) { throw 'missing distributed-run HTTP log' }
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $DistScript show
    if ($LASTEXITCODE -ne 0) { throw 'extracted honsen.ps1 show failed' }
    $Record = (& $DistExe record show $Runs[0].FullName | Out-String) | ConvertFrom-Json
    if ($Record.missing_seq -ne 0 -or $Record.unparsed_lines -ne 0) {
        throw 'record replay reported missing or malformed lines'
    }
    $Manifest = Get-Content -LiteralPath (Join-Path $Runs[0].FullName 'manifest.json') -Raw | ConvertFrom-Json
    if ($Manifest.binary.sha256 -ne $ActualSha.ToLower()) {
        throw 'manifest SHA does not match extracted exe'
    }
    Write-Host "PASS: extracted ZIP preflight/start/show; 3 accepted days; logs; hash=$ActualSha"
    Write-Host "Evidence: $SmokeRoot"
}
finally {
    if ($null -ne $Server -and -not $Server.HasExited) {
        Stop-Process -Id $Server.Id -Force -ErrorAction SilentlyContinue
    }
}
