[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Baseline,
    [Parameter(Mandatory)][string]$Candidate,
    [int]$Rounds = 4,
    [int]$Seconds = 12,
    [int]$WarmupSeconds = 6,
    [switch]$Video,
    [switch]$Fixture,
    [string]$Output = 'artifacts\performance.json'
)
$ErrorActionPreference = 'Stop'
if ($Fixture) { $Video = $true }
$oldWebviewArgs = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class ReleaseProbe {
  public const string FocusName="\uD3EC\uCEE4\uC2A4";
  public const string GridName="\uACA9\uC790";
  [DllImport("user32.dll", SetLastError=true)]
  public static extern bool SetWindowPos(IntPtr hwnd, IntPtr insertAfter, int x, int y, int width, int height, uint flags);
  [DllImport("user32.dll")]
  public static extern bool SetForegroundWindow(IntPtr hwnd);
  [DllImport("user32.dll")]
  public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", SetLastError=true)]
  public static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
}
'@
$baselinePath = (Resolve-Path -LiteralPath $Baseline).Path
$candidatePath = (Resolve-Path -LiteralPath $Candidate).Path
if (Get-Process Moabom -ErrorAction SilentlyContinue) { throw 'Close the running Moabom before benchmarking' }
$config = Join-Path $env:APPDATA 'io.github.moabom\settings.json'
$hadConfig = Test-Path -LiteralPath $config
$backup = if ($hadConfig) { [IO.File]::ReadAllBytes($config) } else { $null }
$rows = [System.Collections.Generic.List[object]]::new()
function Write-Fixture {
    $tiles = @()
    if ($Video) {
        $tiles = @(@{input='youtube:aqz-KE-bpKQ'; platform='youtube'; volume=0; mix=$false},
                   @{input='youtube:YE7VzlLtp-4'; platform='youtube'; volume=0; mix=$false})
    }
    $quality = if ($Video) { 'low' } else { 'auto' }
    $saved = @{ restore=$true; notify=$false; layout='grid'; quality=$quality; splitSoop=$true;
        chzzkHq=$true; tiles=$tiles; selected=$null; groups=@() }
    New-Item -ItemType Directory -Force (Split-Path -Parent $config) | Out-Null
    [IO.File]::WriteAllText($config, ($saved | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
}
function Read-Usage($births) {
    $cpu = 0.0; $ws = 0.0; $private = 0.0; $count = 0
    $times = @{}; $names = @{}
    foreach ($processId in $births.Keys) {
        $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
        if ($process) {
            # Windows reuses PIDs; an unrelated process must never enter the sample.
            if ([Math]::Abs(($process.StartTime - $births[$processId]).TotalMilliseconds) -ge 1) { continue }
            $cpu += $process.TotalProcessorTime.TotalSeconds
            $ws += $process.WorkingSet64
            $private += $process.PrivateMemorySize64
            $count++
            $times[$processId] = $process.TotalProcessorTime.TotalSeconds
            $names[$processId] = $process.ProcessName
        }
    }
    return @{cpu=$cpu; ws=$ws/1MB; private=$private/1MB; count=$count; times=$times; names=$names}
}
function Read-MediaState($element) {
    $condition = [Windows.Automation.PropertyCondition]::new(
        [Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Slider)
    $values = @()
    foreach ($slider in $element.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)) {
        $value = $null
        $pattern = $null
        if ($slider.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) {
            $value = $pattern.Current.Value
        } elseif ($slider.TryGetCurrentPattern([Windows.Automation.RangeValuePattern]::Pattern, [ref]$pattern)) {
            $value = $pattern.Current.Value
        }
        $values += @{name=$slider.Current.Name; value=$value}
    }
    return $values
}
function Test-MediaProgress($first, $second) {
    $first = @($first | Where-Object { $_.value -is [string] -and $_.value -match '/\d' })
    $second = @($second | Where-Object { $_.value -is [string] -and $_.value -match '/\d' })
    if ($first.Count -ne 2 -or $second.Count -ne 2) { return $false }
    foreach ($slider in $second) {
        $duration = ($slider.value -split '/',2)[1]
        $prior = @($first | Where-Object { ($_.value -split '/',2)[1] -eq $duration })
        if ($prior.Count -ne 1 -or $prior[0].value -eq $slider.value) { return $false }
    }
    return $true
}
try {
    for ($round=0; $round -lt $Rounds; $round++) {
        $order = if ($round % 2 -eq 0) { @('baseline','candidate') } else { @('candidate','baseline') }
        foreach ($label in $order) {
            Write-Fixture
            $exe = if ($label -eq 'baseline') { $baselinePath } else { $candidatePath }
            $watch = [Diagnostics.Stopwatch]::StartNew()
            if ($Fixture) { $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=0' }
            $app = Start-Process -FilePath $exe -PassThru
            $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $oldWebviewArgs
            try {
                $ready = $false
                while ($watch.Elapsed.TotalSeconds -lt 30) {
                    $app.Refresh()
                    if ($app.HasExited) { throw "$label exited during startup" }
                    if ($app.MainWindowHandle -ne [IntPtr]::Zero) {
                        $element = [Windows.Automation.AutomationElement]::FromHandle($app.MainWindowHandle)
                        $condition = [Windows.Automation.PropertyCondition]::new(
                            [Windows.Automation.AutomationElement]::ControlTypeProperty,
                            [Windows.Automation.ControlType]::Edit)
                        $input = $element.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
                        if ($input -and $input.Current.IsEnabled) { $ready = $true; break }
                    }
                    Start-Sleep -Milliseconds 25
                }
                if (-not $ready) { throw "$label UI not ready after 30 seconds" }
                $startup = $watch.Elapsed.TotalMilliseconds
                # Position only after Tauri has finished creating its child webviews.
                if (-not [ReleaseProbe]::SetWindowPos($app.MainWindowHandle, [IntPtr](-1), 100, 100, 1296, 839, 0x40)) {
                    throw 'Cannot position the benchmark window'
                }
                [void][ReleaseProbe]::SetForegroundWindow($app.MainWindowHandle)
                Start-Sleep -Seconds $WarmupSeconds
                $layoutTimes = @()
                # Measure a real UI -> JavaScript -> IPC -> Rust -> settings write roundtrip.
                foreach ($target in @('focus','grid','focus','grid','focus','grid')) {
                    $name = if ($target -eq 'focus') { [ReleaseProbe]::FocusName } else { [ReleaseProbe]::GridName }
                    $buttonCondition = [Windows.Automation.PropertyCondition]::new(
                        [Windows.Automation.AutomationElement]::NameProperty, $name)
                    $button = $element.FindFirst([Windows.Automation.TreeScope]::Descendants, $buttonCondition)
                    if (-not $button) { throw "Layout control not found: $target" }
                    $pattern = $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)
                    $latency = [Diagnostics.Stopwatch]::StartNew()
                    $pattern.Invoke()
                    $changed = $false
                    while ($latency.Elapsed.TotalSeconds -lt 5) {
                        $stream = [IO.File]::Open($config, [IO.FileMode]::Open, [IO.FileAccess]::Read,
                            ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
                        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8)
                        try { $savedState = $reader.ReadToEnd() | ConvertFrom-Json }
                        finally { $reader.Dispose() }
                        if ($savedState.layout -eq $target) { $changed = $true; break }
                        Start-Sleep -Milliseconds 5
                    }
                    if (-not $changed) { throw "UI/IPC layout command failed: $target" }
                    $layoutTimes += $latency.Elapsed.TotalMilliseconds
                }
                $mediaBefore = @()
                if ($Fixture) {
                    $prepared = & node (Join-Path $PSScriptRoot 'media-fixture.mjs') prepare
                    if ($LASTEXITCODE) { throw 'Local media fixture setup failed' }
                    Start-Sleep -Seconds 8
                    $mediaBefore = (& node (Join-Path $PSScriptRoot 'media-fixture.mjs') metrics) | ConvertFrom-Json
                    if ($LASTEXITCODE) { throw 'Local media fixture metrics failed' }
                } elseif ($Video) {
                    $mediaReady = $false
                    $mediaWait = [Diagnostics.Stopwatch]::StartNew()
                    while ($mediaWait.Elapsed.TotalSeconds -lt 120) {
                        $first = @(Read-MediaState $element | Where-Object { $_.value -is [string] -and $_.value -match '/\d' })
                        Start-Sleep -Seconds 3
                        $second = @(Read-MediaState $element | Where-Object { $_.value -is [string] -and $_.value -match '/\d' })
                        if (Test-MediaProgress $first $second) {
                            $mediaReady = $true; $mediaBefore = $second; break
                        }
                    }
                    if (-not $mediaReady) { throw 'Both videos must be playing before measuring steady-state performance' }
                    # Allow page loading/JIT work to settle after playback is confirmed.
                    Start-Sleep -Seconds 8
                    $mediaBefore = @(Read-MediaState $element)
                }
                $all = @(Get-CimInstance Win32_Process | Where-Object { $_.CreationDate -ge $app.StartTime } |
                    Select-Object ProcessId,ParentProcessId,CreationDate,CommandLine)
                $ids = [System.Collections.Generic.HashSet[int]]::new()
                [void]$ids.Add($app.Id)
                $births = @{}; $births[$app.Id] = $app.StartTime
                do {
                    $added = $false
                    foreach ($p in $all) {
                        if ($ids.Contains([int]$p.ParentProcessId) -and
                            $p.CreationDate -ge $births[[int]$p.ParentProcessId] -and $ids.Add([int]$p.ProcessId)) {
                            $births[[int]$p.ProcessId] = $p.CreationDate
                            $added=$true
                        }
                    }
                } while ($added)
                $previous = Read-Usage $births
                if ($previous.count -lt 2) { throw 'App and WebView2 process identity verification failed' }
                $foregroundBefore = [ReleaseProbe]::GetForegroundWindow() -eq $app.MainWindowHandle
                $cpuDeltas = @{}; $processNames = @{}
                $clock = [Diagnostics.Stopwatch]::StartNew()
                $samples = @()
                for ($sample=0; $sample -lt $Seconds; $sample++) {
                    Start-Sleep -Seconds 1
                    $usage = Read-Usage $births
                    foreach ($processId in $usage.times.Keys) {
                        $processNames[$processId] = $usage.names[$processId]
                        if ($previous.times.ContainsKey($processId)) {
                            $delta = [Math]::Max(0.0, [double]($usage.times[$processId] - $previous.times[$processId]))
                            $cpuDeltas[$processId] += $delta
                        }
                    }
                    $previous = $usage
                    $elapsed = $clock.Elapsed.TotalSeconds
                    $ping = [Diagnostics.Stopwatch]::StartNew()
                    $reply = [IntPtr]::Zero
                    $ok = [ReleaseProbe]::SendMessageTimeout($app.MainWindowHandle, 0, [IntPtr]::Zero, [IntPtr]::Zero, 2, 1000, [ref]$reply)
                    $ping.Stop()
                    if ($ok -eq [IntPtr]::Zero) { throw "$label window is unresponsive" }
                    $samples += [ordered]@{workingSetMB=$usage.ws; privateMB=$usage.private;
                        responseMs=$ping.Elapsed.TotalMilliseconds; processCount=$usage.count}
                    $last = $usage
                }
                $mediaAfter = @()
                if ($Fixture) {
                    $mediaAfter = (& node (Join-Path $PSScriptRoot 'media-fixture.mjs') metrics) | ConvertFrom-Json
                    if ($LASTEXITCODE) { throw 'Local media fixture metrics failed' }
                    if ($mediaBefore.Count -ne 2 -or $mediaAfter.Count -ne 2) { throw 'Expected two playing fixtures' }
                    for ($i=0; $i -lt 2; $i++) {
                        if ($mediaAfter[$i].paused -or $mediaAfter[$i].totalFrames -le $mediaBefore[$i].totalFrames) {
                            throw 'Fixture video playback did not progress'
                        }
                        if (-not $mediaAfter[$i].videoOnly -or -not $mediaAfter[$i].overlay -or -not $mediaAfter[$i].watchdog) {
                            throw 'The application initialization scripts must run on the fixtures'
                        }
                    }
                } elseif ($Video) { $mediaAfter = @(Read-MediaState $element) }
                if ($Video -and -not $Fixture -and -not (Test-MediaProgress $mediaBefore $mediaAfter)) {
                    throw 'Playback did not progress on both videos during this measurement; result is invalid'
                }
                $processCpu = foreach ($processId in $cpuDeltas.Keys) {
                    $info = $all | Where-Object { $_.ProcessId -eq $processId } | Select-Object -First 1
                    $role = 'browser'
                    if ($processId -eq $app.Id) { $role='app' }
                    elseif ($info.CommandLine -match '--type=([^ ]+)') { $role=$Matches[1] }
                    @{name=$processNames[$processId]; id=$processId;
                        role=$role;
                        oneCorePercent=100*$cpuDeltas[$processId]/$elapsed}
                }
                $row = [ordered]@{label=$label; round=$round+1; startupMs=$startup;
                    layoutRoundtripMs=$layoutTimes;
                    mediaBefore=$mediaBefore; mediaAfter=$mediaAfter;
                    processCpu=@($processCpu);
                    measurementSeconds=$elapsed;
                    foregroundBefore=$foregroundBefore;
                    foregroundAfter=([ReleaseProbe]::GetForegroundWindow() -eq $app.MainWindowHandle);
                    cpuOneCorePercent=100*($cpuDeltas.Values | Measure-Object -Sum).Sum/$elapsed;
                    samples=$samples; workload= $(if ($Fixture) {'two-local-720p-vp8'} elseif ($Video) {'two-youtube-pages'} else {'empty-window'})}
                $rows.Add($row)
                Write-Output ("{0} round {1}: UI {2:N1} ms, tree CPU {3:N3}% of one core, private {4:N1} MB, foreground {5}/{6}" -f
                    $label,($round+1),$startup,$row.cpuOneCorePercent,($samples.privateMB | Measure-Object -Average).Average,
                    $row.foregroundBefore,$row.foregroundAfter)
            } finally {
                if (-not $app.HasExited) {
                    $app.CloseMainWindow() | Out-Null
                    if (-not $app.WaitForExit(5000)) { Stop-Process -Id $app.Id -Force }
                }
                Start-Sleep -Seconds 2
            }
        }
    }
} finally {
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $oldWebviewArgs
    if ($hadConfig) { [IO.File]::WriteAllBytes($config, $backup) }
    elseif (Test-Path -LiteralPath $config) { Remove-Item -LiteralPath $config }
    $result = [ordered]@{timestamp=(Get-Date).ToString('o');
        baselineSHA256=(Get-FileHash -LiteralPath $baselinePath).Hash;
        candidateSHA256=(Get-FileHash -LiteralPath $candidatePath).Hash;
        logicalProcessors=[Environment]::ProcessorCount; warmupSeconds=$WarmupSeconds; rows=$rows.ToArray()}
    $outputPath = [IO.Path]::GetFullPath($Output)
    New-Item -ItemType Directory -Force (Split-Path -Parent $outputPath) | Out-Null
    [IO.File]::WriteAllText($outputPath, ($result | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
}
