# Full screen opens on the screen Sortypix is on (Mickael, 2026-10-30): on every connected
# screen, and after the window is moved to another one. Skipped with a single screen.
# Launches the release build with throwaway settings; keys are posted to Sortypix only.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$screens = [System.Windows.Forms.Screen]::AllScreens
if ($screens.Count -lt 2) {
    Write-Output 'Full screen on the window screen: SKIPPED (one screen)'
    return
}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SortypixWindow {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Ri, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  delegate bool Each(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(Each f, IntPtr l);
  public static IntPtr Largest(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, l) => {
      uint owner; GetWindowThreadProcessId(h, out owner);
      R r;
      if (owner == pid && IsWindowVisible(h) && GetWindowRect(h, out r)) {
        long a = (long)(r.Ri - r.L) * (r.B - r.T);
        if (a > area) { area = a; best = h; }
      }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
'@
function Get-Rect($h) {
    $r = New-Object SortypixWindow+R
    [void][SortypixWindow]::GetWindowRect($h, [ref]$r)
    New-Object System.Drawing.Rectangle($r.L, $r.T, ($r.Ri - $r.L), ($r.B - $r.T))
}
function Send-Key($h, [int]$key, [int]$scan) {
    [void][SortypixWindow]::PostMessage($h, 0x0100, [IntPtr]$key, [IntPtr](1 -bor ($scan -shl 16)))
    [void][SortypixWindow]::PostMessage($h, 0x0101, [IntPtr]$key, [IntPtr](1 -bor ($scan -shl 16) -bor 0xC0000000))
}
$run = Join-Path $root ('target/fullscreen-' + [Guid]::NewGuid().ToString('N'))
$photos = Join-Path $run 'photos'
& (Join-Path $root 'target/release/examples/fixtures.exe') $photos | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Fixture generation failed' }
$failures = @()
$cases = @()
foreach ($screen in $screens) { $cases += , @($screen, $null) }
# Opened on the first screen, moved to the second before full screen.
$cases += , @($screens[0], $screens[1])
$previous = $env:LOCALAPPDATA
try {
    foreach ($case in $cases) {
        $start, $moveTo = $case
        $app = Join-Path $run ('app-' + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force (Join-Path $app 'Sortypix') | Out-Null
        $x = $start.WorkingArea.X + 120
        $y = $start.WorkingArea.Y + 80
        $settings = @{ last_folder = $photos; window = @{ position_pixels = @($x, $y); size = @(900, 600); maximized = $false } } | ConvertTo-Json -Depth 4
        [IO.File]::WriteAllText((Join-Path $app 'Sortypix/settings.json'), $settings)
        $env:LOCALAPPDATA = $app
        $process = Start-Process -FilePath (Join-Path $root 'target/release/sortypix.exe') -PassThru
        try {
            $h = [IntPtr]::Zero
            for ($i = 0; $i -lt 100 -and ((Get-Rect $h).Width -lt 200); $i++) {
                Start-Sleep -Milliseconds 100
                $h = [SortypixWindow]::Largest([uint32]$process.Id)
            }
            Start-Sleep -Seconds 2
            if ($moveTo) {
                [void][SortypixWindow]::SetWindowPos($h, [IntPtr]::Zero, ($moveTo.WorkingArea.X + 120), ($moveTo.WorkingArea.Y + 80), 0, 0, 0x0015)
                Start-Sleep -Milliseconds 800
            }
            $before = Get-Rect $h
            $expected = [System.Windows.Forms.Screen]::FromRectangle($before).Bounds
            $after = $before
            # Enter opens the photo in full screen; it is sent again if the first one came early.
            for ($try = 0; $try -lt 3 -and $after -eq $before; $try++) {
                Send-Key $h 0x0D 0x1C
                Start-Sleep -Milliseconds 1500
                $after = Get-Rect $h
            }
            $label = if ($moveTo) { "moved to $($moveTo.DeviceName)" } else { "opened on $($start.DeviceName)" }
            if ($after -ne $expected) {
                $failures += "Full screen $label covered $after instead of $expected"
            }
        } finally {
            Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
            $process.WaitForExit(3000) | Out-Null
        }
    }
} finally {
    $env:LOCALAPPDATA = $previous
    Remove-Item -Recurse -Force $run -ErrorAction SilentlyContinue
}
if ($failures.Count -gt 0) { throw ($failures -join "`n") }
Write-Output ("Full screen on the window screen: PASS ({0} screens, and after a move)" -f $screens.Count)
