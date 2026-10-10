<#
.SYNOPSIS
    本選用 Windows ネイティブ起動スクリプト（WSL / Docker / Git Bash 不要）。

.DESCRIPTION
    procon.exe coordinator を、試合記録（records\）と HTTP 通信ログ（logs\）を
    有効にした状態で起動する。Windows PowerShell 5.1 と PowerShell 7 の両方で動く。

      .\honsen.ps1 preflight -Server http://<IP>:<port>   起動前検査（何も送信しない。GET /setting を 1 回読むだけ）
      .\honsen.ps1 start     -Server http://<IP>:<port>   試合を開始（落ちたら自動で再起動して途中から続ける）
      .\honsen.ps1 show      [-RunDir <dir>]              保存した記録を読み戻して要約を表示（既定は最新）

    トークンは引数に取らない。環境変数 PROCON_TOKEN があればそれを使い、なければ
    入力を求める（画面に表示されない）。子プロセスへは環境変数で渡し、終了時に消す。

    保存先（既定 %USERPROFILE%\procon-honsen、HONSEN_HOME で変更可）:
      records\<run-id>\  manifest.json / setting.json / events.jsonl / coordinator.log / state.json
      logs\<run-id>\     http.jsonl
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('preflight', 'start', 'show', 'help')]
    [string]$Command = 'help',
    [string]$Server = $env:PROCON_SERVER,
    [string]$Exe = '',
    [string]$Candidate = 'fleet:budget=official:budget_ms=10000',
    [string[]]$KindsArgs = @('--kinds-auto'),
    [string]$RunDir = '',
    [int]$MaxRunMinutes = 40
)

$ErrorActionPreference = 'Stop'

$HonsenHome = if ($env:HONSEN_HOME) { $env:HONSEN_HOME } else { Join-Path $HOME 'procon-honsen' }
$RecordDir = Join-Path $HonsenHome 'records'
$HttpLogDir = Join-Path $HonsenHome 'logs'

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
    $candidates += (Join-Path $PSScriptRoot 'procon.exe')
    $candidates += (Join-Path $HonsenHome 'bin\procon.exe')
    foreach ($path in $candidates) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            return (Resolve-Path -LiteralPath $path).Path
        }
    }
    throw "procon.exe が見つかりません。-Exe <path> で指定するか、このスクリプトと同じフォルダに置いてください。"
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
    if (-not $Server) { throw '-Server http://<IP>:<port> を指定してください。' }
}

function Invoke-Preflight([string]$ExePath) {
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

    $drive = (Get-Item -LiteralPath $HonsenHome).PSDrive
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
            -Headers @{ 'Procon-Token' = $env:PROCON_TOKEN } -TimeoutSec 5
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

function Invoke-Start([string]$ExePath) {
    if (Get-Process -Name 'procon' -ErrorAction SilentlyContinue) {
        throw '別の procon.exe が動いています。二重起動はできません。'
    }
    Disable-QuickEdit
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
            break
        }
        $restarts++
        if (((Get-Date) - $began).TotalMinutes -ge $MaxRunMinutes) {
            Write-Host "開始から $MaxRunMinutes 分を超えたので再起動をやめます" -ForegroundColor Red
            break
        }
        Start-Sleep -Seconds 1
    }
    $latest = Get-LatestRun
    if ($latest) {
        Write-Host "記録: $latest"
        & $ExePath record show $latest
    }
    Write-Host "HTTP 通信ログ: $HttpLogDir"
}

if ($Command -eq 'help') {
    Get-Help $PSCommandPath -Detailed
    exit 0
}

$exePath = Resolve-Exe
New-Item -ItemType Directory -Force -Path $HonsenHome | Out-Null

if ($Command -eq 'show') {
    $target = if ($RunDir) { $RunDir } else { Get-LatestRun }
    if (-not $target) { throw "記録がありません: $RecordDir" }
    & $exePath record show $target
    exit $LASTEXITCODE
}

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
    Invoke-Start $exePath
}
finally {
    if ($prompted) { Remove-Item Env:PROCON_TOKEN -ErrorAction SilentlyContinue }
}
