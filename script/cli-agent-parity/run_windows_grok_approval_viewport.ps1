param([Parameter(Mandatory)][string]$Binary)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'This verification requires Windows' }
if (-not $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR) { throw 'Missing GUI artifacts directory' }

# 与既有剪贴板 GUI 验收共用已固定的 Mesa WGL 包和两个 DLL 摘要。
$directory = Join-Path $env:RUNNER_TEMP ('cli-grok-approval-view-' + [Guid]::NewGuid().ToString('N'))
$application = Join-Path $directory 'application'
$mesa = Join-Path $directory 'mesa'
$root = $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR
New-Item -ItemType Directory -Path $root -Force | Out-Null
$recordPath = Join-Path $root 'renderer.safe.json'
$record = [ordered]@{
    schema = 1
    test = 'test_cli_grok_approval_viewport'
    source_commit = $env:WARP_TEST_GUI_SOURCE_COMMIT
    outcome = 'pending'
    mesa_version = '26.2.1'
    mesa_archive_sha256 = '78a0305844074535e73dfb6dcb5eb2a65f1d9dc445102e1f4c3c3e97dace19bf'
    binary_sha256 = $null
    grok_sha256 = $null
    synthetic_approval_exercised = $false
    native_approval_exercised = $false
    v05_complete = $false
    locale_results = @()
}
$stdout = $null
$stderr = $null
try {
    if ($record.source_commit -notmatch '^[0-9a-f]{40}$') { throw 'Missing source commit' }
    if (-not $env:INFINISHELL_TEST_GROK_EXE) { throw 'Missing fixed Grok executable' }
    $grok = (Resolve-Path -LiteralPath $env:INFINISHELL_TEST_GROK_EXE).Path
    if ((Split-Path -Leaf $grok) -cne 'grok.exe') { throw 'Unexpected fixed Grok executable name' }
    $record.grok_sha256 = (Get-FileHash -LiteralPath $grok -Algorithm SHA256).Hash.ToLowerInvariant()
    New-Item -ItemType Directory -Path $application, $mesa -Force | Out-Null
    $archive = Join-Path $directory 'mesa.7z'
    Invoke-WebRequest -Uri 'https://github.com/pal1000/mesa-dist-win/releases/download/26.2.1/mesa3d-26.2.1-release-msvc.7z' -OutFile $archive
    if ((Get-Item -LiteralPath $archive).Length -ne 71046553 -or
        (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $record.mesa_archive_sha256) {
        throw 'Mesa archive digest or size mismatch'
    }
    & tar -xf $archive -C $mesa x64/opengl32.dll x64/libgallium_wgl.dll
    if ($LASTEXITCODE -ne 0) { throw 'Mesa extraction failed' }
    $mesaFiles = @{
        'opengl32.dll' = '7a8f7aa1b73099d24ddeca962a7e7360bccdfc234ff79cb557426e2a2ce3d4f6'
        'libgallium_wgl.dll' = '46ca4e92166f1389c8e9024dbeee0e24ccd5c46f9b6e2b048cc6bc476e6474d3'
    }
    foreach ($name in $mesaFiles.Keys) {
        $source = Join-Path $mesa "x64/$name"
        if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $mesaFiles[$name]) {
            throw "Mesa DLL digest mismatch: $name"
        }
        Copy-Item -LiteralPath $source -Destination (Join-Path $application $name)
    }
    $record.mesa_dll_sha256 = $mesaFiles

    # 复制同一构建的测试程序与 Windows 运行库，保持 Cargo 原文件不变。
    $binaryDirectory = Split-Path -Parent (Resolve-Path -LiteralPath $Binary).Path
    foreach ($name in @('integration.exe', 'conpty.dll', 'dxcompiler.dll', 'dxil.dll', 'x64/OpenConsole.exe')) {
        $source = Join-Path $binaryDirectory $name
        $destination = Join-Path $application $name
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        $digest = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Copy-Item -LiteralPath $source -Destination $destination
        if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant() -ne $digest) {
            throw "Runtime copy digest mismatch: $name"
        }
        if ($name -eq 'integration.exe') { $record.binary_sha256 = $digest }
    }

    foreach ($locale in @('en', 'zh-CN')) {
        foreach ($size in @('compact', 'normal')) {
            $output = Join-Path $root "$locale/$size"
            New-Item -ItemType Directory -Path $output -Force | Out-Null
            $stdout = Join-Path $directory "stdout-$locale-$size.log"
            $stderr = Join-Path $directory "stderr-$locale-$size.log"
            $result = [ordered]@{ locale = $locale; size = $size; outcome = 'pending'; exit_code = $null; renderer = @() }
            $record.locale_results += $result

            # 仅给本轮 integration 子进程追加固定 Grok 目录和 Mesa 渲染变量。
            $info = [System.Diagnostics.ProcessStartInfo]::new()
            $info.FileName = Join-Path $application 'integration.exe'
            $info.ArgumentList.Add('test_cli_grok_approval_viewport')
            $info.WorkingDirectory = (Get-Location).Path
            $info.UseShellExecute = $false
            $info.RedirectStandardOutput = $true
            $info.RedirectStandardError = $true
            $info.Environment['PATH'] = "$(Split-Path -Parent $grok);$env:PATH"
            $info.Environment['WGPU_BACKEND'] = 'gl'
            $info.Environment['GALLIUM_DRIVER'] = 'llvmpipe'
            $info.Environment['WARP_INTEGRATION'] = '1'
            $info.Environment['WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS'] = '1'
            $info.Environment['WARP_TEST_GUI_LOCALE'] = $locale
            $info.Environment['WARP_TEST_GUI_SIZE'] = $size
            $info.Environment['WARP_TEST_GUI_SOURCE_COMMIT'] = $record.source_commit
            $info.Environment['WARP_INTEGRATION_TEST_ARTIFACTS_DIR'] = $output
            $process = [System.Diagnostics.Process]::Start($info)
            if ($null -eq $process) { throw "Cannot start real GUI window: $locale/$size" }
            try {
                $outTask = $process.StandardOutput.ReadToEndAsync()
                $errTask = $process.StandardError.ReadToEndAsync()
                $timedOut = -not $process.WaitForExit(180000)
                if ($timedOut) {
                    $process.Kill($true)
                    $process.WaitForExit()
                }
                $outTask.GetAwaiter().GetResult() | Set-Content -LiteralPath $stdout -Encoding utf8
                $errTask.GetAwaiter().GetResult() | Set-Content -LiteralPath $stderr -Encoding utf8
                $result.exit_code = $process.ExitCode
                if ($timedOut) { throw "Real GUI window exceeded 180 seconds: $locale/$size" }
                if ($process.ExitCode -ne 0) { throw "Real GUI viewport failed: $locale/$size" }
            } finally {
                $process.Dispose()
            }
            $renderer = @(Select-String -LiteralPath $stdout, $stderr -Pattern 'Using Gl Cpu \(llvmpipe.*\) for rendering new window\.' |
                ForEach-Object { $_.Matches[0].Value })
            if ($renderer.Count -eq 0) { throw "Missing real Mesa window renderer: $locale/$size" }
            $result.renderer = $renderer
            python -B script/cli-agent-parity/verify_grok_approval_viewport.py `
                --root $output --binary $Binary --grok $grok --source $record.source_commit `
                --platform windows --locale $locale --size $size
            if ($LASTEXITCODE -ne 0) { throw "Grok synthetic approval receipt mismatch: $locale/$size" }
            $result.outcome = 'passed'
        }
    }
    $record.outcome = 'passed'
    $record.synthetic_approval_exercised = $true
} catch {
    $record.outcome = 'failed'
    if ($record.locale_results.Count -gt 0) {
        $last = $record.locale_results[$record.locale_results.Count - 1]
        if ($last.outcome -eq 'pending') { $last.outcome = 'failed' }
    }
    foreach ($log in @($stdout, $stderr)) {
        if ($log -and (Test-Path -LiteralPath $log)) {
            Select-String -LiteralPath $log -Pattern '\[ERROR\]|panicked at|failed to bootstrap|Test step.*failed' |
                Select-Object -First 30 | ForEach-Object { $_.Line }
            Get-Content -LiteralPath $log -Tail 80
        }
    }
    throw
} finally {
    $record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $recordPath -Encoding utf8
}
