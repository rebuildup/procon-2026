<#
.SYNOPSIS
    本選用 Windows ネイティブ起動スクリプト（WSL / Docker / Git Bash 不要）。

.DESCRIPTION
    procon.exe coordinator を、試合記録（records\）と HTTP 通信ログ（logs\）を
    有効にした状態で起動する。Windows PowerShell 5.1 と PowerShell 7 の両方で動く。

      .\ops\honsen\honsen.ps1 preflight   .\.env による接続前検査
      .\ops\honsen\honsen.ps1 start       .\.env による本選起動
      .\ops\honsen\honsen.ps1 show        試合記録を表示

    操作時のカレントディレクトリは procon-2026 のリポジトリ直下。
    スクリプト自体は配置場所からリポジトリルートを解決する。

    .\.env に定義された PROCON_SERVER / PROCON_TOKEN を使う。
    .env がある場合はプロセス環境変数より優先。明示 -Server は最優先。
    .env がない場合は従来どおり環境変数・入力へフォールバック。

    保存先（すべて procon-2026 を起点とする）:
      .\records\<run-id>\  manifest.json / setting.json / events.jsonl / coordinator.log / state.json
      .\logs\<run-id>\     http.jsonl
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('preflight', 'start', 'show', 'help')]
    [string]$Command = 'help',
    [string]$Server = '',
    [string]$Exe = '',
    [string]$Candidate = 'sized:budget=official:budget_ms=59000:convoy_ms=20000:seed=42:fuel_need=0',
    [string[]]$KindsArgs = @('--kinds-auto'),
    [string]$RunDir = '',
    [int]$MaxRunMinutes = 40
)

$ErrorActionPreference = 'Stop'

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$RecordDir = Join-Path $RepoRoot 'records'
$HttpLogDir = Join-Path $RepoRoot 'logs'

# .env は PowerShell と procon.exe では自動で読み込まれない。
# 許可した2キーだけをデータとして取り出し、任意のコードとして実行しない。
# .env の値は環境変数の古い値（練習用のものを含む）より優先する。
function Import-HonsenEnv {
    $file = Join-Path $RepoRoot '.env'
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { return }
    foreach ($line in (Get-Content -LiteralPath $file -Encoding UTF8)) {
        if ($line -match '^\s*(?:export\s+)?(PROCON_SERVER|PROCON_TOKEN)\s*=\s*(.*?)\s*$') {
            $name = $Matches[1]
            $value = $Matches[2].Trim()
            if ($value.Length -ge 2 -and (
                ($value.StartsWith('"') -and $value.EndsWith('"')) -or
                ($value.StartsWith("'") -and $value.EndsWith("'")))) {
                $value = $value.Substring(1, $value.Length - 2)
            }
            if ($value) { [Environment]::SetEnvironmentVariable($name, $value, 'Process') }
        }
    }
}

function Fail([string]$Message) {
    Write-Host "FAIL  $Message" -ForegroundColor Red
    $script:Failed = $true
}

function Ok([string]$Message) {
    Write-Host "OK    $Message" -ForegroundColor Green
}

function Resolve-Exe {
    $candidates = @()
    if ($Exe) { $candidates += $Exe }
    if ($env:HONSEN_EXE) { $candidates += $env:HONSEN_EXE }
    $candidates += (Join-Path $RepoRoot 'target\release\procon.exe')
    foreach ($path in $candidates) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            return (Resolve-Path -LiteralPath $path).Path
        }
    }
    throw "procon.exe が見つかりません。procon-2026\target\release\procon.exe を用意してください。"
}

function Read-Token {
    if ($env:PROCON_TOKEN) { return $false }
    $secure = Read-Host -AsSecureString 'チームのトークン（表示されません）'
    $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try {
        $env:PROCON_TOKEN = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr)
    }
    finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr)
    }
    if (-not $env:PROCON_TOKEN) { throw 'トークンが空です。' }
    return $true
}

function Set-RunEnvironment {
    $env:PROCON_RECORD_DIR = $RecordDir
    $env:PROCON_HTTP_LOG_DIR = $HttpLogDir
    $env:PROCON_RESUME_RUNNING_MATCH = '1'
    $env:RUST_BACKTRACE = '1'
    $env:NO_COLOR = '1'
    if (-not $env:RUST_LOG) { $env:RUST_LOG = 'info' }
}

# 旧来のコンソール（conhost）は、ウィンドウ内をクリックして範囲選択が始まると
# 標準出力への書込みが止まり、procon.exe ごと停止する。簡易編集モードを切って防ぐ。
# 失敗しても起動は続ける（Windows Terminal ではそもそも止まらない）。
function Disable-QuickEdit {
    if ($env:OS -ne 'Windows_NT') { return }
    try {
        if (-not ('Honsen.ConsoleMode' -as [type])) {
            Add-Type -Namespace Honsen -Name ConsoleMode -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr GetStdHandle(int nStdHandle);
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool GetConsoleMode(IntPtr handle, out uint mode);
[DllImport("kernel32.dll", SetLastError = true)] public static extern bool SetConsoleMode(IntPtr handle, uint mode);
'@
        }
        $handle = [Honsen.ConsoleMode]::GetStdHandle(-10)
        $mode = 0
        if ([Honsen.ConsoleMode]::GetConsoleMode($handle, [ref]$mode)) {
            # ENABLE_QUICK_EDIT_MODE (0x40) を外し、ENABLE_EXTENDED_FLAGS (0x80) を立てる
            $mode = ($mode -band (-bnot 0x40)) -bor 0x80
            [void][Honsen.ConsoleMode]::SetConsoleMode($handle, $mode)
        }
    }
    catch {
        Write-Host "WARN  簡易編集モードを無効にできませんでした。試合中はコンソールをクリックしないでください: $($_.Exception.Message)" -ForegroundColor Yellow
    }
}

function Require-Server {
    if (-not $Server) { throw 'PROCON_SERVER が未設定です。.env または環境変数、-Server で指定してください。' }
}

function Invoke-Preflight([string]$ExePath) {
    # Windows PowerShell 5.1 は、stderr に何か書いたネイティブコマンドを
    # 'Stop' の下ではエラー扱いにすることがある。procon.exe は通信ログを
    # stderr に出すので、ネイティブコマンドを呼ぶ関数の中では 'Continue' にする。
    $ErrorActionPreference = 'Continue'
    $script:Failed = $false
    Write-Host "== preflight: $Server"

    $version = & $ExePath version
    if ($LASTEXITCODE -eq 0) { Ok "バイナリ: $ExePath  $version" } else { Fail "procon.exe version が失敗しました" }
    $hash = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLower()
    Ok "sha256: $hash"

    # --dry-run は通信しない。記録先を実際に書いて確かめる。
    $lines = & $ExePath coordinator --dry-run --server $Server
    $json = $lines | Where-Object { $_ -match '^\s*\{' } | Select-Object -Last 1
    if (-not $json) {
        Fail "coordinator --dry-run の出力を読めません"
    }
    else {
        $dry = $json | ConvertFrom-Json
        if ($dry.has_token) { Ok 'トークン: 設定済み' } else { Fail 'トークンが空です' }
        if (-not $dry.record -or -not $dry.record.enabled) {
            Fail '試合記録が無効です（この procon.exe は --record-dir に対応していない可能性があります）'
        }
        elseif ($dry.record.writable) { Ok "試合記録: $($dry.record.dir)" }
        else { Fail "試合記録の保存先に書けません: $($dry.record.dir)  $($dry.record.error)" }
        if ($dry.PSObject.Properties.Name -contains 'http_log') {
            if (-not $dry.http_log.enabled) { Fail 'HTTP 通信ログが無効です' }
            elseif ($dry.http_log.writable) { Ok "HTTP 通信ログ: $($dry.http_log.dir)" }
            else { Fail "HTTP 通信ログの保存先に書けません: $($dry.http_log.dir)  $($dry.http_log.error)" }
        }
        else {
            Write-Host 'WARN  この procon.exe は HTTP 通信ログ（--http-log-dir）に対応していません' -ForegroundColor Yellow
        }
    }

    $drive = (Get-Item -LiteralPath $RepoRoot).PSDrive
    if ($drive -and $null -ne $drive.Free) {
        $freeGb = [math]::Round($drive.Free / 1GB, 1)
        if ($drive.Free -lt 1GB) { Fail "空き容量が ${freeGb} GB しかありません" } else { Ok "空き容量: ${freeGb} GB" }
    }

    if (Get-Process -Name 'procon' -ErrorAction SilentlyContinue) {
        Fail '別の procon.exe が動いています（同じトークンだと回答を上書きし合います）'
    }

    # サーバーへは GET /setting を 1 回だけ。何も送信しない。
    $status = 0
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri ($Server.TrimEnd('/') + '/setting') `
            -Headers @{ 'Procon-Token' = $env:PROCON_TOKEN } -TimeoutSec 5 -ErrorAction Stop
        $status = [int]$response.StatusCode
    }
    catch {
        $failure = $_.Exception
        if ($failure.Response) { $status = [int]$failure.Response.StatusCode }
        else { Fail "サーバーに接続できません: $($failure.Message)" }
    }
    switch ($status) {
        200 { Ok 'サーバー応答: 200（認証 OK、試合設定が出ている）' }
        403 { Ok 'サーバー応答: 403 AccessTimeError（認証 OK、試合はまだ設定されていない。そのまま待つ）' }
        401 { Fail 'サーバー応答: 401 AuthError（トークン違い）' }
        0 { }
        default { Fail "サーバー応答: $status" }
    }

    if ($script:Failed) {
        Write-Host 'PREFLIGHT FAIL' -ForegroundColor Red
        return $false
    }
    Write-Host 'PREFLIGHT OK' -ForegroundColor Green
    return $true
}

function Get-LatestRun {
    if (-not (Test-Path -LiteralPath $RecordDir)) { return $null }
    $latest = Get-ChildItem -LiteralPath $RecordDir -Directory | Sort-Object Name | Select-Object -Last 1
    if ($latest) { return $latest.FullName }
    return $null
}

# 二重起動の防止。ロックファイルを「なければ作る」(CreateNew) で開く。これは 1 操作
# なので、同時に 2 つの start を打っても片方しか通らない。中身は持ち主の PID。
# 持ち主がもういなければ（前回の異常終了）引き継ぐ。PID がまだ書かれていない
# ロックは作成途中かもしれないので、すぐには捨てず、少し待って読み直す。
$LockFile = Join-Path $RepoRoot '.honsen.lock'

function Enter-Lock {
    for ($try = 0; $try -lt 2; $try++) {
        try {
            $stream = [System.IO.File]::Open($LockFile, [System.IO.FileMode]::CreateNew,
                [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read)
            try {
                $bytes = [System.Text.Encoding]::ASCII.GetBytes("$PID")
                $stream.Write($bytes, 0, $bytes.Length)
            }
            finally { $stream.Dispose() }
            return
        }
        catch [System.IO.IOException] {
            $owner = 0
            for ($read = 0; $read -lt 10 -and $owner -le 0; $read++) {
                try { $owner = [int]([System.IO.File]::ReadAllText($LockFile).Trim()) } catch { $owner = 0 }
                if ($owner -le 0) { Start-Sleep -Milliseconds 200 }
            }
            if ($owner -gt 0 -and (Get-Process -Id $owner -ErrorAction SilentlyContinue)) {
                throw "別の honsen.ps1 start が動いています（PID $owner）。二重起動はできません。"
            }
            Remove-Item -LiteralPath $LockFile -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    throw "ロックを取得できません: $LockFile"
}

function Exit-Lock {
    Remove-Item -LiteralPath $LockFile -Force -ErrorAction SilentlyContinue
}

function Invoke-Start([string]$ExePath) {
    $ErrorActionPreference = 'Continue'
    if (Get-Process -Name 'procon' -ErrorAction SilentlyContinue) {
        throw '別の procon.exe が動いています。二重起動はできません。'
    }
    $outcome = 'failed'
    Disable-QuickEdit
    $before = @()
    if (Test-Path -LiteralPath $RecordDir) {
        $before = @(Get-ChildItem -LiteralPath $RecordDir -Directory | ForEach-Object { $_.Name })
    }
    $began = Get-Date
    $restarts = 0
    while ($true) {
        Write-Host "$(Get-Date -Format HH:mm:ss) honsen: coordinator を起動します（再起動 $restarts 回目）"
        $arguments = @('coordinator', '--server', $Server, '--exit-after-match', '--candidate', $Candidate) + $KindsArgs
        & $ExePath @arguments
        $code = $LASTEXITCODE
        Write-Host "$(Get-Date -Format HH:mm:ss) honsen: coordinator が終了しました（終了コード $code）"
        if ($code -eq 0) {
            Write-Host '試合完了（正常終了）' -ForegroundColor Green
            $outcome = 'completed'
            break
        }
        # 回答が拒否された日があると、coordinator は試合を最後まで進めたうえで 0 以外で
        # 終了する。これはクラッシュではないので再起動しない（終わった試合へ入り直さない）。
        $latest = Get-LatestRun
        if ($latest) {
            $statePath = Join-Path $latest 'state.json'
            if (Test-Path -LiteralPath $statePath) {
                $status = ''
                try { $status = (Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json).status } catch { }
                if ($status -eq 'rejected') {
                    Write-Host '試合は最後まで進みましたが、拒否された日があります（再起動しません）' -ForegroundColor Red
                    $outcome = 'rejected'
                    break
                }
            }
        }
        $restarts++
        if (((Get-Date) - $began).TotalMinutes -ge $MaxRunMinutes) {
            Write-Host "開始から $MaxRunMinutes 分を超えたので再起動をやめます" -ForegroundColor Red
            break
        }
        Start-Sleep -Seconds 1
    }
    # 再起動があると run ディレクトリが複数できる。今回の起動で増えたもののうち、
    # 試合のイベントを持つものを古い順にすべて表示する（最後の空の再起動だけを見せない）。
    $runs = @()
    if (Test-Path -LiteralPath $RecordDir) {
        $runs = @(Get-ChildItem -LiteralPath $RecordDir -Directory | Sort-Object Name |
                Where-Object { ($before -notcontains $_.Name) -and (Test-Path (Join-Path $_.FullName 'events.jsonl')) })
    }
    foreach ($run in $runs) {
        Write-Host "記録: $($run.FullName)"
        & $ExePath record show $run.FullName
    }
    if ($runs.Count -eq 0) { Write-Host "今回の起動では試合の記録ができていません: $RecordDir" -ForegroundColor Yellow }
    Write-Host "HTTP 通信ログ: $HttpLogDir"
    return $outcome
}

if ($Command -eq 'help') {
    Get-Help $PSCommandPath -Detailed
    exit 0
}

$exePath = Resolve-Exe
New-Item -ItemType Directory -Force -Path $RecordDir, $HttpLogDir | Out-Null

if ($Command -eq 'show') {
    $target = if ($RunDir) { $RunDir } else { Get-LatestRun }
    if (-not $target) { throw "記録がありません: $RecordDir" }
    & $exePath record show $target
    exit $LASTEXITCODE
}

Import-HonsenEnv
if (-not $Server) { $Server = $env:PROCON_SERVER }
Require-Server
$prompted = Read-Token
try {
    Set-RunEnvironment
    if ($Command -eq 'preflight') {
        if (Invoke-Preflight $exePath) { exit 0 } else { exit 1 }
    }
    if (-not (Invoke-Preflight $exePath)) {
        # 試合途中の緊急再起動を検査で止めないよう、最終判断は操作者に委ねる。
        $answer = Read-Host 'preflight に FAIL があります。それでも起動しますか？ 起動するなら YES と入力'
        if ($answer -ne 'YES') { throw '起動を中止しました。上の FAIL 行を直してください。' }
    }
    Enter-Lock
    $outcome = 'failed'
    try {
        # 関数の出力（record show の JSON など）はそのまま流し、最後の値だけを結果として受け取る。
        Invoke-Start $exePath | ForEach-Object { $outcome = $_; $_ } | Select-Object -SkipLast 1
    }
    finally {
        Exit-Lock
    }
    # 正常終了だけを 0 にする。拒否された日がある・再起動をあきらめた、は 1。
    if ($outcome -ne 'completed') { exit 1 }
}
finally {
    if ($prompted) { Remove-Item Env:PROCON_TOKEN -ErrorAction SilentlyContinue }
}
