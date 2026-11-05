# game-project-graph installer (Windows / PowerShell 7+)
#
# Builds the Rust binary globally with `cargo install` and registers the MCP
# server.  One global install serves every project; each project only keeps its
# own .codegraph233.json and index.
#
# Usage:
#   ./install.ps1                        # install from the current checkout
#   ./install.ps1 -Scope project         # opt-in: pin one project in project configs
#   ./install.ps1 -NoMcp                 # binary only
param(
    [string]$Scope = "global",
    [switch]$NoMcp,
    [string]$Clients = "codex,cursor,vscode,air,cowork,workbuddy,project"
)

$ErrorActionPreference = "Stop"

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    Write-Host "ERROR: 未找到 cargo。请先安装 Rust（https://rustup.rs/）" -ForegroundColor Red
    exit 1
}
$rustc = (& rustc --version) 2>&1
Write-Host "Rust detected: $rustc" -ForegroundColor DarkGray

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Write-Host "Installing codegraph233 from $root ..."
& cargo install --path $root --force
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: cargo install 失败（退出码 $LASTEXITCODE）" -ForegroundColor Red
    exit 1
}

$exe = Get-Command codegraph233 -ErrorAction SilentlyContinue
if (-not $exe) {
    $gobin = Join-Path $env:USERPROFILE ".cargo\bin"
    if (-not (($env:Path -split ';') -contains $gobin)) {
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        [Environment]::SetEnvironmentVariable("Path", ("$userPath;$gobin").TrimStart(';'), "User")
        Write-Host "已把 $gobin 追加到用户 PATH（新终端生效）" -ForegroundColor Yellow
    }
    $env:Path = "$gobin;$env:Path"
    $exe = Join-Path $gobin "codegraph233.exe"
}

& $exe version

if (-not $NoMcp) {
    if (Test-Path ".codegraph233.json") {
        Write-Host "Registering MCP server for: $Clients (scope=$Scope)"
        & $exe install-mcp --clients $Clients --exe "codegraph233" --scope $Scope
    } else {
        Write-Host "当前目录没有 .codegraph233.json，跳过 MCP 注册。" -ForegroundColor Yellow
        Write-Host "在项目根执行： codegraph233 init && codegraph233 index && codegraph233 install-mcp"
    }
}

Write-Host ""
Write-Host "Done. Next steps:" -ForegroundColor Green
Write-Host "  codegraph233 init          # 生成 .codegraph233.json"
Write-Host "  codegraph233 index         # 建立/增量更新索引"
Write-Host "  codegraph233 status        # 查看统计"
Write-Host "  codegraph233 serve         # MCP stdio（客户端会自动启动）"
