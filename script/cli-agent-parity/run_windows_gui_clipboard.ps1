param([Parameter(Mandatory)][string]$Binary)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'This verification requires Windows' }
if ($env:WARP_TEST_GUI_SOURCE_COMMIT -notmatch '^[0-9a-f]{40}$') { throw 'Missing source commit' }
if (-not $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR) { throw 'Missing GUI artifacts directory' }

# 服务会话的 DXGI 不可用；只在本次私有目录使用固定 Mesa WGL，仍创建真实窗口。
$directory = Join-Path $env:RUNNER_TEMP ('cli-gui-wgl-' + [Guid]::NewGuid().ToString('N'))
$executableDirectory = Join-Path $directory 'application'
$extractDirectory = Join-Path $directory 'mesa'
New-Item -ItemType Directory -Path $executableDirectory, $extractDirectory -Force | Out-Null
New-Item -ItemType Directory -Path $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR -Force | Out-Null
$recordPath = Join-Path $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR 'renderer.safe.json'
$record = [ordered]@{
    schema = 1
    source_commit = $env:WARP_TEST_GUI_SOURCE_COMMIT
    outcome = 'pending'
    renderer = $null
    exit_code = $null
    mesa_version = '26.2.1'
    mesa_archive_url = 'https://github.com/pal1000/mesa-dist-win/releases/download/26.2.1/mesa3d-26.2.1-release-msvc.7z'
    mesa_archive_sha256 = '78a0305844074535e73dfb6dcb5eb2a65f1d9dc445102e1f4c3c3e97dace19bf'
    copied_runtime_files = @()
    locale_results = @()
    model_inputs = 0
}
$savedBackend = $env:WGPU_BACKEND
$savedDriver = $env:GALLIUM_DRIVER
$savedLocale = $env:WARP_TEST_GUI_LOCALE
$savedArtifacts = $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR
$currentLocale = $null
$stdout = Join-Path $directory 'stdout.log'
$stderr = Join-Path $directory 'stderr.log'
try {
    $archive = Join-Path $directory 'mesa.7z'
    Invoke-WebRequest -Uri $record.mesa_archive_url -OutFile $archive
    if ((Get-Item -LiteralPath $archive).Length -ne 71046553 -or
        (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $record.mesa_archive_sha256) {
        throw 'Mesa archive digest or size mismatch'
    }
    & tar -xf $archive -C $extractDirectory x64/opengl32.dll x64/libgallium_wgl.dll
    if ($LASTEXITCODE -ne 0) { throw 'Mesa extraction failed' }
    $mesaFiles = @{
        'opengl32.dll' = '7a8f7aa1b73099d24ddeca962a7e7360bccdfc234ff79cb557426e2a2ce3d4f6'
        'libgallium_wgl.dll' = '46ca4e92166f1389c8e9024dbeee0e24ccd5c46f9b6e2b048cc6bc476e6474d3'
    }
    foreach ($name in $mesaFiles.Keys) {
        $source = Join-Path $extractDirectory "x64/$name"
        if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $mesaFiles[$name]) {
            throw "Mesa DLL digest mismatch: $name"
        }
        Copy-Item -LiteralPath $source -Destination (Join-Path $executableDirectory $name)
    }
    $record.mesa_dll_sha256 = $mesaFiles

    # 保持 Cargo 产物不变，复制同一构建的 EXE 及其运行库；不安装系统级 DLL。
    $binaryDirectory = Split-Path -Parent (Resolve-Path -LiteralPath $Binary).Path
    foreach ($name in @('integration.exe', 'conpty.dll', 'dxcompiler.dll', 'dxil.dll', 'x64/OpenConsole.exe')) {
        $source = Join-Path $binaryDirectory $name
        $destination = Join-Path $executableDirectory $name
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        $digest = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
        Copy-Item -LiteralPath $source -Destination $destination
        if ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant() -ne $digest) {
            throw "Runtime copy digest mismatch: $name"
        }
        $record.copied_runtime_files += @{ name = $name; sha256 = $digest }
    }
    $env:WGPU_BACKEND = 'gl'
    $env:GALLIUM_DRIVER = 'llvmpipe'
    $binaryDigest = ($record.copied_runtime_files | Where-Object { $_.name -eq 'integration.exe' }).sha256
    # 共用一次 Mesa 和程序准备，每种语言都启动新窗口，避免缓存文案混用。
    foreach ($locale in @('en', 'zh-CN')) {
        $env:WARP_TEST_GUI_LOCALE = $locale
        $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR = Join-Path $savedArtifacts $locale
        New-Item -ItemType Directory -Path $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR | Out-Null
        $stdout = Join-Path $directory "stdout-$locale.log"
        $stderr = Join-Path $directory "stderr-$locale.log"
        $currentLocale = [ordered]@{
            ui_locale = $locale
            outcome = 'pending'
            exit_code = $null
            renderer = @()
        }
        $record.locale_results += $currentLocale
        $process = Start-Process -FilePath (Join-Path $executableDirectory 'integration.exe') `
            -ArgumentList 'test_cli_composer_system_clipboard_multiline_and_image' `
            -WorkingDirectory (Get-Location).Path -RedirectStandardOutput $stdout -RedirectStandardError $stderr `
            -NoNewWindow -PassThru
        if (-not $process.WaitForExit(180000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "Real Windows GUI initialization or clipboard test exceeded 180 seconds: $locale"
        }
        $currentLocale.exit_code = $process.ExitCode
        $record.exit_code = $process.ExitCode
        $renderer = @(Select-String -LiteralPath $stdout, $stderr -Pattern 'Using Gl Cpu \(llvmpipe.*\) for rendering new window\.' | ForEach-Object { $_.Matches[0].Value })
        $currentLocale.renderer = $renderer
        $record.renderer = $renderer
        if ($process.ExitCode -ne 0) { throw "Real Windows window or system clipboard verification failed: $locale" }
        if ($renderer.Count -eq 0) { throw "Missing actual GL llvmpipe window renderer evidence: $locale" }
        $receipts = @(Get-ChildItem -LiteralPath $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR -Recurse -Filter 'receipt.safe.json')
        if ($receipts.Count -ne 1) { throw "Expected exactly one real clipboard receipt: $locale" }
        $receipt = Get-Content -LiteralPath $receipts[0].FullName -Raw | ConvertFrom-Json
        if ($receipt.source_commit -ne $record.source_commit -or $receipt.binary_sha256 -ne $binaryDigest -or
            $receipt.ui_locale -cne $locale -or
            $receipt.desktop_default_features_applied -cne $true -or
            $receipt.managed_feature_test_override -cne $false -or $receipt.managed_feature_enabled -cne $true -or
            -not $receipt.system_clipboard -or -not $receipt.text_exact -or -not $receipt.image_pixels_exact -or
            $receipt.model_inputs -ne 0 -or $receipt.managed_tasks_created -ne 0 -or $receipt.screenshots.Count -ne 2) {
            throw "Real clipboard receipt does not match this binary, UI language, or required assertions: $locale"
        }
        $currentLocale.outcome = 'passed'
    }
    $record.outcome = 'passed'
} catch {
    $record.outcome = 'failed'
    if ($null -ne $currentLocale -and $currentLocale.outcome -ne 'passed') { $currentLocale.outcome = 'failed' }
    # 失败日志留在本次私有目录，CI 输出保留尾部诊断；不伪造截图或剪贴板收据。
    foreach ($log in @($stdout, $stderr)) {
        if (Test-Path -LiteralPath $log) {
            # 长回溯可能挤掉实际原因，先显示有界错误行，再保留末尾上下文。
            Select-String -LiteralPath $log -Pattern '\[ERROR\]|panicked at|failed to bootstrap' |
                Select-Object -First 30 | ForEach-Object { $_.Line }
            Select-String -LiteralPath $log -Pattern 'VideoRecorder:|Frame capture|Failed to render frame|Test step.*succeeded|GUI 引导失败' |
                Select-Object -Last 20 | ForEach-Object { $_.Line }
            Get-Content -LiteralPath $log -Tail 80
        }
    }
    throw
} finally {
    $record | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $recordPath -Encoding utf8
    $env:WGPU_BACKEND = $savedBackend
    $env:GALLIUM_DRIVER = $savedDriver
    $env:WARP_TEST_GUI_LOCALE = $savedLocale
    $env:WARP_INTEGRATION_TEST_ARTIFACTS_DIR = $savedArtifacts
}
