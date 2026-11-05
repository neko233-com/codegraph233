# game-project-graph installer (Windows / PowerShell 7+)
#
# Global install of the `game-project-graph` MCP server, then optional MCP
# registration for Codex / Cursor / VS Code / JetBrains Air / Cowork.
#
# Usage:
#   ./install.ps1                      # install from GitHub (main branch)
#   ./install.ps1 -Source "D:\path\game-project-graph"   # install a local checkout
#   ./install.ps1 -NoMcp               # install binary only
#   ./install.ps1 -Ref v0.1.0          # pin a tag/branch/commit
param(
    [string]$Source = "",
    [string]$Ref = "main",
    [switch]$NoMcp,
    [string]$Clients = "codex,cursor,vscode,air,cowork"
)

$ErrorActionPreference = "Stop"
$Module = "github.com/neko233-com/game-project-graph"

function Fail($msg) {
    Write-Host "ERROR: $msg" -ForegroundColor Red
    exit 1
}

# --- 1. require Go ---------------------------------------------------------
$go = Get-Command go -ErrorAction SilentlyContinue
if (-not $go) {
    Fail "未找到 Go。请先安装 Go 1.22+（推荐 1.27）：https://go.dev/dl/"
}
$goVersion = (& go version) -replace '^go version go([0-9]+\.[0-9]+).*$', '$1'
Write-Host "Go $goVersion detected" -ForegroundColor DarkGray

# --- 2. install ------------------------------------------------------------
if ($Source -ne "") {
    if (-not (Test-Path (Join-Path $Source "go.mod"))) {
        Fail "本地源码目录缺少 go.mod：$Source"
    }
    Write-Host "Installing from local source: $Source"
    & go install -C $Source "$Module/cmd/game-project-graph"
} else {
    Write-Host "Installing $Module@$Ref (this downloads modules on first run)…"
    & go install "$Module/cmd/game-project-graph@$Ref"
}
if ($LASTEXITCODE -ne 0) { Fail "go install 失败（退出码 $LASTEXITCODE）" }

# --- 3. make sure the binary is on PATH -----------------------------------
$gobin = (& go env GOBIN)
if ([string]::IsNullOrWhiteSpace($gobin)) {
    $gobin = Join-Path (& go env GOPATH) "bin"
}
$gobin = $gobin.Trim()
$exe = Join-Path $gobin "game-project-graph.exe"
if (-not (Test-Path $exe)) { Fail "安装后未找到 $exe" }

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$parts = @()
if ($userPath) { $parts = $userPath -split ';' | Where-Object { $_ -ne '' } }
if (-not ($parts | Where-Object { $_.TrimEnd('\') -ieq $gobin.TrimEnd('\') })) {
    [Environment]::SetEnvironmentVariable("Path", (($parts + $gobin) -join ';'), "User")
    Write-Host "已把 $gobin 追加到用户 PATH（新终端生效）" -ForegroundColor Yellow
}
$env:Path = "$gobin;$env:Path"

& $exe version
if ($LASTEXITCODE -ne 0) { Fail "二进制自检失败" }

# --- 4. optional MCP registration in the current project -------------------
if (-not $NoMcp) {
    if (Test-Path ".game-project-graph.json") {
        Write-Host "Registering MCP server for: $Clients"
        & $exe install-mcp --clients $Clients --exe "game-project-graph"
    } else {
        Write-Host "当前目录没有 .game-project-graph.json，跳过 MCP 注册。" -ForegroundColor Yellow
        Write-Host "在项目根执行： game-project-graph init && game-project-graph install-mcp"
    }
}

Write-Host ""
Write-Host "Done. Next steps:" -ForegroundColor Green
Write-Host "  game-project-graph init          # 生成 .game-project-graph.json（自动识别引擎/前后端）"
Write-Host "  game-project-graph index         # 建立/增量更新索引"
Write-Host "  game-project-graph status        # 查看识别结果与统计"
