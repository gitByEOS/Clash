# 安装 clash 到当前 Windows 用户 PATH

[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\clash"),
    [string]$RawBaseUrl = "https://raw.githubusercontent.com/gitByEOS/Clash/master",
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"

function Write-Info($Message) {
    Write-Host $Message -ForegroundColor Cyan
}

function Write-Ok($Message) {
    Write-Host $Message -ForegroundColor Green
}

function Write-Warn($Message) {
    Write-Host $Message -ForegroundColor Yellow
}

function Install-FromLocalProject($TargetExe) {
    $candidates = @(
        (Join-Path $PSScriptRoot "target\release\clash.exe"),
        (Join-Path $PSScriptRoot "target\debug\clash.exe"),
        (Join-Path $PSScriptRoot "bin\clash-x86_64-pc-windows-msvc.exe")
    )

    foreach ($source in $candidates) {
        if (Test-Path $source) {
            Copy-Item -Force $source $TargetExe
            return
        }
    }

    throw "本地未找到 clash.exe，请先执行: cargo build --release"
}

function Install-FromRemote($TargetExe) {
    $url = "$RawBaseUrl/bin/clash-x86_64-pc-windows-msvc.exe"
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $TargetExe
}

function Write-CmdShim($TargetExe, $ShimPath) {
    $content = @(
        "@echo off"
        "`"$TargetExe`" %*"
    )
    Set-Content -Path $ShimPath -Value $content -Encoding ASCII
}

function Ensure-UserPath($Directory) {
    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    $parts = @($current -split ";" | Where-Object { $_ })

    if ($parts -contains $Directory) {
        return
    }

    if ($NoPathUpdate) {
        Write-Warn "$Directory 不在用户 PATH 中"
        Write-Warn "请手动加入用户 PATH"
        return
    }

    $next = if ($current) { "$current;$Directory" } else { $Directory }
    [Environment]::SetEnvironmentVariable("Path", $next, "User")
    Write-Ok "已写入用户 PATH，重新打开终端后生效"
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$targetExe = Join-Path $InstallDir "clash.exe"
$shimPath = Join-Path $InstallDir "clash.cmd"
$localSource = Join-Path $PSScriptRoot "Cargo.toml"

if (Test-Path $localSource) {
    Write-Info "使用本地项目安装 clash"
    Install-FromLocalProject $targetExe
}
else {
    Write-Info "从远程地址安装 clash"
    Install-FromRemote $targetExe
}

Write-CmdShim $targetExe $shimPath
Ensure-UserPath $InstallDir

Write-Ok "clash 已安装到 $InstallDir"
Write-Ok "运行 clash 开始配置"
