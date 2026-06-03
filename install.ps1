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

function Install-FromLocalProject($TargetScript) {
    $source = Join-Path $PSScriptRoot "bin\clash.ps1"
    Copy-Item -Force $source $TargetScript
}

function Install-FromRemote($TargetScript) {
    $url = "$RawBaseUrl/bin/clash.ps1"
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $TargetScript
}

function Write-CmdShim($TargetScript, $ShimPath) {
    $content = @(
        "@echo off"
        "pwsh -NoProfile -ExecutionPolicy Bypass -File `"$TargetScript`" %*"
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

$targetScript = Join-Path $InstallDir "clash.ps1"
$shimPath = Join-Path $InstallDir "clash.cmd"
$localSource = Join-Path $PSScriptRoot "bin\clash.ps1"

if (Test-Path $localSource) {
    Write-Info "使用本地项目安装 clash"
    Install-FromLocalProject $targetScript
}
else {
    Write-Info "从远程地址安装 clash"
    Install-FromRemote $targetScript
}

Write-CmdShim $targetScript $shimPath
Ensure-UserPath $InstallDir

Write-Ok "clash 已安装到 $InstallDir"
Write-Ok "运行 clash 开始配置"
