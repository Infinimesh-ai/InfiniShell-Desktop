#requires -Version 5.1
<#
.SYNOPSIS
    解析 InfiniShell BYOP prompt cache 命中率(基于 chat_stream.rs::generate_byop_output
    在每次流末打印的 `[byop-cache]` 日志行)。

.DESCRIPTION
    1. 自动定位 InfiniShell 日志文件:`%LOCALAPPDATA%\infinishell\InfiniShell\data\logs\infinishell.log`
    2. grep 形如下面格式的行:
       [byop-cache] prompt_tokens=N cache_read=R (X.X%) cache_create=W (Y.Y%) model=M compaction=L
       其中 compaction= 是 P2-16 添加的可选字段(none / inactive / active(hidden=N))
    3. 按 model 分组聚合,输出每个模型的:
       - 请求次数
       - 平均 cache_read ratio (主要命中指标)
       - 平均 cache_create ratio (写入指标,首请求会高,后续应低)
       - 总 prompt tokens / 总 cache_read tokens / 总 cache_create tokens
       - 压缩相关请求统计(P2-16)
    4. 提供"对比模式"(-Tail N) 仅看最近 N 条记录,适合做 A/B

.PARAMETER LogPath
    自定义日志路径。默认从 InfiniShell 标准位置查找。

.PARAMETER Tail
    只分析最近 N 条 [byop-cache] 行(默认全部)。

.PARAMETER Watch
    持续 tail 日志,实时打印新出现的命中率行(Ctrl+C 退出)。

.EXAMPLE
    .\analyze-prompt-cache.ps1
.EXAMPLE
    .\analyze-prompt-cache.ps1 -Tail 20
.EXAMPLE
    .\analyze-prompt-cache.ps1 -Watch
.EXAMPLE
    .\analyze-prompt-cache.ps1 -LogPath "D:\backup\infinishell.log"

.NOTES
    需要 InfiniShell 启用 INFO 级日志(`[byop-cache]` 是 log::info!)。
    若没有任何 `[byop-cache]` 行:
      - 上游 provider 没返回 cache 字段(DeepSeek/Ollama 隐式缓存可能就是 0)
      - 或者 RUST_LOG 把 INFO 过滤了
#>
[CmdletBinding()]
param(
    [string]$LogPath,
    [int]$Tail = 0,
    [switch]$Watch
)

$ErrorActionPreference = 'Stop'

# ---------- 1. 定位日志 ----------
function Resolve-InfiniShellLog {
    param([string]$Override)
    if ($Override) {
        if (-not (Test-Path -LiteralPath $Override)) {
            throw "指定的日志路径不存在: $Override"
        }
        return (Resolve-Path -LiteralPath $Override).Path
    }
    $candidates = @()
    if ($env:LOCALAPPDATA) {
        # 当前版本路径(`crates/simple_logger/src/manager.rs::log_directory_path` Windows 分支)
        $candidates += (Join-Path -Path $env:LOCALAPPDATA -ChildPath 'infinishell\InfiniShell\data\logs\infinishell.log')
        # 备选(以前版本的路径)
        $candidates += (Join-Path -Path $env:LOCALAPPDATA -ChildPath 'infinishell\InfiniShell\data\infinishell.log')
        $candidates += (Join-Path -Path $env:LOCALAPPDATA -ChildPath 'infinishell\InfiniShell\infinishell.log')
    }
    if ($env:APPDATA) {
        $candidates += (Join-Path -Path $env:APPDATA -ChildPath 'infinishell\InfiniShell\data\logs\infinishell.log')
        $candidates += (Join-Path -Path $env:APPDATA -ChildPath 'infinishell\InfiniShell\data\infinishell.log')
    }
    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c)) { return (Resolve-Path -LiteralPath $c).Path }
    }
    throw @"
未找到 InfiniShell 日志文件。请检查以下位置或用 -LogPath 显式指定:
  $($candidates -join "`n  ")
若 InfiniShell 还没运行过,先启动一次再来跑此脚本。
"@
}

# ---------- 2. 解析单行 ----------
# 行格式（单行，可能因终端宽度被换行，但 log crate 自带换行只在末尾）：
# [byop-cache] prompt_tokens=12345 cache_read=10000 (81.0%) cache_create=200 (1.6%) model=claude-opus-4-7 compaction=none
# compaction=字段是 P2-16 添加的，取值：none / inactive / active(hidden=N)。
# 为了兼容老日志，compaction 字段设为可选。
$cacheLineRegex = [regex]'\[byop-cache\]\s+prompt_tokens=(?<prompt>\d+)\s+cache_read=(?<read>\d+)\s+\(\s*(?<read_pct>[\d\.]+)%\)\s+cache_create=(?<create>\d+)\s+\(\s*(?<create_pct>[\d\.]+)%\)\s+model=(?<model>\S+?)(?:\s+compaction=(?<compaction>\S+))?$'

function Parse-CacheLine {
    param([string]$Line)
    $m = $cacheLineRegex.Match($Line)
    if (-not $m.Success) { return $null }
    $compactionRaw = if ($m.Groups['compaction'].Success) { $m.Groups['compaction'].Value } else { '' }
    [pscustomobject]@{
        Timestamp    = $null
        PromptTokens = [int]$m.Groups['prompt'].Value
        CacheRead    = [int]$m.Groups['read'].Value
        CacheCreate  = [int]$m.Groups['create'].Value
        ReadPct      = [double]$m.Groups['read_pct'].Value
        CreatePct    = [double]$m.Groups['create_pct'].Value
        Model        = $m.Groups['model'].Value
        # P2-16: 压缩状态. 取值: ''(老日志) / 'none' / 'inactive' / 'active(hidden=N)'
        Compaction   = $compactionRaw
        Raw          = $Line
    }
}

# ---------- 3. 聚合输出 ----------
function Format-Summary {
    param([System.Collections.IList]$Records)
    if ($Records.Count -eq 0) {
        Write-Host '没有匹配到任何 [byop-cache] 行。' -ForegroundColor Yellow
        Write-Host @'

可能原因:
  1. 还没用 BYOP 路径发起过请求(InfiniShell 启动后没和 AI 对话过)
  2. 上游 provider 没返回 cache 字段(DeepSeek/Ollama 服务端隐式缓存)
  3. RUST_LOG 把 INFO 级日志过滤了 - 检查启动环境变量

排查步骤:
  $env:RUST_LOG = 'info'   # 启动 InfiniShell 前设置
  在 InfiniShell 中向 AI 发 2 条消息(同一对话),让其调起 BYOP
  然后重跑本脚本
'@ -ForegroundColor Yellow
        return
    }

    Write-Host ''
    Write-Host '========== InfiniShell BYOP Prompt Cache 命中率分析 ==========' -ForegroundColor Cyan
    Write-Host ("总匹配行数: {0}" -f $Records.Count)

    # P2-16: 压缩相关汇总
    $compactionActive = @($Records | Where-Object { $_.Compaction -like 'active*' })
    if ($compactionActive.Count -gt 0) {
        Write-Host ("  └─ 其中走压缩路径: {0} 条" -f $compactionActive.Count) -ForegroundColor DarkYellow
    }
    Write-Host ''

    # 按 model 分组
    $byModel = $Records | Group-Object Model

    $byModel | ForEach-Object {
        $model = $_.Name
        $rs    = $_.Group
        $n     = $rs.Count
        $sumPrompt = ($rs | Measure-Object PromptTokens -Sum).Sum
        $sumRead   = ($rs | Measure-Object CacheRead    -Sum).Sum
        $sumCreate = ($rs | Measure-Object CacheCreate  -Sum).Sum
        $avgReadPct   = ($rs | Measure-Object ReadPct   -Average).Average
        $avgCreatePct = ($rs | Measure-Object CreatePct -Average).Average

        $globalReadPct = if ($sumPrompt -gt 0) { 100.0 * $sumRead / $sumPrompt } else { 0.0 }
        $globalCreatePct = if ($sumPrompt -gt 0) { 100.0 * $sumCreate / $sumPrompt } else { 0.0 }

        Write-Host ("Model: {0}" -f $model) -ForegroundColor Green
        Write-Host ("  请求数:           {0}" -f $n)
        Write-Host ("  总 prompt tokens: {0:N0}" -f $sumPrompt)
        Write-Host ("  总 cache_read:    {0:N0}  ({1:F1}% of total)" -f $sumRead,   $globalReadPct)
        Write-Host ("  总 cache_create:  {0:N0}  ({1:F1}% of total)" -f $sumCreate, $globalCreatePct)
        Write-Host ("  平均 read ratio:  {0:F1}%   <- 主要命中率指标(>=20% 算正常,>=50% 优秀)" -f $avgReadPct)
        Write-Host ("  平均 create ratio:{0:F1}%   <- 应该随轮数下降" -f $avgCreatePct)

        # 趋势分析(轮数 vs read ratio):看是否随对话推进命中率上升
        if ($n -ge 3) {
            $trend = $rs | ForEach-Object -Begin { $i = 0 } -Process {
                $i++
                $marker = if ($_.Compaction -like 'active*') { '*' } else { '' }
                "{0}{1}:{2:F0}%" -f $i, $marker, $_.ReadPct
            }
            Write-Host ("  Read ratio 趋势:  {0}" -f ($trend -join ' -> '))
            if ($rs | Where-Object { $_.Compaction -like 'active*' }) {
                Write-Host ("  (* = 走压缩路径,该轮 cache miss 是预期)") -ForegroundColor DarkGray
            }
        }
        Write-Host ''
    }

    # 全局健康度判断
    $allReadPct = ($Records | Measure-Object ReadPct -Average).Average
    Write-Host '----------- 全局健康度 -----------' -ForegroundColor Cyan
    if ($allReadPct -ge 50) {
        Write-Host ("✅ 全局平均命中率 {0:F1}% - 优秀" -f $allReadPct) -ForegroundColor Green
    } elseif ($allReadPct -ge 20) {
        Write-Host ("⚠️  全局平均命中率 {0:F1}% - 正常,但有提升空间" -f $allReadPct) -ForegroundColor Yellow
    } else {
        Write-Host ("❌ 全局平均命中率 {0:F1}% - 偏低,可能有 prefix 不稳定问题" -f $allReadPct) -ForegroundColor Red
        Write-Host '   排查方向: 检查 system prompt 是否含每请求都变的字段,MCP tools 顺序是否稳定'
    }

    if ($compactionActive.Count -gt 0) {
        $nonCompactionRecords = @($Records | Where-Object { $_.Compaction -notlike 'active*' })
        if ($nonCompactionRecords.Count -gt 0) {
            $nonCompactionAvg = ($nonCompactionRecords | Measure-Object ReadPct -Average).Average
            Write-Host ("ℹ️  排除压缩轮后平均命中率 {0:F1}% (n={1})" -f $nonCompactionAvg, $nonCompactionRecords.Count) -ForegroundColor DarkCyan
        }
    }
}

# ---------- 4. 主流程 ----------
$logFile = Resolve-InfiniShellLog -Override $LogPath
Write-Host "日志路径: $logFile" -ForegroundColor DarkGray

if ($Watch) {
    Write-Host '进入 watch 模式,Ctrl+C 退出。新增的 [byop-cache] 行将实时输出:' -ForegroundColor Cyan
    Get-Content -LiteralPath $logFile -Wait -Tail 0 | ForEach-Object {
        $rec = Parse-CacheLine $_
        if ($rec) {
            $color = if ($rec.ReadPct -ge 50) { 'Green' }
                     elseif ($rec.ReadPct -ge 20) { 'Yellow' }
                     else { 'Red' }
            $compactionTag = if ($rec.Compaction) { " [$($rec.Compaction)]" } else { '' }
            $msg = '[{0}] read={1:F1}% create={2:F1}% prompt={3} model={4}{5}' -f `
                (Get-Date -Format 'HH:mm:ss'), $rec.ReadPct, $rec.CreatePct, $rec.PromptTokens, $rec.Model, $compactionTag
            Write-Host $msg -ForegroundColor $color
        }
    }
    return
}

# 静态分析(一次性扫描)
$records = New-Object System.Collections.ArrayList
Get-Content -LiteralPath $logFile -ReadCount 1000 | ForEach-Object {
    foreach ($line in $_) {
        $rec = Parse-CacheLine $line
        if ($rec) { [void]$records.Add($rec) }
    }
}

if ($Tail -gt 0 -and $records.Count -gt $Tail) {
    $records = [System.Collections.ArrayList]::new(
        $records.GetRange($records.Count - $Tail, $Tail)
    )
    Write-Host "(只统计最近 $Tail 条)" -ForegroundColor DarkGray
}

Format-Summary -Records $records
