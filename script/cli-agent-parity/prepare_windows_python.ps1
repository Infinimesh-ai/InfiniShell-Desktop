# 使用 Python 官方的 CI 包，仅解压到本次作业目录，不运行系统安装器。
$ErrorActionPreference = 'Stop'
$version = '3.13.15'
$expectedSize = 14391248
$expectedSha256 = '05357887df50d3153efc681bdf432c321d3e2f9ce5788f99f4515b27e8fda0ac'
if (-not $IsWindows -or -not $env:RUNNER_TEMP -or -not $env:GITHUB_PATH) {
    throw 'This helper requires a Windows GitHub Actions job.'
}
$operationRoot = Join-Path $env:RUNNER_TEMP ('infinishell-python-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $operationRoot -ErrorAction Stop | Out-Null
$archive = Join-Path $operationRoot 'python.zip'
$package = Join-Path $operationRoot 'package'
$source = "https://api.nuget.org/v3-flatcontainer/python/$version/python.$version.nupkg"
$downloadAttempts = 3
for ($attempt = 1; $attempt -le $downloadAttempts; $attempt++) {
    Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue
    try {
        Invoke-WebRequest -Uri $source -OutFile $archive -TimeoutSec 120
        break
    }
    catch {
        Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue
        if ($attempt -eq $downloadAttempts) {
            throw
        }
        Write-Warning "Python package download failed; retrying attempt $($attempt + 1)/$downloadAttempts."
        Start-Sleep -Seconds (2 * $attempt)
    }
}
if ((Get-Item -LiteralPath $archive).Length -ne $expectedSize -or
    (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expectedSha256) {
    throw 'The fixed Python package size or SHA256 does not match.'
}
Expand-Archive -LiteralPath $archive -DestinationPath $package
$pythonDirectory = Join-Path $package 'tools'
$pythonExecutable = Join-Path $pythonDirectory 'python.exe'
# 先确认当前解释器、架构与探针依赖，再向后续步骤发布 PATH。
& $pythonExecutable -I -c 'import ctypes,json,os,ssl,struct,sys,tomllib; assert os.name == "nt" and sys.version_info[:3] == (3,13,15) and struct.calcsize("P") == 8; print(json.dumps({"version":sys.version,"executable":sys.executable,"pointer_bits":struct.calcsize("P")*8}))'
if ($LASTEXITCODE -ne 0) {
    throw 'The job-local Python interpreter failed its version, architecture, or standard-library check.'
}
$pythonDirectory | Out-File -FilePath $env:GITHUB_PATH -Append -Encoding utf8
Write-Host "Verified Python $version package SHA256 $expectedSha256; published the job-local interpreter."
