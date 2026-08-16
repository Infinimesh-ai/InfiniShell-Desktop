#!/usr/bin/env pwsh

$ErrorActionPreference = 'Stop'

$testDirectory = Join-Path ([IO.Path]::GetTempPath()) "infinishell-package-test-$([Guid]::NewGuid())"
try {
    $resourcesDirectory = Join-Path $testDirectory 'input-resources'
    $nestedResourcesDirectory = Join-Path $resourcesDirectory 'bundled/skills'
    New-Item -ItemType Directory -Path $nestedResourcesDirectory -Force | Out-Null
    $binaryPath = Join-Path $testDirectory 'source.exe'
    [IO.File]::WriteAllBytes($binaryPath, [byte[]](0x4d, 0x5a, 0x01))
    Set-Content -LiteralPath (Join-Path $nestedResourcesDirectory 'skill.md') -Value 'test skill'

    $archivePath = Join-Path $testDirectory 'infinishell-windows-x86_64.zip'
    & "$PSScriptRoot/package_remote_server.ps1" `
        -BinaryPath $binaryPath `
        -ResourcesDirectory $resourcesDirectory `
        -DestinationPath $archivePath

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        $entryNames = @($archive.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
        if ($entryNames -notcontains 'infinishell.exe') {
            throw "Archive is missing infinishell.exe: $($entryNames -join ', ')"
        }
        if ($entryNames -notcontains 'resources/bundled/skills/skill.md') {
            throw "Archive is missing bundled resources: $($entryNames -join ', ')"
        }
    } finally {
        $archive.Dispose()
    }
} finally {
    if (Test-Path -LiteralPath $testDirectory) {
        Remove-Item -LiteralPath $testDirectory -Recurse -Force
    }
}
