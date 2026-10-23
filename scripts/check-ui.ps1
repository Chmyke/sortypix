# -SkipAI: no Sortypix AI benchmark or diagnostic (check.ps1 passes its own -SkipAI).
param([switch]$SkipAI)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$run = Join-Path $root ('target/check-' + [Guid]::NewGuid().ToString('N'))
$photos = Join-Path $run 'photos'
$screens = Join-Path $run 'ui'
$benchmark = Join-Path $run 'benchmark'
& (Join-Path $root 'target/release/examples/fixtures.exe') $photos
if ($LASTEXITCODE -ne 0) { throw 'Fixture generation failed' }
$before = @(Get-ChildItem -LiteralPath $photos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
# A folder full of animated GIFs runs the same native benchmark: they must play within budgets.
$gifs = Join-Path $run 'gifs'
$gifBenchmark = Join-Path $run 'gif-benchmark'
& (Join-Path $root 'target/release/examples/fixtures.exe') $gifs '--gif'
if ($LASTEXITCODE -ne 0) { throw 'GIF fixture generation failed' }
$exe = Join-Path $root 'target/release/sortypix.exe'
$peaks = @{}
foreach ($mode in @(@('--diagnostic', $screens, $photos), @('--benchmark', $benchmark, $photos), @('--benchmark', $gifBenchmark, $gifs))) {
    $arguments = @($mode[0], ('"' + $mode[2] + '"'), ('"' + $mode[1] + '"'))
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -WindowStyle Hidden -PassThru
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $peakBytes = [long]0
    while (-not $process.WaitForExit(100)) {
        $process.Refresh()
        if (-not $process.HasExited) { $peakBytes = [Math]::Max($peakBytes, $process.PeakWorkingSet64) }
        if ($timer.Elapsed.TotalSeconds -gt 55) { Stop-Process -Id $process.Id; throw 'UI diagnostic timeout' }
    }
    if ($process.ExitCode -ne 0) { throw "UI exited with $($process.ExitCode)" }
    $failure = Join-Path $mode[1] 'FAIL.txt'
    if (Test-Path $failure) { throw ([IO.File]::ReadAllText($failure)) }
    $pass = Join-Path $mode[1] 'PASS.txt'
    if (-not (Test-Path $pass)) { throw "No successful UI report in $($mode[1])" }
    Write-Output ([IO.File]::ReadAllText($pass))
    $memory = ('Peak working set ({0}): {1:N1} MiB' -f $mode[0], ($peakBytes / 1MB))
    [IO.File]::WriteAllText((Join-Path $mode[1] 'memory.txt'), $memory)
    Write-Output $memory
    $peaks[$mode[1]] = $peakBytes
}
# Budgets: 192 MiB of animations + 256 MiB of stills + about 110 MiB of baseline process.
$gifReport = [IO.File]::ReadAllText((Join-Path $gifBenchmark 'PASS.txt'))
if ($gifReport -notmatch 'Animations loaded: (\d+)' -or [int]$Matches[1] -lt 12) { throw "GIFs did not animate: $gifReport" }
if ($peaks[$gifBenchmark] -gt 600MB) { throw ('GIF benchmark exceeded 600 MiB: {0:N1} MiB' -f ($peaks[$gifBenchmark] / 1MB)) }
# Sortypix AI: the same native benchmark on 200 camera-sized JPEGs while the analysis runs, with
# the budgets of the normal binary (the benchmark itself refuses a measure taken once the
# analysis is over). The normal binary runs first on the same folder, as the reference.
$aiExe = Join-Path $root 'target/release/sortypix-ai.exe'
$aiRoot = $env:SORTYPIX_AI_DIR
if (-not $aiRoot) { $aiRoot = Join-Path $env:LOCALAPPDATA 'Sortypix\ai-dev' }
if ($SkipAI) { Write-Output 'AI benchmark and AI UI diagnostic: SKIPPED (-SkipAI)' }
elseif (-not (Test-Path $aiExe) -or -not (Test-Path (Join-Path $aiRoot 'models'))) {
    Write-Output 'AI benchmark and AI UI diagnostic: SKIPPED (no sortypix-ai.exe or no models)'
} else {
    # Generated once under target/ (never in the repository) and reused.
    $large = Join-Path $root 'target/ai-benchmark-photos'
    & (Join-Path $root 'target/release/examples/fixtures.exe') $large '--large'
    if ($LASTEXITCODE -ne 0) { throw 'Large fixture generation failed' }
    $previousAiDir = $env:SORTYPIX_AI_DIR
    $env:SORTYPIX_AI_DIR = $aiRoot
    try {
        foreach ($mode in @(@($exe, (Join-Path $run 'large-benchmark')), @($aiExe, (Join-Path $run 'ai-benchmark')))) {
            # A fresh probe folder each run: no embedding is cached, the analysis really runs.
            $process = Start-Process -FilePath $mode[0] -ArgumentList @('--benchmark', ('"' + $large + '"'), ('"' + $mode[1] + '"')) -WindowStyle Hidden -PassThru
            if (-not $process.WaitForExit(60000)) { Stop-Process -Id $process.Id; throw "Benchmark timeout: $($mode[0])" }
            if ($process.ExitCode -ne 0) { throw "Benchmark exited with $($process.ExitCode): $($mode[0])" }
            $failure = Join-Path $mode[1] 'FAIL.txt'
            if (Test-Path $failure) { throw ("$($mode[0]): " + [IO.File]::ReadAllText($failure)) }
            $pass = Join-Path $mode[1] 'PASS.txt'
            if (-not (Test-Path $pass)) { throw "No successful benchmark report in $($mode[1])" }
            Write-Output ('{0} on 200 photos of 4000 x 3000:' -f (Split-Path -Leaf $mode[0]))
            Write-Output ([IO.File]::ReadAllText($pass))
        }
    } finally { $env:SORTYPIX_AI_DIR = $previousAiDir }
    $aiReport = [IO.File]::ReadAllText((Join-Path $run 'ai-benchmark/PASS.txt'))
    if ($aiReport -notmatch 'AI analysis at end: ready') { throw "AI benchmark ran without the analysis: $aiReport" }
    $aiReports = Join-Path $root 'target/reports'
    [IO.Directory]::CreateDirectory($aiReports) | Out-Null
    Copy-Item -LiteralPath (Join-Path $run 'ai-benchmark/performance.txt') -Destination (Join-Path $aiReports 'ui-ai-performance.txt')
    Write-Output 'AI benchmark during analysis: PASS'
    # Sortypix AI's own diagnostic: Sortypix's stages, then the analysis of the fixtures, marks,
    # Reject suggested and Ctrl+Z, search and every order of the menu (plan 4, task 4.5).
    $aiPhotos = Join-Path $run 'ai-photos'
    $aiScreens = Join-Path $run 'ai-ui'
    & (Join-Path $root 'target/release/examples/fixtures.exe') $aiPhotos
    if ($LASTEXITCODE -ne 0) { throw 'AI fixture generation failed' }
    $aiBefore = @(Get-ChildItem -LiteralPath $aiPhotos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
    $env:SORTYPIX_AI_DIR = $aiRoot
    try {
        $process = Start-Process -FilePath $aiExe -ArgumentList @('--diagnostic', ('"' + $aiPhotos + '"'), ('"' + $aiScreens + '"')) -WindowStyle Hidden -PassThru
        if (-not $process.WaitForExit(180000)) { Stop-Process -Id $process.Id; throw 'AI UI diagnostic timeout' }
        if ($process.ExitCode -ne 0) { throw "AI UI diagnostic exited with $($process.ExitCode)" }
    } finally { $env:SORTYPIX_AI_DIR = $previousAiDir }
    $failure = Join-Path $aiScreens 'FAIL.txt'
    if (Test-Path $failure) { throw ('AI UI diagnostic: ' + [IO.File]::ReadAllText($failure)) }
    $pass = Join-Path $aiScreens 'PASS.txt'
    if (-not (Test-Path $pass)) { throw "No successful AI UI report in $aiScreens" }
    Write-Output ([IO.File]::ReadAllText($pass))
    $aiAfter = @(Get-ChildItem -LiteralPath $aiPhotos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
    if (Compare-Object $aiBefore $aiAfter) { throw 'AI UI actions and undo changed fixture bytes' }
    if ((Get-ChildItem -LiteralPath $aiPhotos -Directory).Count -ne 0) { throw 'AI UI undo left a sorting directory behind' }
    Write-Output "AI UI diagnostic and byte restoration: PASS ($aiScreens)"
}
$session = Join-Path $run 'session'
$other = Join-Path $run 'other'
& (Join-Path $root 'target/release/examples/fixtures.exe') $other
if ($LASTEXITCODE -ne 0) { throw 'Session fixture generation failed' }
foreach ($mode in @('--session-store', '--session-restore', '--session-other')) {
    $source = if ($mode -eq '--session-other') { $other } else { $photos }
    $process = Start-Process -FilePath $exe -ArgumentList @($mode, ('"' + $source + '"'), ('"' + $session + '"')) -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(20000)) { Stop-Process -Id $process.Id; throw 'Session probe timeout' }
    if ($process.ExitCode -ne 0) { throw "Session probe exited with $($process.ExitCode)" }
    if (Test-Path (Join-Path $session 'FAIL.txt')) { throw ([IO.File]::ReadAllText((Join-Path $session 'FAIL.txt'))) }
    $report = Join-Path $session ($mode.TrimStart('-') + '.PASS.txt')
    if (-not (Test-Path $report)) { throw "Session report missing: $mode" }
    Write-Output ([IO.File]::ReadAllText($report))
}
$after = @(Get-ChildItem -LiteralPath $photos -File | Sort-Object Name | Get-FileHash | Select-Object -ExpandProperty Hash)
if (Compare-Object $before $after) { throw 'UI actions and undo changed fixture bytes' }
if ((Get-ChildItem -LiteralPath $photos -Directory).Count -ne 0) { throw 'Undo left a sorting directory behind' }
$reports = Join-Path $root 'target/reports'
[IO.Directory]::CreateDirectory($reports) | Out-Null
[IO.File]::WriteAllText((Join-Path $reports 'latest-ui.txt'), $run, (New-Object Text.UTF8Encoding($false)))
Copy-Item -LiteralPath (Join-Path $benchmark 'performance.txt') -Destination (Join-Path $reports 'ui-performance.txt')
Write-Output "UI and byte restoration: PASS ($run)"
