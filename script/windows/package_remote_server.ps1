#!/usr/bin/env pwsh

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $BinaryPath,
    [Parameter(Mandatory = $true)]
    [string] $ResourcesDirectory,
    [Parameter(Mandatory = $true)]
    [string] $DestinationPath
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "Windows remote-server binary does not exist: $BinaryPath"
}
if (-not (Test-Path -LiteralPath $ResourcesDirectory -PathType Container)) {
    throw "Windows remote-server resources directory does not exist: $ResourcesDirectory"
}

$destinationDirectory = Split-Path -Parent $DestinationPath
if ($destinationDirectory) {
    New-Item -ItemType Directory -Path $destinationDirectory -Force | Out-Null
}

$stagingDirectory = Join-Path ([IO.Path]::GetTempPath()) "infinishell-remote-server-$([Guid]::NewGuid())"
try {
    New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
    Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $stagingDirectory 'infinishell.exe')
    Copy-Item -LiteralPath $ResourcesDirectory -Destination (Join-Path $stagingDirectory 'resources') -Recurse

    $archiveEntries = @(
        (Join-Path $stagingDirectory 'infinishell.exe'),
        (Join-Path $stagingDirectory 'resources')
    )
    Compress-Archive -LiteralPath $archiveEntries -DestinationPath $DestinationPath -CompressionLevel Optimal -Force
} finally {
    if (Test-Path -LiteralPath $stagingDirectory) {
        Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
    }
}
