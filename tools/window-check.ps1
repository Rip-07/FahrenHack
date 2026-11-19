# Checks the real desktop window (WebView2), not a browser: UI loaded, window buttons, optional real mouse drag.
#   powershell -ExecutionPolicy Bypass -File tools\window-check.ps1 [-Exe path] [-Drag]
# Uses a temporary data dir, so the user's %APPDATA%\FahrenHack is untouched.
# -Drag moves the real mouse cursor for about a second (it is put back afterwards).
param([string]$Exe = "$PSScriptRoot\..\target\release\fahrenhack.exe", [switch]$Drag)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\cdp.ps1"
if (-not ('FhW32' -as [type])) {
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class FhW32 {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
}
"@
}
function Rect($h) { $r = New-Object FhW32+RECT; [void][FhW32]::GetWindowRect($h, [ref]$r); $r }

$Exe = (Resolve-Path $Exe).Path
Get-Process fahrenhack -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800
$data = Join-Path $env:TEMP 'fahrenhack-window-check'
$env:FAHRENHACK_DATA = $data
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
$p = Start-Process $Exe -PassThru
try {
  $ws = Get-CdpPage 9223
  Start-Sleep 3
  "UI: " + (Invoke-CdpEval $ws "JSON.stringify({ desktopClass: document.body.classList.contains('desktop'), view: document.body.dataset.view, clock: document.querySelector('#clock').textContent, tiles: document.querySelectorAll('#shelf .tile').length, hints: document.querySelectorAll('#hints .hint').length, winButtons: [...document.querySelectorAll('.winctl button')].filter(b => b.offsetWidth > 0).length })")
  $shot = Join-Path $env:TEMP 'fahrenhack-window.png'
  Save-CdpScreenshot $ws $shot
  "screenshot: $shot"

  $h = [IntPtr]::Zero
  for ($i = 0; $i -lt 20 -and $h -eq [IntPtr]::Zero; $i++) { $p.Refresh(); $h = $p.MainWindowHandle; Start-Sleep -Milliseconds 200 }

  if ($Drag) {
    # Real mouse drag on an empty spot of the header, only if that spot belongs to our window.
    $s = [FhW32]::GetDpiForWindow($h) / 96.0
    $o = New-Object FhW32+POINT; [void][FhW32]::ClientToScreen($h, [ref]$o)
    $cssX = 300; $cssY = 33
    "element at header point: " + (Invoke-CdpEval $ws "(() => { const e = document.elementFromPoint($cssX, $cssY); return e ? e.tagName + (e.id ? '#' + e.id : '') + (e.closest('button') ? ' (inside button!)' : '') : null })()")
    [void][FhW32]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0040)
    Start-Sleep -Milliseconds 300
    $pt = New-Object FhW32+POINT; $pt.X = [int]($o.X + $cssX * $s); $pt.Y = [int]($o.Y + $cssY * $s)
    $root = [FhW32]::GetAncestor([FhW32]::WindowFromPoint($pt), 2)
    if ($root -ne $h) { "SKIP drag: point ($($pt.X),$($pt.Y)) is not over the FahrenHack window" }
    else {
      $saved = New-Object FhW32+POINT; [void][FhW32]::GetCursorPos([ref]$saved)
      $r0 = Rect $h
      [void][FhW32]::SetCursorPos($pt.X, $pt.Y); Start-Sleep -Milliseconds 150
      [FhW32]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 200
      for ($k = 1; $k -le 12; $k++) { [void][FhW32]::SetCursorPos($pt.X + 12 * $k, $pt.Y + 6 * $k); Start-Sleep -Milliseconds 25 }
      Start-Sleep -Milliseconds 150
      [FhW32]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 400
      $r1 = Rect $h
      [void][FhW32]::SetCursorPos($saved.X, $saved.Y)
      "drag: window moved by ($($r1.L - $r0.L), $($r1.T - $r0.T)) px, expected about (144, 72)"
    }
    [void][FhW32]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x0001 -bor 0x0002)
  }

  # Window buttons use the same IPC path (window.ipc.postMessage) as dragging.
  [void](Invoke-CdpEval $ws "window.ipc.postMessage('max')"); Start-Sleep -Milliseconds 700
  "after 'max': zoomed=$([FhW32]::IsZoomed($h))"
  [void](Invoke-CdpEval $ws "window.ipc.postMessage('max')"); Start-Sleep -Milliseconds 700
  "after 'max' again: zoomed=$([FhW32]::IsZoomed($h))"
  [void](Invoke-CdpEval $ws "window.ipc.postMessage('min')"); Start-Sleep -Milliseconds 700
  "after 'min': minimized=$([FhW32]::IsIconic($h))"
} finally {
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
  Remove-Item Env:FAHRENHACK_DATA -ErrorAction SilentlyContinue
}
