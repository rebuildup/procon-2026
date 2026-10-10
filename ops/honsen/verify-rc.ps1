<#
.SYNOPSIS
    Windows 実機リリース候補（RC）検証のワンコマンド化（PR #438 添付シート準拠）。

.DESCRIPTION
    Windows 11 実機検証コマンドシート（release-0-5-1 基準）の項目を 1 コマンドで実行する。
    既存スクリプト（verify-offline-bundle.ps1 / honsen.ps1 / mock_server.py / e2e.py）を
    再利用し、検証ロジックは再実装しない。

      powershell -NoProfile -ExecutionPolicy Bypass -File ops\honsen\verify-rc.ps1
      pwsh -File ops/honsen/verify-rc.ps1 -OutDir /tmp/rc-verify-test   # macOS 側の構文 / SKIP 検証

    フェーズごとに PASS / FAIL / SKIP を分類し、ログを <OutDir>\<phase>.log に、
    サマリを <OutDir>\summary.json と <OutDir>\summary.txt に出力する。
    Windows 固有フェーズは macOS などでは SKIP になる。$IsWindows は PS 5.1 に存在しないので
    $env:OS -eq 'Windows_NT' で判定する（PS 5.1 / 7 両対応）。

    終了コード: FAIL が 1 つでもあれば 1、なければ 0。SKIP は PASS 扱いにしないが、
    件数をサマリに出す（未実施を PASS と書かないため）。

    フェーズ一覧:
      prereq          git / cargo / Python 3 の確認（OS 中立）
      build-release   cargo build --release --locked -p cli（OS 中立。Windows では MSVC）
      sha-check       HEAD SHA・exe SHA256 の照合（manifest binary.sha256 は e2e-mock で照合）
      tournament-basics cargo test --locked -p cli --test tournament_basic（#391 完了条件）
      bundle-e2e      PR #443 の verify-offline-bundle.ps1 を呼び出し（Windows 専用。未 merge なら SKIP）
      e2e-mock        ZIP 生成 → 展開 → mock_server.py で 3 日模擬試合 → 全日受理（シート §3 代替。OS 中立）
      e2e-official    e2e.py による公式簡易サーバー E2E（サーバー バイナリがある環境のみ。Windows では SKIP）
      double-start    honsen.ps1 の .honsen.lock による二重起動拒否と異常終了後の復帰（シート §5。OS 中立）
      telemetry       PROCON_TELEMETRY=8437 でモニター応答を検証（#436 未 merge / 非対応バイナリなら SKIP）
#>
[CmdletBinding()]
param(
    # ログとサマリの出力先。省略時は <リポジトリ直下>\verify-rc-logs。
    [string]$OutDir = '',
    # 期待する source commit SHA。指定したら HEAD と不一致のとき FAIL（省略時は記録のみ）。
    [string]$ExpectedSha = '',
    # cargo build を省略してこの既成バイナリを検証するときに指定。
    [string]$Exe = '',
    # e2e.py に渡す公式簡易サーバー バイナリ。省略時は ~/.procon-honsen/official-server を探す。
    # 見つからなければ e2e-official フェーズは SKIP になる（Windows 実機には配布されない）。
    [string]$ServerBinary = '',
    # 模擬サーバーのポート起点。0 = エフェメラル自動割当。
    [int]$Port = 0
)

# PS 5.1 は stderr に書くネイティブコマンドを 'Stop' の下でエラー扱いにすることがある
# （honsen.ps1 と同じ配慮）。失敗は throw と $LASTEXITCODE の明示チェックで扱う。
$ErrorActionPreference = 'Continue'

$script:IsWin = ($env:OS -eq 'Windows_NT')
$script:RepoRoot = (Resolve-Path -LiteralPath (Join-Path (Join-Path $PSScriptRoot '..') '..')).Path
$script:OutDir = $OutDir
if (-not $script:OutDir) { $script:OutDir = Join-Path $script:RepoRoot 'verify-rc-logs' }
$script:ExeParam = $Exe
$script:ExePath = ''
$script:ExeSha = ''
$script:HeadSha = ''
$script:Python = ''
$script:Results = @()
$script:PhaseDetail = ''
$script:LastHonsenExit = $null
$script:ServerBinaryPath = ''
$script:NextFixedPort = $Port
$script:HostPs = 'powershell.exe'
if (-not $script:IsWin) { $script:HostPs = 'pwsh' }
# e2e / telemetry が意図せずモニターを上げないように、外から来た設定は落としておく
Remove-Item Env:PROCON_TELEMETRY -ErrorAction SilentlyContinue

function Join-Under([string]$Base, [string[]]$Parts) {
    $path = $Base
    foreach ($part in $Parts) { $path = Join-Path $path $part }
    return $path
}

$script:CaseFile = (Resolve-Path -LiteralPath (Join-Under $script:RepoRoot @(
    'docs', 'evidence', 'campaign-311', 'fleet-371', 'corpus-hard', 'hard-16-a4-d10-short-s9001.json'))).Path
$script:RecordDir = Join-Under $script:RepoRoot @('records')
$script:LockPath = Join-Under $script:RepoRoot @('.honsen.lock')

New-Item -ItemType Directory -Force $script:OutDir | Out-Null

function Set-Detail([string]$Text) { $script:PhaseDetail = $Text }

function Assert-ExitZero([string]$Message) {
    if ($LASTEXITCODE -ne 0) { throw ("{0} (exit code {1})" -f $Message, $LASTEXITCODE) }
}

function Add-Result([string]$Phase, [string]$Status, [string]$Detail) {
    $script:Results += [pscustomobject]@{ phase = $Phase; status = $Status; detail = $Detail }
    $color = 'Yellow'
    if ($Status -eq 'PASS') { $color = 'Green' }
    if ($Status -eq 'FAIL') { $color = 'Red' }
    Write-Host ('[' + $Status + '] ' + $Phase + '  ' + $Detail) -ForegroundColor $color
}

function Invoke-Phase {
    param(
        [string]$Name,
        [scriptblock]$When = $null,
        [Parameter(Mandatory = $true)][scriptblock]$Body
    )
    $logPath = Join-Path $script:OutDir ($Name + '.log')
    $script:PhaseDetail = ''
    $skip = ''
    if ($null -ne $When) { $skip = [string](& $When) }
    if ($skip.Trim() -ne '') {
        Add-Content -LiteralPath $logPath -Value ('SKIP: ' + $skip) -Encoding UTF8
        Add-Result $Name 'SKIP' $skip
        return
    }
    Write-Host ('== phase: ' + $Name)
    Add-Content -LiteralPath $logPath -Value ('phase: ' + $Name + '  started: ' + (Get-Date -Format o)) -Encoding UTF8
    try {
        # ストリームごと 1 行ずつログに残す（途中で throw されてもそこまでの記録が残る）
        & $Body 2>&1 | ForEach-Object {
            $line = [string]$_
            Add-Content -LiteralPath $logPath -Value $line -Encoding UTF8
            Write-Host ('    ' + $line)
        }
        Add-Content -LiteralPath $logPath -Value 'RESULT: PASS' -Encoding UTF8
        Add-Result $Name 'PASS' $script:PhaseDetail
    }
    catch {
        $message = $_.Exception.Message
        Add-Content -LiteralPath $logPath -Value ('ERROR: ' + $message) -Encoding UTF8
        Add-Content -LiteralPath $logPath -Value 'RESULT: FAIL' -Encoding UTF8
        Write-Host ('    ERROR: ' + $message)
        Add-Result $Name 'FAIL' $message
    }
}

# PS 5.1 の Start-Process は引数配列を空白で結合するので、空白を含む引数だけ自前で引用符を付ける。
function ConvertTo-StartProcessArgs([string[]]$RawArgs) {
    $quoted = @()
    foreach ($arg in $RawArgs) {
        if ($arg -match '\s') { $quoted += ('"{0}"' -f $arg) } else { $quoted += $arg }
    }
    return $quoted
}

function Get-MockPort {
    if ($script:NextFixedPort -gt 0) {
        $port = $script:NextFixedPort
        $script:NextFixedPort = $port + 1
        return $port
    }
    $tcp = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 0)
    $tcp.Start()
    try { return $tcp.LocalEndpoint.Port } finally { $tcp.Stop() }
}

function Start-MockServer {
    param([int]$Port, [int]$Days, [string]$SummaryFile, [string]$StdoutFile, [string]$StderrFile)
    $mockPy = Join-Path $PSScriptRoot 'mock_server.py'
    $argList = @($mockPy, '--case-file', $script:CaseFile, '--port', [string]$Port,
        '--days', [string]$Days, '--day-seconds', '8', '--summary', $SummaryFile)
    if ($script:IsWin) { $argList = ConvertTo-StartProcessArgs $argList }
    $proc = Start-Process -FilePath $script:Python -ArgumentList $argList -PassThru -NoNewWindow `
        -RedirectStandardOutput $StdoutFile -RedirectStandardError $StderrFile
    Start-Sleep -Seconds 2
    if ($proc.HasExited) {
        throw ('mock server が起動直後に終了しました。stderr を確認してください: ' + $StderrFile)
    }
    return $proc
}

# exe + honsen.ps1 を ZIP にして別ディレクトリへ展開し、展開後 exe のパスを返す（シート §3-1）。
function New-DistBundle([string]$SmokeDir) {
    $exeName = 'procon'
    if ($script:IsWin) { $exeName = 'procon.exe' }
    if (Test-Path -LiteralPath $SmokeDir) { Remove-Item -LiteralPath $SmokeDir -Recurse -Force }
    $bin = Join-Path $SmokeDir 'bin'
    $pkg = Join-Path $SmokeDir 'package-input'
    New-Item -ItemType Directory -Force $bin, $pkg | Out-Null
    Copy-Item -LiteralPath $script:ExePath -Destination (Join-Path $pkg $exeName)
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'honsen.ps1') -Destination (Join-Path $pkg 'honsen.ps1')
    Compress-Archive -Path (Join-Path $pkg '*') -DestinationPath (Join-Path $SmokeDir 'release.zip')
    Expand-Archive -LiteralPath (Join-Path $SmokeDir 'release.zip') -DestinationPath $bin
    $distExe = Join-Path $bin $exeName
    # Expand-Archive が実行ビットを落とす環境があるため、非 Windows では付け直す
    if (-not $script:IsWin) { & chmod '+x' $distExe }
    $expected = (Get-FileHash -LiteralPath $script:ExePath -Algorithm SHA256).Hash
    $actual = (Get-FileHash -LiteralPath $distExe -Algorithm SHA256).Hash
    if ($expected -ne $actual) { throw 'ZIP 展開後の exe SHA256 が不一致です' }
    return $distExe
}

# honsen.ps1（preflight → start）を同期的に呼ぶ。出力はそのままフェーズログへ流す。
function Invoke-Honsen {
    param([string]$Command, [string]$ExePath, [string]$ServerUrl)
    $honsen = Join-Path $PSScriptRoot 'honsen.ps1'
    $lines = @(& $script:HostPs -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $honsen `
            $Command -Exe $ExePath -Server $ServerUrl -Candidate 'fleet:budget=official:budget_ms=1000' `
            -MaxRunMinutes 3 2>&1 | ForEach-Object { [string]$_ })
    $script:LastHonsenExit = $LASTEXITCODE
    Write-Output $lines
}

function Start-HonsenBackground {
    param([string]$ExePath, [string]$ServerUrl, [string]$StdoutFile, [string]$StderrFile)
    $honsen = Join-Path $PSScriptRoot 'honsen.ps1'
    $argList = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $honsen,
        'start', '-Exe', $ExePath, '-Server', $ServerUrl,
        '-Candidate', 'fleet:budget=official:budget_ms=1000', '-MaxRunMinutes', '3')
    if ($script:IsWin) { $argList = ConvertTo-StartProcessArgs $argList }
    return Start-Process -FilePath $script:HostPs -ArgumentList $argList -PassThru -NoNewWindow `
        -RedirectStandardOutput $StdoutFile -RedirectStandardError $StderrFile
}

# 模擬試合系フェーズの共通前提: exe があること、リポジトリ直下に .env がないこと。
# .env があると honsen.ps1 が本番サーバー設定を優先してしまい、模擬サーバーに届かない。
$MatchGuard = {
    if (-not $script:ExePath) { return '検証対象 exe が用意できませんでした（build-release が失敗した可能性）' }
    if (Test-Path -LiteralPath (Join-Under $script:RepoRoot @('.env'))) {
        return 'リポジトリ直下に .env があり、模擬サーバー設定が本番設定で上書きされるため SKIP。.env を退避してから再実行してください'
    }
    return ''
}

# ---- フェーズ 1: 前提ツール -------------------------------------------------
Invoke-Phase -Name 'prereq' -Body {
    Write-Output ('PowerShell: ' + $PSVersionTable.PSVersion.ToString() + ' (' + $PSVersionTable.PSEdition + ')')
    Write-Output ('OS: ' + $env:OS + '  ' + [System.Environment]::OSVersion.VersionString)
    foreach ($tool in @('git', 'cargo')) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw ($tool + ' が PATH にありません') }
        Write-Output ($tool + ': ' + [string](& $tool --version))
    }
    # Microsoft Store の python エイリアスは --version が空になるので、Python 3 の実体だけを受け入れる
    foreach ($cand in @('python3', 'python')) {
        if (-not (Get-Command $cand -ErrorAction SilentlyContinue)) { continue }
        $version = [string](& $cand --version 2>&1)
        if ($version -match 'Python\s+3') {
            $script:Python = $cand
            Write-Output ('python: ' + $version)
            break
        }
        Write-Output ($cand + ' は Python 3 ではないのでスキップ: ' + $version)
    }
    if (-not $script:Python) {
        throw 'Python 3.x が見つかりません（Microsoft Store エイリアスではなく python.org 版を入れてください）'
    }
    Write-Output ('リポジトリ: ' + $script:RepoRoot)
    Write-Output ('模擬試合ケース: ' + $script:CaseFile)
    Set-Detail ('git / cargo / Python 3（' + $script:Python + '）を確認')
}

# ---- フェーズ 2: release ビルド ---------------------------------------------
Invoke-Phase -Name 'build-release' -Body {
    $script:HeadSha = ([string](& git rev-parse HEAD)).Trim()
    $env:PROCON_SOURCE_COMMIT_SHA = $script:HeadSha
    Write-Output ('HEAD: ' + $script:HeadSha)
    if ($script:ExeParam) {
        $script:ExePath = (Resolve-Path -LiteralPath $script:ExeParam -ErrorAction Stop).Path
        Write-Output ('cargo build を省略し、指定バイナリを検証します: ' + $script:ExePath)
        Set-Detail ('exe: ' + $script:ExePath + '（-Exe 指定のためビルド省略）')
        return
    }
    Write-Output 'cargo build --release --locked -p cli'
    & cargo build --release --locked -p cli
    Assert-ExitZero 'cargo build --release --locked -p cli が失敗しました'
    $releaseDir = Join-Under $script:RepoRoot @('target', 'release')
    $candidate = Join-Path $releaseDir 'procon.exe'
    if (-not $script:IsWin) { $candidate = Join-Path $releaseDir 'procon' }
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw ('ビルド済みバイナリが見つかりません: ' + $candidate)
    }
    $script:ExePath = (Resolve-Path -LiteralPath $candidate -ErrorAction Stop).Path
    Set-Detail ('exe: ' + $script:ExePath)
}

# ---- フェーズ 3: SHA 照合（HEAD + exe。manifest は e2e-mock で照合） --------
Invoke-Phase -Name 'sha-check' -When {
    if (-not $script:ExePath) { return '検証対象 exe が用意できませんでした（build-release が失敗した可能性）' }
    return ''
} -Body {
    $head = ([string](& git rev-parse HEAD)).Trim()
    Write-Output ('HEAD SHA: ' + $head)
    if ($ExpectedSha) {
        if ($head -ne $ExpectedSha) {
            throw ('HEAD SHA が期待値と不一致です: HEAD=' + $head + ' 期待値=' + $ExpectedSha)
        }
        Write-Output ('HEAD SHA は期待値（' + $ExpectedSha + '）と一致しました')
    }
    $sha = (Get-FileHash -LiteralPath $script:ExePath -Algorithm SHA256).Hash
    Write-Output ('exe SHA256: ' + $sha)
    Set-Content -LiteralPath (Join-Path $script:OutDir 'exe-sha256.txt') -Value $sha -Encoding ASCII
    # version の JSON 1 行（シート §1 の PASS 判定）
    $version = @(& $script:ExePath version | ForEach-Object { [string]$_ })
    Assert-ExitZero 'procon version が失敗しました'
    $versionLine = @($version | Where-Object { $_ -match '^\s*\{' }) | Select-Object -Last 1
    if (-not $versionLine) { throw ('version の出力に JSON 行がありません: ' + ($version -join ' | ')) }
    $versionObj = $versionLine | ConvertFrom-Json
    if (-not $versionObj.package) { throw 'version JSON に package フィールドがありません' }
    Write-Output ('version: ' + $versionLine)
    $script:ExeSha = $sha
    Set-Detail ('exe sha256 ' + $sha.Substring(0, 12) + '...（HEAD と version JSON を記録済み）')
}

# ---- フェーズ 4: #391 完了条件の tournament_basic ---------------------------
Invoke-Phase -Name 'tournament-basics' -When {
    if (-not $script:ExePath) { return '検証対象 exe が用意できませんでした（build-release が失敗した可能性）' }
    return ''
} -Body {
    Write-Output 'cargo test --locked -p cli --test tournament_basic'
    $out = @(& cargo test --locked -p cli --test tournament_basic 2>&1 | ForEach-Object { [string]$_ })
    $out | ForEach-Object { Write-Output $_ }
    Assert-ExitZero 'cargo test --locked -p cli --test tournament_basic が失敗しました'
    $resultLines = @($out | Where-Object { $_ -match '^test result: ' })
    if ($resultLines.Count -eq 0) { throw 'test result 行が読み取れませんでした' }
    foreach ($line in $resultLines) {
        # -match は大文字小文字を無視するので、FAILED の検出は -cmatch で行う（'0 failed' を誤爆しない）
        if ($line -cmatch 'FAILED') { throw ('テストが失敗しました: ' + $line) }
    }
    $okLine = @($resultLines | Where-Object { $_ -match '^test result: ok\.' }) | Select-Object -Last 1
    if (-not $okLine) { throw 'test result: ok がありません' }
    $passed = 0
    if ($okLine -match '^test result: ok\. (\d+) passed') { $passed = [int]$Matches[1] }
    Write-Output ('tournament_basic: ' + $okLine)
    Set-Detail ('ok. ' + $passed + ' passed; 0 failed')
}

# ---- フェーズ 5: バンドル検証（PR #443 の verify-offline-bundle.ps1 を再利用） ----
Invoke-Phase -Name 'bundle-e2e' -When {
    if (-not $script:IsWin) {
        return 'Windows 実機専用フェーズです（macOS では代替フェーズ e2e-mock が ZIP 生成→展開→模擬試合を代行）'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'verify-offline-bundle.ps1'))) {
        return 'PR #443 未 merge のため ops/honsen/verify-offline-bundle.ps1 が存在しません（代替フェーズ e2e-mock で代行）'
    }
    if (-not $script:ExePath) { return '検証対象 exe が用意できませんでした（build-release が失敗した可能性）' }
    return ''
} -Body {
    Write-Output 'verify-offline-bundle.ps1: ZIP 生成 → 別ディレクトリ展開 → preflight/start/show → 3 日受理 → manifest SHA'
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass `
        -File (Join-Path $PSScriptRoot 'verify-offline-bundle.ps1') -Exe $script:ExePath
    Assert-ExitZero 'verify-offline-bundle.ps1 が失敗しました'
    Set-Detail 'verify-offline-bundle.ps1 が PASS（ZIP→展開→preflight/start/show→3 日受理→manifest SHA）'
}

# ---- フェーズ 6: 模擬試合 3 日全日受理（シート §3 代替。OS 中立） -----------
Invoke-Phase -Name 'e2e-mock' -When $MatchGuard -Body {
    $smoke = Join-Path $script:OutDir 'e2e-smoke'
    $distExe = New-DistBundle $smoke
    Write-Output ('展開バンドル: ' + $distExe)
    $port = Get-MockPort
    $summaryFile = Join-Path $smoke 'mock-summary.json'
    $server = Start-MockServer -Port $port -Days 3 -SummaryFile $summaryFile `
        -StdoutFile (Join-Path $smoke 'server.stdout.log') -StderrFile (Join-Path $smoke 'server.stderr.log')
    try {
        $env:PROCON_TOKEN = 'local-e2e-token'
        $before = @(Get-ChildItem -LiteralPath $script:RecordDir -Directory -ErrorAction SilentlyContinue |
            ForEach-Object { $_.Name })
        Write-Output ('== honsen.ps1 start（展開 exe、模擬サーバー 3 日、ポート ' + $port + '）')
        Invoke-Honsen -Command 'start' -ExePath $distExe -ServerUrl ('http://127.0.0.1:' + $port)
        if ($script:LastHonsenExit -ne 0) {
            throw ('honsen.ps1 start が異常終了しました: exit ' + $script:LastHonsenExit)
        }
        if (-not $server.WaitForExit(30000)) {
            Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
            throw 'mock server が試合後も終了しません'
        }
        if ($server.ExitCode -ne 0) { throw ('mock server が受理を拒否しました: exit ' + $server.ExitCode) }
        $summary = Get-Content -LiteralPath $summaryFile -Raw -ErrorAction Stop | ConvertFrom-Json
        $accepted = @($summary.accepted_days) -join ','
        if ($accepted -ne '0,1,2') { throw ('accepted_days が 0,1,2 ではありません: ' + $accepted) }
        if ($summary.bad_token -ne 0) { throw 'bad_token が 0 ではありません' }
        Write-Output ('accepted_days: ' + $accepted + '  requests: ' + $summary.requests)
        # 記録一式（シート §3-3）
        $runs = @(Get-ChildItem -LiteralPath $script:RecordDir -Directory -ErrorAction SilentlyContinue |
            Where-Object { $before -notcontains $_.Name })
        $run = @($runs | Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'events.jsonl') }) |
            Select-Object -Last 1
        if (-not $run) { throw '今回の起動で試合の run ディレクトリが作成されていません' }
        foreach ($name in @('manifest.json', 'setting.json', 'events.jsonl', 'state.json', 'coordinator.log')) {
            if (-not (Test-Path -LiteralPath (Join-Path $run.FullName $name))) {
                throw ('記録に ' + $name + ' がありません')
            }
        }
        $httpLog = Join-Under (Join-Under $script:RepoRoot @('logs')) @($run.Name, 'http.jsonl')
        if (-not (Test-Path -LiteralPath $httpLog)) { throw 'HTTP 通信ログ http.jsonl がありません' }
        Write-Output ('run: ' + $run.FullName)
        # honsen.ps1 show の再利用（show は server/token 不要）
        Invoke-Honsen -Command 'show' -ExePath $distExe -ServerUrl ''
        if ($script:LastHonsenExit -ne 0) { throw 'honsen.ps1 show が失敗しました' }
        # record show 再生: 欠損 0・異常行 0
        $replay = @(& $distExe record show $run.FullName | ForEach-Object { [string]$_ })
        $replay | ForEach-Object { Write-Output $_ }
        if ($LASTEXITCODE -ne 0) { throw 'record show が失敗しました' }
        $record = ($replay -join "`n") | ConvertFrom-Json
        if (-not $record) { throw 'record show の出力を JSON として読めません' }
        if ($record.missing_seq -ne 0 -or $record.unparsed_lines -ne 0) {
            throw ('record 再生で欠損または異常行があります: missing_seq=' + $record.missing_seq +
                ' unparsed_lines=' + $record.unparsed_lines)
        }
        # manifest binary.sha256 が実 exe と一致（三重照合の 3 つ目）
        $manifest = Get-Content -LiteralPath (Join-Path $run.FullName 'manifest.json') -Raw -ErrorAction Stop |
            ConvertFrom-Json
        if ($manifest.binary.sha256 -ne $script:ExeSha.ToLower()) {
            throw ('manifest binary.sha256 が exe SHA256 と不一致です: manifest=' + $manifest.binary.sha256)
        }
        Write-Output ('manifest SHA 照合 OK: ' + $manifest.binary.sha256)
        if (Test-Path -LiteralPath $script:LockPath) { throw '試合完了後も .honsen.lock が残っています' }
        Set-Detail '3 日受理・記録一式・HTTP ログ・show/record show 欠損 0・manifest SHA 照合・ロック解除'
    }
    finally {
        if (-not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
        Get-Process -Name 'procon' -ErrorAction SilentlyContinue |
            ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
        Remove-Item Env:PROCON_TOKEN -ErrorAction SilentlyContinue
    }
}

# ---- フェーズ 7: 公式簡易サーバー E2E（e2e.py を再利用） --------------------
# 公式簡易サーバーは macOS / Linux 配布のみで、Windows 実機には存在しない。
# 実機では SKIP、macOS 側の RC 検証では実施する。
Invoke-Phase -Name 'e2e-official' -When {
    if (-not $script:ExePath) { return '検証対象 exe が用意できませんでした（build-release が失敗した可能性）' }
    if ($ServerBinary) {
        if (Test-Path -LiteralPath $ServerBinary -PathType Leaf) {
            $script:ServerBinaryPath = (Resolve-Path -LiteralPath $ServerBinary -ErrorAction Stop).Path
            return ''
        }
        return '-ServerBinary で指定されたサーバー バイナリが見つかりません: ' + $ServerBinary
    }
    $officialDir = Join-Path (Join-Path $HOME '.procon-honsen') 'official-server'
    $found = @(Get-ChildItem -LiteralPath $officialDir -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'procon-server-*' })
    if ($found.Count -eq 0) {
        return '公式簡易サーバー バイナリ（procon-server-*）が見つかりません。Windows 実機には配布されないため、実機では SKIP になります'
    }
    $script:ServerBinaryPath = $found[0].FullName
    return ''
} -Body {
    Write-Output ('e2e.py: 公式簡易サーバー ' + $script:ServerBinaryPath + ' に対して 3 日模擬試合（プロキシ記録・record 照合つき）')
    $outDir = Join-Path $script:OutDir 'e2e-official'
    if (Test-Path -LiteralPath $outDir) { Remove-Item -LiteralPath $outDir -Recurse -Force }
    $argList = @((Join-Path $PSScriptRoot 'e2e.py'), '--server-binary', $script:ServerBinaryPath,
        '--cli', $script:ExePath, '--case-file', $script:CaseFile,
        '--days', '3', '--seconds', '8', '--record', '--output-dir', $outDir,
        '--candidate', 'fleet:budget=official:budget_ms=1000')
    $out = @(& $script:Python @argList 2>&1 | ForEach-Object { [string]$_ })
    $out | ForEach-Object { Write-Output $_ }
    Assert-ExitZero 'e2e.py が FAIL を返しました（出力の verdict / problems を確認してください）'
    Set-Detail '公式簡易サーバーで 3 日受理・記録照合まで PASS'
}

# ---- フェーズ 8: 二重起動防止と異常終了後の復帰（シート §5） -----------------
Invoke-Phase -Name 'double-start' -When $MatchGuard -Body {
    $smoke = Join-Path $script:OutDir 'double-start-smoke'
    $distExe = New-DistBundle $smoke
    Write-Output ('展開バンドル: ' + $distExe)
    if (Test-Path -LiteralPath $script:LockPath) {
        Remove-Item -LiteralPath $script:LockPath -Force -ErrorAction SilentlyContinue
    }
    $port1 = Get-MockPort
    $summary1 = Join-Path $smoke 'mock-summary-1.json'
    # 6 日にして、二重起動検査と kill の間に試合が終わらないようにする
    $server1 = Start-MockServer -Port $port1 -Days 6 -SummaryFile $summary1 `
        -StdoutFile (Join-Path $smoke 'server1.stdout.log') -StderrFile (Join-Path $smoke 'server1.stderr.log')
    $start1 = $null
    try {
        $env:PROCON_TOKEN = 'local-e2e-token'
        $start1 = Start-HonsenBackground -ExePath $distExe -ServerUrl ('http://127.0.0.1:' + $port1) `
            -StdoutFile (Join-Path $smoke 'honsen-start-1.stdout.log') `
            -StderrFile (Join-Path $smoke 'honsen-start-1.stderr.log')
        $locked = $false
        $coordinator = $null
        for ($i = 0; $i -lt 60; $i++) {
            if (Test-Path -LiteralPath $script:LockPath) { $locked = $true }
            $proc = Get-Process -Name 'procon' -ErrorAction SilentlyContinue
            if ($proc) { $coordinator = @($proc)[0]; break }
            Start-Sleep -Seconds 1
        }
        if (-not $locked -or -not $coordinator) {
            throw '1 回目の start がロック取得と coordinator 起動まで到達しません'
        }
        Write-Output ('coordinator PID: ' + $coordinator.Id + '  ロック: ' + $script:LockPath)
        # 稼働中にもう 1 つ start → 必ず拒否されること（どちらかのメッセージで exit 0 でない）
        Write-Output '== 2 回目の start（二重起動は拒否されるのが期待）'
        Invoke-Honsen -Command 'start' -ExePath $distExe -ServerUrl ('http://127.0.0.1:' + $port1)
        if ($script:LastHonsenExit -eq 0) {
            throw '2 回目の start が受理されてしまいました（二重起動ができてしまう）'
        }
        Write-Output ('2 回目の start は拒否されました（exit ' + $script:LastHonsenExit + '）')
        # 異常終了を模擬: ラッパー（ロック保持者）と coordinator を PID 指定で強制終了
        Write-Output ('== 異常終了の模擬: PID ' + $start1.Id + '（honsen ラッパー）と PID ' +
            $coordinator.Id + '（coordinator）を強制終了')
        Stop-Process -Id $start1.Id -Force -ErrorAction SilentlyContinue
        Stop-Process -Id $coordinator.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
        $left = @(Get-Process -Name 'procon' -ErrorAction SilentlyContinue)
        foreach ($proc in $left) {
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        }
        if (-not (Test-Path -LiteralPath $script:LockPath)) {
            throw 'kill 後に .honsen.lock が残っていません（異常終了の模擬になっていません）'
        }
        Write-Output 'kill 後も .honsen.lock が残っていることを確認（PID が書かれたままの残骸）'
        # 復帰: 死んだ所有者のロックを引き継いで起動に進むこと（新しい模擬サーバーに対して）
        $port2 = Get-MockPort
        $summary2 = Join-Path $smoke 'mock-summary-2.json'
        $server2 = Start-MockServer -Port $port2 -Days 2 -SummaryFile $summary2 `
            -StdoutFile (Join-Path $smoke 'server2.stdout.log') -StderrFile (Join-Path $smoke 'server2.stderr.log')
        try {
            Write-Output '== 復帰: もう一度 start（ロック引継ぎで起動に進むのが期待）'
            Invoke-Honsen -Command 'start' -ExePath $distExe -ServerUrl ('http://127.0.0.1:' + $port2)
            if ($script:LastHonsenExit -ne 0) {
                throw ('復帰の start が異常終了しました: exit ' + $script:LastHonsenExit)
            }
            if (Test-Path -LiteralPath $script:LockPath) { throw '復帰後も .honsen.lock が残っています' }
            # mock server は試合終了 2 秒後に summary を書いて自己終了するので、終了を待ってから読む
            if (-not $server2.WaitForExit(30000)) {
                Stop-Process -Id $server2.Id -Force -ErrorAction SilentlyContinue
                throw '復帰試合の mock server が終了しません'
            }
            if ($server2.ExitCode -ne 0) { throw ('復帰試合の mock server が受理を拒否しました: exit ' + $server2.ExitCode) }
            $summary = Get-Content -LiteralPath $summary2 -Raw -ErrorAction Stop | ConvertFrom-Json
            $accepted = @($summary.accepted_days) -join ','
            if ($accepted -ne '0,1') {
                throw ('復帰試合の accepted_days が 0,1 ではありません: ' + $accepted)
            }
            Write-Output ('復帰試合 accepted_days: ' + $accepted)
            Set-Detail '二重起動拒否 / kill 後のロック残骸 / ロック引継ぎで復帰 / 復帰試合 2 日受理'
        }
        finally {
            if (-not $server2.HasExited) { Stop-Process -Id $server2.Id -Force -ErrorAction SilentlyContinue }
        }
    }
    finally {
        if ($null -ne $start1 -and -not $start1.HasExited) {
            Stop-Process -Id $start1.Id -Force -ErrorAction SilentlyContinue
        }
        if (-not $server1.HasExited) { Stop-Process -Id $server1.Id -Force -ErrorAction SilentlyContinue }
        Get-Process -Name 'procon' -ErrorAction SilentlyContinue |
            ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
        Remove-Item -LiteralPath $script:LockPath -Force -ErrorAction SilentlyContinue
        Remove-Item Env:PROCON_TOKEN -ErrorAction SilentlyContinue
    }
}

# ---- フェーズ 9: PROCON_TELEMETRY モニター（#436 merge 後のみ実施可能） ------
Invoke-Phase -Name 'telemetry' -When {
    if (-not $script:ExePath) { return '検証対象 exe が用意できませんでした（build-release が失敗した可能性）' }
    $help = (@(& $script:ExePath coordinator --help 2>&1 | ForEach-Object { [string]$_ }) -join "`n")
    if ($help -match '--telemetry') { return '' }
    return 'このバイナリは PROCON_TELEMETRY / --telemetry 非対応です（PR #436 / branch 429 が未 merge）。merge 後に再実施してください'
} -Body {
    $smoke = Join-Path $script:OutDir 'telemetry-smoke'
    if (Test-Path -LiteralPath $smoke) { Remove-Item -LiteralPath $smoke -Recurse -Force }
    New-Item -ItemType Directory -Force $smoke | Out-Null
    $port = Get-MockPort
    $summaryFile = Join-Path $smoke 'mock-summary.json'
    $server = Start-MockServer -Port $port -Days 3 -SummaryFile $summaryFile `
        -StdoutFile (Join-Path $smoke 'server.stdout.log') -StderrFile (Join-Path $smoke 'server.stderr.log')
    try {
        $env:PROCON_TOKEN = 'local-e2e-token'
        # 値はそのままポート番号として parse される。1 はポート 1 への bind で失敗するので、必ず 8437 を書く
        $env:PROCON_TELEMETRY = '8437'
        $start = Start-HonsenBackground -ExePath $script:ExePath -ServerUrl ('http://127.0.0.1:' + $port) `
            -StdoutFile (Join-Path $smoke 'honsen.stdout.log') -StderrFile (Join-Path $smoke 'honsen.stderr.log')
        $telemetry = $null
        $table = $null
        $responded = $false
        for ($i = 0; $i -lt 45; $i++) {
            if ($start.HasExited) { break }
            try {
                $telemetry = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:8437/telemetry.json' -TimeoutSec 2
                $table = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:8437/table.json' -TimeoutSec 2
                if ($telemetry.StatusCode -eq 200 -and $table.StatusCode -eq 200) { $responded = $true; break }
            }
            catch { }
            Start-Sleep -Seconds 1
        }
        if (-not $responded) {
            throw 'PROCON_TELEMETRY=8437 のモニターが /telemetry.json と /table.json を 200 で返しません'
        }
        Write-Output ('telemetry.json: ' + $telemetry.Content.Substring(0, [Math]::Min(160, $telemetry.Content.Length)))
        Write-Output ('table.json: ' + $table.Content.Substring(0, [Math]::Min(160, $table.Content.Length)))
        if (-not $start.WaitForExit(120000)) { throw 'モニター付き試合が 120 秒以内に終わりません' }
        if ($start.ExitCode -ne 0) { throw ('モニター付き試合が異常終了しました: exit ' + $start.ExitCode) }
        if (-not $server.WaitForExit(30000)) {
            Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
            throw 'mock server が試合後も終了しません'
        }
        if ($server.ExitCode -ne 0) { throw ('mock server が受理を拒否しました: exit ' + $server.ExitCode) }
        $summary = Get-Content -LiteralPath $summaryFile -Raw -ErrorAction Stop | ConvertFrom-Json
        $accepted = @($summary.accepted_days) -join ','
        if ($accepted -ne '0,1,2') { throw ('accepted_days が 0,1,2 ではありません: ' + $accepted) }
        Set-Detail 'PROCON_TELEMETRY=8437 で /telemetry.json・/table.json 応答・3 日受理を確認'
    }
    finally {
        Remove-Item Env:PROCON_TELEMETRY -ErrorAction SilentlyContinue
        Remove-Item Env:PROCON_TOKEN -ErrorAction SilentlyContinue
        if (-not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
        Get-Process -Name 'procon' -ErrorAction SilentlyContinue |
            ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    }
}

# ---- サマリ -----------------------------------------------------------------
$passCount = @($script:Results | Where-Object { $_.status -eq 'PASS' }).Count
$failCount = @($script:Results | Where-Object { $_.status -eq 'FAIL' }).Count
$skipCount = @($script:Results | Where-Object { $_.status -eq 'SKIP' }).Count
$exitCode = 0
if ($failCount -gt 0) { $exitCode = 1 }
$summary = [pscustomobject]@{
    generated_at = (Get-Date -Format o)
    head_sha     = $script:HeadSha
    expected_sha = $ExpectedSha
    exe_path     = $script:ExePath
    exe_sha256   = $script:ExeSha
    ps_version   = $PSVersionTable.PSVersion.ToString()
    os           = $env:OS
    is_windows   = $script:IsWin
    pass         = $passCount
    fail         = $failCount
    skip         = $skipCount
    exit_code    = $exitCode
    phases       = $script:Results
}
Set-Content -LiteralPath (Join-Path $script:OutDir 'summary.json') `
    -Value (ConvertTo-Json -InputObject $summary -Depth 4) -Encoding UTF8
$lines = @('verify-rc summary  ' + (Get-Date -Format o))
$lines += ('  head: ' + $script:HeadSha)
$lines += ('  exe:  ' + $script:ExePath + '  sha256=' + $script:ExeSha)
foreach ($result in $script:Results) {
    $lines += ('  ' + $result.status.PadRight(5) + ' ' + $result.phase.PadRight(18) + ' ' + $result.detail)
}
$lines += ('  RESULT: PASS=' + $passCount + ' FAIL=' + $failCount + ' SKIP=' + $skipCount +
    '  exit=' + $exitCode)
Set-Content -LiteralPath (Join-Path $script:OutDir 'summary.txt') -Value $lines -Encoding UTF8
$lines | ForEach-Object { Write-Host $_ }
exit $exitCode