param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('on-session-start.sh', 'on-prompt-submit.sh', 'on-stop.sh',
                 'on-permission-request.sh', 'on-post-tool-use.sh')]
    [string] $NotificationHook
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version 2

function Get-NotificationField($Object, [string] $Name, $Fallback) {
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value -or
        ($property.Value -is [bool] -and -not $property.Value)) {
        return ,$Fallback
    }
    return ,$property.Value
}

function ConvertTo-NotificationText($Value) {
    if ($null -eq $Value) { return '' }
    # 对齐 Bash 命令替换：只剥末尾 LF，保留内部 LF/CRLF 和末尾 CR。
    if ($Value -is [string]) { return $Value.TrimEnd([char] 10) }
    return (ConvertTo-Json -InputObject $Value -Compress -Depth 100)
}

function Assert-NotificationJsonUnicode([string] $Json) {
    # JSON 解码器可能替换孤立代理项；在解码前验证转义，双反斜杠表示的字面文本除外。
    for ($index = 0; $index -lt $Json.Length; $index++) {
        if ($Json[$index] -ne [char] 92) { continue }
        $index++
        if ($index -ge $Json.Length -or $Json[$index] -cne 'u') { continue }
        if ($index + 4 -ge $Json.Length) { throw 'notification_invalid_unicode' }
        $unit = [Convert]::ToInt32($Json.Substring($index + 1, 4), 16)
        $index += 4
        if ($unit -ge 0xD800 -and $unit -le 0xDBFF) {
            if ($index + 6 -ge $Json.Length -or $Json.Substring($index + 1, 2) -cne '\u') {
                throw 'notification_invalid_unicode'
            }
            $low = [Convert]::ToInt32($Json.Substring($index + 3, 4), 16)
            if ($low -lt 0xDC00 -or $low -gt 0xDFFF) { throw 'notification_invalid_unicode' }
            $index += 6
        } elseif ($unit -ge 0xDC00 -and $unit -le 0xDFFF) {
            throw 'notification_invalid_unicode'
        }
    }
}

function Limit-NotificationText([string] $Text, [int] $Maximum, [string] $Suffix = '') {
    # 按 Unicode 码点计数；验证完整输入，不让截断产生孤立代理项。
    $count = 0
    $cut = 0
    for ($index = 0; $index -lt $Text.Length; $index++) {
        if ($count -eq ($Maximum - $Suffix.Length)) { $cut = $index }
        $character = $Text[$index]
        if ([char]::IsHighSurrogate($character)) {
            if ($index + 1 -ge $Text.Length -or -not [char]::IsLowSurrogate($Text[$index + 1])) {
                throw 'notification_invalid_unicode'
            }
            $index++
        } elseif ([char]::IsLowSurrogate($character)) {
            throw 'notification_invalid_unicode'
        }
        $count++
    }
    if ($count -gt $Maximum) { return $Text.Substring(0, $cut) + $Suffix }
    return $Text
}

try {
    # 与 Unix 入口使用同一协商条件；门禁关闭时不读取输入，也不触碰控制台。
    if ([string]::IsNullOrEmpty($env:WARP_CLI_AGENT_PROTOCOL_VERSION) -or
        [string]::IsNullOrEmpty($env:WARP_CLIENT_VERSION)) { exit 0 }

    $writer = Join-Path $PSScriptRoot 'warp-notify.ps1'
    $writerItem = Get-Item -LiteralPath $writer -Force
    if ($writerItem.PSIsContainer -or ($writerItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'notification_writer_invalid'
    }
    . $writer

    # 直接读取 UTF-8 字节，避免 Windows PowerShell 5.1 的控制台代码页和 BOM 转码。
    $stream = [Console]::OpenStandardInput()
    $bytes = New-Object IO.MemoryStream
    $buffer = New-Object byte[] 8192
    try {
        while (($length = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            if ($bytes.Length + $length -gt 1048576) { throw 'notification_input_too_large' }
            $bytes.Write($buffer, 0, $length)
        }
        $utf8 = New-Object Text.UTF8Encoding($false, $true)
        $inputJson = $utf8.GetString($bytes.ToArray())
        Assert-NotificationJsonUnicode $inputJson
        $data = ConvertFrom-Json -InputObject $inputJson
    } finally {
        $bytes.Dispose()
    }
    if ($null -eq $data -or $data -isnot [Management.Automation.PSCustomObject]) {
        throw 'notification_input_not_object'
    }

    $version = 1L
    $advertised = 0L
    if ([long]::TryParse($env:WARP_CLI_AGENT_PROTOCOL_VERSION, [ref] $advertised) -and $advertised -lt 1) {
        $version = $advertised
    }
    $cwd = ConvertTo-NotificationText (Get-NotificationField $data 'cwd' '')
    $project = ''
    if ($cwd.Length -gt 0) {
        $project = [IO.Path]::GetFileName($cwd.TrimEnd([char[]] @('/', '\')))
        if ($project.Length -eq 0) { $project = $cwd }
    }
    $payload = [ordered] @{
        v = $version
        agent = 'codex'
        event = ''
        session_id = (ConvertTo-NotificationText (Get-NotificationField $data 'session_id' ''))
        cwd = $cwd
        project = $project
    }
    $turn = Get-NotificationField $data 'turn_id' ''
    if ($turn -is [string]) { $turn = ConvertTo-NotificationText $turn }
    if ($turn -is [string] -and $turn.Length -gt 0) { $payload['turn_id'] = $turn }

    switch ($NotificationHook) {
        'on-session-start.sh' {
            $payload.event = 'session_start'
            $payload['plugin_version'] = '0.4.0'
        }
        'on-prompt-submit.sh' {
            $payload.event = 'prompt_submit'
            $query = ConvertTo-NotificationText (Get-NotificationField $data 'prompt' '')
            $payload['query'] = Limit-NotificationText $query 200 '...'
        }
        'on-stop.sh' {
            $active = ConvertTo-NotificationText (Get-NotificationField $data 'stop_hook_active' $false)
            if ($active -ceq 'true') { exit 0 }
            $response = Get-NotificationField $data 'last_assistant_message' ''
            if ($response -isnot [string]) { $response = '' }
            $response = ConvertTo-NotificationText $response
            $payload.event = 'notification'
            if ($response.Length -gt 0) { $payload.event = 'stop' }
            $payload['response'] = Limit-NotificationText $response 200 '...'
            $payload['transcript_path'] = ConvertTo-NotificationText (Get-NotificationField $data 'transcript_path' '')
            if ($payload.event -eq 'stop' -and -not $payload.Contains('turn_id')) {
                $payload.event = 'notification'
                $payload['terminal_unverified'] = $true
                $payload['error_type'] = 'uncorrelated_hook'
            }
        }
        'on-permission-request.sh' {
            $payload.event = 'permission_request'
            $toolName = ConvertTo-NotificationText (Get-NotificationField $data 'tool_name' 'unknown')
            $toolInput = Get-NotificationField $data 'tool_input' ([pscustomobject] @{})
            $rawToolProperty = $data.PSObject.Properties['tool_input']
            $rawToolInput = $null
            if ($null -ne $rawToolProperty) { $rawToolInput = $rawToolProperty.Value }
            $preview = ''
            if ($null -eq $rawToolInput) {
                $preview = 'null'
            } elseif ($rawToolInput -is [Management.Automation.PSCustomObject]) {
                $command = Get-NotificationField $rawToolInput 'command' $null
                $file = Get-NotificationField $rawToolInput 'file_path' $null
                if ($null -ne $command) {
                    $preview = ConvertTo-NotificationText $command
                } elseif ($null -ne $file) {
                    $preview = ConvertTo-NotificationText $file
                } else {
                    $preview = Limit-NotificationText (ConvertTo-NotificationText $rawToolInput) 80
                }
            }
            $payload['summary'] = 'Wants to run ' + $toolName
            if ($preview.Length -gt 0) {
                $payload.summary += ': ' + (Limit-NotificationText $preview 120 '...')
            }
            $payload['tool_name'] = $toolName
            $payload['tool_input'] = $toolInput
        }
        'on-post-tool-use.sh' {
            $payload.event = 'tool_complete'
            $payload['tool_name'] = ConvertTo-NotificationText (Get-NotificationField $data 'tool_name' '')
        }
    }

    # 与 jq -a 相同只发 ASCII JSON；控制字符和代理对均留在 JSON 转义内，不能注入 OSC。
    $json = ConvertTo-Json -InputObject $payload -Compress -Depth 100
    $ascii = New-Object Text.StringBuilder
    foreach ($character in $json.ToCharArray()) {
        $value = [int] $character
        if ($value -lt 32 -or $value -gt 126) {
            [void] $ascii.Append(('\u{0:x4}' -f $value))
        } else {
            [void] $ascii.Append($character)
        }
    }
    Write-InfiniShellNotification (([string] [char] 27) + ']777;notify;warp://cli-agent;' + $ascii.ToString() + [char] 7)
    exit 0
} catch {
    # 不回显输入、路径或工具内容；失败也不能退回 stdout 污染 Codex 的 hook 协议。
    [Console]::Error.WriteLine('infinishell_codex_hook_transport_error: native_notification_failed')
    exit 1
}
