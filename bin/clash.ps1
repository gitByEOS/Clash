#!/usr/bin/env pwsh
# clash - Claude Code 自定义渠道启动器

[CmdletBinding()]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CliArgs
)

$ErrorActionPreference = "Stop"

$AppName = "clash"

function Get-ConfigDir {
    if ($env:APPDATA) {
        return Join-Path $env:APPDATA $AppName
    }

    $userHome = $env:HOME
    if (-not $userHome) {
        $userHome = $env:USERPROFILE
    }
    if (-not $userHome) {
        throw "无法确定配置目录：缺少 APPDATA、HOME 或 USERPROFILE"
    }

    return Join-Path (Join-Path $userHome ".config") $AppName
}

$ConfigDir = Get-ConfigDir
$ConfigFile = Join-Path $ConfigDir "auth"

function Write-Info($Message) {
    Write-Host $Message -ForegroundColor Cyan
}

function Write-Ok($Message) {
    Write-Host $Message -ForegroundColor Green
}

function Write-Warn($Message) {
    Write-Host $Message -ForegroundColor Yellow
}

function Write-Err($Message) {
    Write-Host $Message -ForegroundColor Red
}

function Protect-Token([string]$Token) {
    ConvertTo-SecureString $Token -AsPlainText -Force | ConvertFrom-SecureString
}

function Unprotect-Token([string]$Value) {
    $secure = ConvertTo-SecureString $Value
    $ptr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
    try {
        [Runtime.InteropServices.Marshal]::PtrToStringBSTR($ptr)
    }
    finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($ptr)
    }
}

function Get-ConfigValue([string]$Key) {
    if (-not (Test-Path $ConfigFile)) {
        return $null
    }

    $line = Get-Content $ConfigFile | Where-Object { $_ -like "$Key=*" } | Select-Object -First 1
    if (-not $line) {
        return $null
    }

    return $line.Substring($Key.Length + 1)
}

function Get-Models {
    if (-not (Test-Path $ConfigFile)) {
        return @()
    }

    $models = New-Object System.Collections.Generic.List[string]
    $inModels = $false
    foreach ($line in Get-Content $ConfigFile) {
        if ($line -eq "MODELS=<<MODELS") {
            $inModels = $true
            continue
        }

        if ($inModels -and $line -eq "MODELS") {
            break
        }

        if ($inModels -and -not [string]::IsNullOrWhiteSpace($line)) {
            $models.Add($line)
        }
    }

    return $models.ToArray()
}

function Normalize-Models([string]$Models) {
    return @(
        $Models -split "," |
            ForEach-Object { $_.Trim() } |
            Where-Object { $_ }
    )
}

function Get-AuthToken {
    $raw = Get-ConfigValue "AUTH_TOKEN"
    if (-not $raw) {
        return $null
    }

    try {
        return Unprotect-Token $raw
    }
    catch {
        return $raw
    }
}

function Save-Config([string]$BaseUrl, [string]$Token, [string[]]$Models, [string]$CommandName = "clash") {
    New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null

    $encrypted = if ($Token) { Protect-Token $Token } else { "" }
    $content = @(
        "# Clash 配置文件"
        "# 生成时间: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
        "BASE_URL=$BaseUrl"
        "AUTH_TOKEN=$encrypted"
        "COMMAND=$CommandName"
        "MODELS=<<MODELS"
    ) + $Models + @("MODELS")

    Set-Content -Path $ConfigFile -Value $content -Encoding UTF8
    Invoke-ConfigProbe
}

function Invoke-ConfigProbe {
    if ($env:CLASH_SKIP_AUTO_TEST -match '^(?i)(1|true|yes)$') {
        return
    }

    $baseUrl = (Get-ConfigValue "BASE_URL").Trim().TrimEnd('/')
    $token = Get-AuthToken
    $models = @(Get-Models)

    if (-not $baseUrl -or -not $token -or $models.Count -eq 0) {
        Write-Warn "配置不完整，跳过连通性测试"
        return
    }

    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if (-not $curl) {
        $curl = Get-Command curl -ErrorAction SilentlyContinue
    }
    if (-not $curl) {
        Write-Warn "未找到 curl，跳过连通性测试"
        return
    }

    $endpoint = if ($baseUrl -match '/v1$') { "$baseUrl/messages" } else { "$baseUrl/v1/messages" }
    $isDashscope = $baseUrl -match 'dashscope'

    Write-Info "正在进行 Anthropic 兼容 API 连通测试..."
    $failed = 0
    foreach ($model in $models) {
        Write-Info "  连通测试 $model ..."
        [Console]::Out.Flush()
        $escaped = $model.Replace('\', '\\').Replace('"', '\"')
        if ($isDashscope) {
            $body = "{`"model`":`"$escaped`",`"max_tokens`":1,`"thinking`":{`"type`":`"disabled`"},`"messages`":[{`"role`":`"user`",`"content`":`"ping`"}]}"
        }
        else {
            $body = "{`"model`":`"$escaped`",`"max_tokens`":1,`"messages`":[{`"role`":`"user`",`"content`":`"ping`"}]}"
        }

        $raw = & $curl.Source -sS -w "`n%{http_code}" -X POST --max-time 30 `
            -H "content-type: application/json" `
            -H "x-api-key: $token" `
            -H "anthropic-version: 2023-06-01" `
            -H "user-agent: claude-cli/2.1.118 (external, cli)" `
            -H "x-app: cli" `
            -H "anthropic-beta: interleaved-thinking-2025-05-14" `
            -d $body $endpoint 2>&1 | Out-String

        $lines = $raw.TrimEnd() -split "`n"
        $codeLine = $lines[-1]
        if ($codeLine -match '^2\d\d$') {
            Write-Ok "  $model 通过"
        }
        else {
            $failed++
            $detail = if ($lines.Count -gt 1) { ($lines[0..($lines.Count - 2)] -join "`n").Substring(0, [Math]::Min(300, ($lines[0..($lines.Count - 2)] -join "`n").Length)) } else { $codeLine }
            Write-Err "  $model 失败: $detail"
        }
        [Console]::Out.Flush()
    }

    if ($failed -gt 0) {
        throw "$failed/$($models.Count) 个模型连通测试失败"
    }

    Write-Ok "全部通过（$($models.Count) 个模型）"
}

function Parse-ConfigArgs([string[]]$InputArgs) {
    $result = @{
        Url = ""
        Key = ""
        Models = ""
    }

    for ($i = 0; $i -lt $InputArgs.Count; $i++) {
        switch ($InputArgs[$i]) {
            "--url" {
                $i++
                if ($i -ge $InputArgs.Count) { throw "--url 缺少值" }
                $result.Url = $InputArgs[$i]
            }
            "--key" {
                $i++
                if ($i -ge $InputArgs.Count) { throw "--key 缺少值" }
                $result.Key = $InputArgs[$i]
            }
            "--models" {
                $i++
                if ($i -ge $InputArgs.Count) { throw "--models 缺少值" }
                $result.Models = $InputArgs[$i]
            }
            default {
                throw "未知参数: $($InputArgs[$i])"
            }
        }
    }

    return $result
}

function Invoke-ConfigInteractive {
    Write-Info "Clash 配置向导"

    $baseUrl = Read-Host "请输入 Anthropic 兼容 API 地址"
    if (-not $baseUrl) {
        throw "地址不能为空"
    }

    $token = Read-Host "请输入 API Key"
    if (-not $token) {
        throw "Key 不能为空"
    }

    $models = @()
    while ($models.Count -eq 0) {
        $modelInput = Read-Host "请输入模型列表，多个模型用逗号分隔"
        $models = Normalize-Models $modelInput
        if ($models.Count -eq 0) {
            Write-Err "模型列表不能为空"
        }
    }

    Save-Config $baseUrl $token $models "clash"
    Write-Ok "配置已保存到 $ConfigFile"
}

function Invoke-ConfigSet([string[]]$InputArgs) {
    $parsed = Parse-ConfigArgs $InputArgs

    if (-not $parsed.Url -and -not $parsed.Key -and -not $parsed.Models) {
        throw "请至少提供一个 --url、--key 或 --models"
    }

    $baseUrl = Get-ConfigValue "BASE_URL"
    $token = Get-AuthToken
    $models = @(Get-Models)
    $name = Get-ConfigValue "COMMAND"
    if (-not $name) {
        $name = "clash"
    }

    if ($parsed.Url) {
        $baseUrl = $parsed.Url
    }
    if ($parsed.Key) {
        $token = $parsed.Key
    }
    if (-not $token) {
        $token = ""
    }
    if ($parsed.Models) {
        $models = Normalize-Models $parsed.Models
        if ($models.Count -eq 0) {
            throw "模型列表不能为空"
        }
    }

    Save-Config $baseUrl $token $models $name
    Write-Ok "配置已保存到 $ConfigFile"
}

function Invoke-Config([string[]]$InputArgs) {
    if ($InputArgs.Count -eq 0) {
        Show-Config
        return
    }

    Invoke-ConfigSet $InputArgs
}

function Remove-Config {
    if (Test-Path $ConfigFile) {
        Remove-Item -Force $ConfigFile
    }

    Write-Ok "已删除配置 $ConfigFile"
}

function Show-Config {
    if (-not (Test-Path $ConfigFile)) {
        Write-Warn "未配置，请运行 clash 进行初始化"
        return
    }

    Write-Info "=== 当前配置 ==="
    foreach ($line in Get-Content $ConfigFile) {
        if ($line -like "AUTH_TOKEN=*") {
            $raw = $line.Substring("AUTH_TOKEN=".Length)
            if (-not $raw) {
                Write-Host "AUTH_TOKEN="
                continue
            }
            $token = Get-AuthToken
            if ($token -and $token.Length -ge 10) {
                Write-Host ("AUTH_TOKEN={0}****{1} (DPAPI 加密存储)" -f $token.Substring(0, 5), $token.Substring($token.Length - 5))
            }
            else {
                Write-Host "AUTH_TOKEN=**** (DPAPI 加密存储)"
            }
        }
        else {
            Write-Host $line
        }
    }
}

function Add-Model([string]$Model) {
    if (-not (Test-Path $ConfigFile)) {
        throw "未找到配置，请先运行 clash"
    }
    if (-not $Model) {
        throw "用法: clash add-model <模型名>"
    }

    $models = @(Get-Models)
    if ($models -contains $Model) {
        Write-Warn "模型 $Model 已存在"
        return
    }

    $baseUrl = Get-ConfigValue "BASE_URL"
    $token = Get-AuthToken
    $name = Get-ConfigValue "COMMAND"
    if (-not $name) {
        $name = "clash"
    }

    Save-Config $baseUrl $token ($models + $Model) $name
    Write-Ok "已添加模型: $Model"
}

function Change-Token([string]$Token) {
    if (-not (Test-Path $ConfigFile)) {
        throw "未找到配置，请先运行 clash"
    }
    if (-not $Token) {
        throw "用法: clash change-token <新Key>"
    }

    $baseUrl = Get-ConfigValue "BASE_URL"
    $models = @(Get-Models)
    $name = Get-ConfigValue "COMMAND"
    if (-not $name) {
        $name = "clash"
    }

    Save-Config $baseUrl $Token $models $name
    Write-Ok "API Key 已更新"
}

function Enable-VirtualTerminal {
    if (-not ($IsWindows -or $env:OS -eq "Windows_NT")) {
        return
    }

    try {
        if (-not ("Console.WinVT" -as [type])) {
            $sig = @'
[DllImport("kernel32.dll")] public static extern IntPtr GetStdHandle(int h);
[DllImport("kernel32.dll")] public static extern bool GetConsoleMode(IntPtr h, out uint m);
[DllImport("kernel32.dll")] public static extern bool SetConsoleMode(IntPtr h, uint m);
'@
            Add-Type -MemberDefinition $sig -Name WinVT -Namespace Console -PassThru | Out-Null
        }

        foreach ($handleId in -11, -12) {
            $handle = [Console.WinVT]::GetStdHandle($handleId)
            [uint32]$mode = 0
            if ([Console.WinVT]::GetConsoleMode($handle, [ref]$mode)) {
                [Console.WinVT]::SetConsoleMode($handle, $mode -bor 4) | Out-Null
            }
        }
    }
    catch {
    }
}

function Select-Model([string[]]$Models) {
    if (-not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected) {
        $esc = [char]0x1b
        $query = ""
        $selected = 0
        $offset = 0
        $markerCols = 2
        $rowPrefix = "model  "
        $helpText = "选择模型 | ↑/↓ 选择, Enter 确认, Esc 退出"

        function Write-Tui([string]$Text) {
            [Console]::Out.Write($Text)
        }

        function Save-Cursor {
            Write-Tui "${esc}7"
        }

        function Restore-Cursor {
            Write-Tui "${esc}8"
        }

        function Hide-Cursor {
            Write-Tui "${esc}[?25l"
        }

        function Show-Cursor {
            Write-Tui "${esc}[?25h"
        }

        function Get-FrameWidth {
            return [Math]::Max(20, [Console]::WindowWidth - 1)
        }

        function Clear-Frame {
            Restore-Cursor
            Write-Tui "${esc}[G${esc}[J"
        }

        function Write-TuiLine([string]$Text, [string]$Style = "${esc}[37m") {
            $width = Get-FrameWidth
            $line = if ($Text.Length -gt $width) { $Text.Substring(0, $width) } else { $Text.PadRight($width) }
            Write-Tui "`r${esc}[K${Style}${line}${esc}[0m`n"
        }

        function Set-InputCursor([string]$Text) {
            Restore-Cursor
            $column = 9 + $Text.Length
            Write-Tui "${esc}[${column}G"
        }

        function Get-FuzzyScore([string]$Needle, [string]$Haystack) {
            if (-not $Needle) {
                return 0
            }

            $needleLower = $Needle.ToLowerInvariant()
            $haystackLower = $Haystack.ToLowerInvariant()
            $index = $haystackLower.IndexOf($needleLower, [StringComparison]::Ordinal)
            if ($index -ge 0) {
                return 100000 - ($index * 100) - $Haystack.Length
            }

            $position = 0
            $gaps = 0
            $last = -1
            foreach ($char in $needleLower.ToCharArray()) {
                $found = $haystackLower.IndexOf($char, $position)
                if ($found -lt 0) {
                    return $null
                }

                $gaps += $found - $position
                $last = $found
                $position = $found + 1
            }

            return 50000 - ($gaps * 100) - $last - $Haystack.Length
        }

        function Get-FilteredModels([string[]]$SourceModels, [string]$Needle) {
            $ranked = foreach ($model in $SourceModels) {
                $score = Get-FuzzyScore $Needle $model
                if ($null -ne $score) {
                    [pscustomobject]@{
                        Score = $score
                        Model = $model
                    }
                }
            }

            return @($ranked | Sort-Object -Property Score, Model -Descending | ForEach-Object { $_.Model })
        }

        function Get-MatchMask([string]$Haystack, [string]$Needle) {
            $mask = New-Object bool[] $Haystack.Length
            if (-not $Needle) {
                return $mask
            }

            $haystackLower = $Haystack.ToLowerInvariant()
            $needleLower = $Needle.ToLowerInvariant()
            $position = 0
            foreach ($char in $needleLower.ToCharArray()) {
                $found = $haystackLower.IndexOf($char, $position)
                if ($found -lt 0) {
                    break
                }
                $mask[$found] = $true
                $position = $found + 1
            }

            return $mask
        }

        function Write-FrameLine([string]$Text, [ConsoleColor]$Foreground = [ConsoleColor]::Gray, [ConsoleColor]$Background = [ConsoleColor]::Black) {
            $style = switch ($Foreground) {
                ([ConsoleColor]::Cyan) { "${esc}[36m" }
                ([ConsoleColor]::Yellow) { "${esc}[33m" }
                default { "${esc}[90m" }
            }

            if ($Background -eq [ConsoleColor]::DarkGray) {
                $width = Get-FrameWidth
                $countPart = ($Text -split " ", 2)[0]
                $rulePart = if ($Text.Length -gt $countPart.Length) { $Text.Substring($countPart.Length + 1) } else { "" }
                Write-Tui "`r${esc}[K${esc}[37m$countPart ${esc}[90m$rulePart${esc}[0m`n"
                return
            }

            Write-TuiLine $Text $style
        }

        function Write-SelectorRow([string]$ModelName, [string]$Needle, [bool]$IsSelected) {
            $width = Get-FrameWidth
            $modelWidth = [Math]::Max(0, $width - $markerCols - $rowPrefix.Length)
            $mask = Get-MatchMask $ModelName $Needle
            $rowStyle = if ($IsSelected) { "${esc}[48;5;53m${esc}[37m" } else { "${esc}[37m" }

            Write-Tui "`r${esc}[K$rowStyle"
            if ($IsSelected) {
                Write-Tui "${esc}[35m→ ${esc}[48;5;53m${esc}[37m"
            }
            else {
                Write-Tui "  "
            }

            Write-Tui $rowPrefix

            $written = 0
            for ($i = 0; $i -lt $ModelName.Length; $i++) {
                if ($written -ge $modelWidth) {
                    break
                }

                $ch = $ModelName[$i]
                if ($mask[$i]) {
                    if ($IsSelected) {
                        Write-Tui "${esc}[48;5;53m${esc}[33m"
                    }
                    else {
                        Write-Tui "${esc}[33m"
                    }
                }
                elseif ($IsSelected) {
                    Write-Tui "${esc}[48;5;53m${esc}[37m"
                }
                else {
                    Write-Tui "${esc}[37m"
                }

                Write-Tui $ch
                $written++
            }

            $padding = $modelWidth - $written
            if ($padding -gt 0) {
                Write-Tui (" " * $padding)
            }

            Write-Tui "${esc}[0m`n"
        }

        Enable-VirtualTerminal
        Write-Tui "${esc}[H${esc}[2J"
        Save-Cursor
        Hide-Cursor

        $selection = $null
        try {
            while ($true) {
                $filtered = @(Get-FilteredModels $Models $query)
                $listHeight = [Math]::Max(1, [Math]::Min(10, [Console]::WindowHeight - 3))

                if ($filtered.Count -eq 0) {
                    $selected = 0
                }
                elseif ($selected -ge $filtered.Count) {
                    $selected = $filtered.Count - 1
                }
                elseif ($selected -lt 0) {
                    $selected = 0
                }

                if ($selected -lt $offset) {
                    $offset = $selected
                }
                elseif ($selected -ge $offset + $listHeight) {
                    $offset = $selected - $listHeight + 1
                }

                Clear-Frame

                Write-FrameLine ("cursor> {0}" -f $query) Cyan

                $current = if ($filtered.Count -eq 0) { 0 } else { $selected + 1 }
                $countText = "{0}/{1}" -f $current, $filtered.Count
                $frameWidth = Get-FrameWidth
                $ruleWidth = [Math]::Max(0, $frameWidth - $countText.Length - 1)
                Write-FrameLine ("{0} {1}" -f $countText, ("-" * $ruleWidth)) Gray DarkGray
                Write-FrameLine $helpText Cyan

                if ($filtered.Count -eq 0) {
                    Write-FrameLine "  no matches"
                }
                else {
                    $end = [Math]::Min($filtered.Count, $offset + $listHeight)
                    for ($i = $offset; $i -lt $end; $i++) {
                        Write-SelectorRow $filtered[$i] $query ($i -eq $selected)
                    }
                }

                Set-InputCursor $query
                Show-Cursor
                [Console]::Out.Flush()

                $key = [Console]::ReadKey($true)
                Hide-Cursor

                switch ($key.Key) {
                    "UpArrow" {
                        if ($selected -gt 0) {
                            $selected--
                        }
                    }
                    "DownArrow" {
                        if ($selected + 1 -lt $filtered.Count) {
                            $selected++
                        }
                    }
                    "Home" {
                        $selected = 0
                    }
                    "End" {
                        if ($filtered.Count -gt 0) {
                            $selected = $filtered.Count - 1
                        }
                    }
                    "Enter" {
                        if ($filtered.Count -gt 0) {
                            $selection = $filtered[$selected]
                            break
                        }
                    }
                    "Escape" {
                        break
                    }
                    "Backspace" {
                        if ($query.Length -gt 0) {
                            $query = $query.Substring(0, $query.Length - 1)
                            $selected = 0
                            $offset = 0
                        }
                    }
                    default {
                        if ($key.Modifiers -band [ConsoleModifiers]::Control) {
                            if ($key.Key -eq "N" -and $selected + 1 -lt $filtered.Count) {
                                $selected++
                            }
                            elseif ($key.Key -eq "P" -and $selected -gt 0) {
                                $selected--
                            }
                        }
                        elseif (-not [char]::IsControl($key.KeyChar)) {
                            $query += $key.KeyChar
                            $selected = 0
                            $offset = 0
                        }
                    }
                }

                if ($null -ne $selection -or $key.Key -eq "Escape") {
                    break
                }
            }
        }
        finally {
            Clear-Frame
            Write-Tui "${esc}[0m"
            Show-Cursor
            [Console]::Out.Flush()
        }

        if ($null -ne $selection) {
            return $selection
        }

        return $null
    }

    return $Models[0]
}

function Invoke-Claude([string[]]$ClaudeArgs) {
    if (-not (Test-Path $ConfigFile)) {
        Write-Warn "未找到配置，请先配置厂商地址和 API Key"
        Invoke-ConfigInteractive
    }

    $baseUrl = Get-ConfigValue "BASE_URL"
    $token = Get-AuthToken
    $models = @(Get-Models)

    if (-not $baseUrl -or -not $token -or $models.Count -eq 0) {
        Write-Err "配置不完整，请重新配置"
        Invoke-ConfigInteractive
        $baseUrl = Get-ConfigValue "BASE_URL"
        $token = Get-AuthToken
        $models = @(Get-Models)
    }

    $model = Select-Model $models
    if (-not $model) {
        return
    }

    Write-Info "模型: $model"
    Write-Info "地址: $baseUrl"

    $env:ANTHROPIC_BASE_URL = $baseUrl
    $env:ANTHROPIC_AUTH_TOKEN = $token
    $env:CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = "1"
    $env:CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS = "1"
    $env:CLAUDE_CODE_ATTRIBUTION_HEADER = "0"
    $env:CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS = "1"
    $env:CLAUDE_CODE_ENABLE_AUTO_MODE = "1"
    $env:CLAUDE_CODE_SUBAGENT_MODEL = $model
    $env:ANTHROPIC_MODEL = $model
    $env:ANTHROPIC_SMALL_FAST_MODEL = $model
    $env:ANTHROPIC_DEFAULT_SONNET_MODEL = $model
    $env:ANTHROPIC_DEFAULT_OPUS_MODEL = $model
    $env:ANTHROPIC_DEFAULT_HAIKU_MODEL = $model

    & claude --permission-mode bypassPermissions --effort max --model $model @ClaudeArgs
    exit $LASTEXITCODE
}

$command = if ($CliArgs.Count -gt 0) { $CliArgs[0] } else { "" }
$rest = if ($CliArgs.Count -gt 1) { $CliArgs[1..($CliArgs.Count - 1)] } else { @() }

try {
    switch ($command) {
        "run" { Invoke-Claude $rest }
        "config" { Invoke-Config $rest }
        "reset" { Remove-Config }
        "add-model" { Add-Model ($rest | Select-Object -First 1) }
        "change-token" { Change-Token ($rest | Select-Object -First 1) }
        default { Invoke-Claude $CliArgs }
    }
}
catch {
    Write-Err $_.Exception.Message
    exit 1
}
