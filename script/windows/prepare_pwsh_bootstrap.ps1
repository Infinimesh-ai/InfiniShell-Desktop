#!/usr/bin/env pwsh

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $DestinationPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$workspaceRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$assetsRoot = Join-Path $workspaceRoot 'app\assets'
$sourcePath = Join-Path $assetsRoot 'bundled\bootstrap\pwsh.ps1'
$source = [IO.File]::ReadAllText($sourcePath).TrimStart([char]0xFEFF)

$expanded = [Regex]::Replace(
    $source,
    '(?m)^[\t ]*#include (?<path>bundled/[^\r\n]+)\r?$',
    {
        param([Text.RegularExpressions.Match] $match)

        $relativePath = $match.Groups['path'].Value.Replace('/', [IO.Path]::DirectorySeparatorChar)
        $includePath = Join-Path $assetsRoot $relativePath
        if (-not (Test-Path -LiteralPath $includePath -PathType Leaf)) {
            throw "PowerShell bootstrap include does not exist: $includePath"
        }

        [IO.File]::ReadAllText($includePath).
            TrimStart([char]0xFEFF).
            TrimEnd([char[]]"`r`n")
    }
).Replace('@@USING_CON_PTY_BOOLEAN@@', 'true')

if ([Regex]::IsMatch($expanded, '(?m)^[\t ]*#include ')) {
    throw 'PowerShell bootstrap still contains an unexpanded include'
}

$null = [ScriptBlock]::Create($expanded)
$destinationDirectory = Split-Path -Parent $DestinationPath
if (-not [String]::IsNullOrEmpty($destinationDirectory)) {
    New-Item -ItemType Directory -Path $destinationDirectory -Force | Out-Null
}
[IO.File]::WriteAllText($DestinationPath, $expanded, [Text.UTF8Encoding]::new($false))
