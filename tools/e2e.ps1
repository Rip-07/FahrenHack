# End-to-end check of the cheat engine against tools/dummy-game (no window, HTTP API only):
# auto-attach, hidden-value calibration (snapshot + decreased), pointer scan, freeze,
# exact-value calibration, set, restart persistence, re-confirmation, settings override.
#   powershell -ExecutionPolicy Bypass -File tools\e2e.ps1
# Builds the debug app and the dummy game first. Uses a temporary data dir.
# Keep this file ASCII-only: Windows PowerShell 5.1 reads BOM-less scripts as ANSI.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..").Path
Get-Process fahrenhack, dummygame2 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

Push-Location $root
cargo build -q
if ($LASTEXITCODE) { throw 'cargo build failed' }
cargo build -q --release --manifest-path tools\dummy-game\Cargo.toml --target-dir target\dummy
if ($LASTEXITCODE) { throw 'dummy game build failed' }
Pop-Location

$exe = "$root\target\debug\fahrenhack.exe"
$dummy = "$root\target\dummy\release\dummygame2.exe"
$data = Join-Path $env:TEMP 'fahrenhack-e2e'
$log = Join-Path $env:TEMP 'fahrenhack-e2e.log'
$out = Join-Path $env:TEMP 'fahrenhack-e2e-game.txt'
Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$data\games" | Out-Null
$profile = @'
{
  "id": "dummy", "title": "Dummy Game", "subtitle": "test", "processes": ["dummygame2.exe"], "accent": "#2f8f6a",
  "pointer_scan": { "max_depth": 4, "max_offset": 4096 },
  "cheats": [
    { "id": "hp", "category": "Player", "icon": "heart", "name": "HP", "kind": "freeze", "vtype": "i32", "value": null, "hotkey": "F9",
      "locator": { "type": "calibrate", "mode": "unknown", "steps": ["snapshot", "decreased"] } },
    { "id": "ammo", "category": "Weapons", "icon": "ammo", "name": "Ammo", "kind": "set", "vtype": "i32", "value": "999",
      "locator": { "type": "calibrate", "mode": "exact" } }
  ]
}
'@
[IO.File]::WriteAllText("$data\games\dummy.json", $profile)

$env:FAHRENHACK_DATA = $data
$t = Start-Process $exe -ArgumentList '--headless' -PassThru -WindowStyle Hidden -RedirectStandardOutput $log
$g = Start-Process $dummy -PassThru -WindowStyle Hidden -RedirectStandardOutput $out
$fail = 0
function Check($ok, $what) { if ($ok) { "PASS  $what" } else { "FAIL  $what"; $script:fail++ } }
try {
  Start-Sleep 4
  $port = [regex]::Match((Get-Content $log -Raw -Encoding utf8), '127\.0\.0\.1:(\d+)').Groups[1].Value
  $token = (Get-Content "$data\settings.json" -Raw | ConvertFrom-Json).token
  $h = @{ 'x-token' = $token }
  $b = "http://127.0.0.1:$port/api"
  function J($m, $p, $body) {
    if ($null -ne $body) { Invoke-RestMethod "$b$p" -Method $m -Headers $h -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes(($body | ConvertTo-Json -Compress))) }
    else { Invoke-RestMethod "$b$p" -Method $m -Headers $h }
  }
  function WaitJob($r) {
    while ($true) { Start-Sleep -Milliseconds 200; $s = J GET /job; if ($s.id -eq $r.job -and $s.done) { if ($s.error) { throw "job: $($s.error)" }; return $s.result } }
  }
  function Last { Get-Content $out -Tail 1 }
  function Ammo { [int]([regex]::Match((Last), 'ammo=(\d+)').Groups[1].Value) }
  function Hp { [int]([regex]::Match((Last), 'hp=(-?\d+)').Groups[1].Value) }

  $denied = $false; try { Invoke-RestMethod "$b/state" | Out-Null } catch { $denied = $true }
  Check $denied 'API rejects requests without the token'
  $s = J GET /state
  Check ($s.attached.exe -eq 'dummygame2.exe') "auto-attached to the game (build $($s.attached.build))"

  # HP: hidden value -> snapshot + decreased
  WaitJob (J POST /scan/unknown @{ vtype = 'i32' }) | Out-Null
  foreach ($i in 1..3) { Start-Sleep -Milliseconds 1300; $r = WaitJob (J POST /scan/next @{ mode = 'decreased' }) }
  $res = @(J GET /scan/results | ForEach-Object { $_ })
  Check ($res.Count -ge 1 -and $res.Count -le 3) "hidden-value search narrowed to $($res.Count) candidate(s)"
  $r = WaitJob (J POST /calib/hp/bind @{ addr = $res[0].addr; vtype = 'i32' })
  Check ($r.chains -gt 0) "pointer scan found $($r.chains) chain(s)"
  $cal = Get-Content (Get-ChildItem "$data\calibration\dummy\*.json").FullName -Raw | ConvertFrom-Json
  $first = $cal.cheats.hp.chains[0]
  Check (($first.offsets | Select-Object -Skip 1) -join ',' -eq '0x40,0x18') "best chain is $($first.module) $($first.offsets -join ' -> ')"
  J POST /cheats/hp/toggle | Out-Null
  Start-Sleep 3
  $hp1 = Hp; Start-Sleep 2; $hp2 = Hp
  Check ($hp1 -eq $hp2) "freeze holds HP ($hp1 = $hp2)"

  # Ammo: visible value -> exact
  # The game changes ammo on its own timer, so a scan can race with it: retry a few times.
  $res = @()
  for ($try = 1; $try -le 3 -and $res.Count -eq 0; $try++) {
    $a1 = Ammo
    $deadline = (Get-Date).AddSeconds(8)
    while ((Ammo) -eq $a1 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }  # start right after a change
    $a1 = Ammo
    WaitJob (J POST /scan/first @{ vtype = 'i32'; value = "$a1" }) | Out-Null
    $deadline = (Get-Date).AddSeconds(8)
    while ((Ammo) -eq $a1 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
    WaitJob (J POST /scan/next @{ mode = 'exact'; value = "$(Ammo)" }) | Out-Null
    $res = @(J GET /scan/results | ForEach-Object { $_ })
  }
  Check ($res.Count -ge 1) "exact search found ammo ($($res.Count) candidate(s))"
  WaitJob (J POST /calib/ammo/bind @{ addr = $res[0].addr; vtype = 'i32' }) | Out-Null
  J POST /cheats/ammo/toggle | Out-Null
  Start-Sleep 1.2
  Check ((Ammo) -eq 999) "set writes ammo = 999 (game shows $(Ammo))"

  # Restart the game: cheats must resolve from saved chains without calibration
  Stop-Process -Id $g.Id; Start-Sleep 1.5
  $g = Start-Process $dummy -PassThru -WindowStyle Hidden -RedirectStandardOutput $out
  Start-Sleep 4
  $s = J GET /state
  $hpC = $s.cheats | Where-Object id -eq 'hp'
  Check ($s.attached.pid -eq $g.Id) 're-attached after restart'
  Check ($hpC.status -eq 'ready' -and [math]::Abs([int]$hpC.current - (Hp)) -le 7) "HP resolves after restart (cheat sees $($hpC.current), game shows $(Hp))"
  J POST /cheats/hp/toggle | Out-Null
  Start-Sleep 3
  $hp1 = Hp; Start-Sleep 2; $hp2 = Hp
  Check ($hp1 -eq $hp2) "freeze holds HP after restart ($hp1 = $hp2)"
  $r = WaitJob (J POST /calib/hp/bind @{ addr = $hpC.addr; vtype = 'i32' })
  Check ($r.how -eq 'confirmed' -and $r.confirmations -eq 2) "re-calibration confirms chains ($($r.chains) left, confirmations $($r.confirmations))"

  J PUT /games/dummy/cheats/ammo @{ value = '555'; hotkey = 'CTRL+F6' } | Out-Null
  $c = (@(J GET /games | ForEach-Object { $_ }) | Where-Object id -eq 'dummy').cheats | Where-Object id -eq 'ammo'
  Check ($c.value -eq '555' -and $c.hotkey -eq 'CTRL+F6') 'settings override saved'
  $rejected = $false; try { J PUT /games/dummy/cheats/ammo @{ value = 'abc'; hotkey = '' } | Out-Null } catch { $rejected = $true }
  Check $rejected 'invalid value rejected'
} finally {
  Get-Process fahrenhack, dummygame2 -ErrorAction SilentlyContinue | Stop-Process -Force
  Remove-Item Env:FAHRENHACK_DATA -ErrorAction SilentlyContinue
}
Check (-not (Get-ChildItem $env:TEMP -Filter 'fahrenhack-*.snap')) 'no snapshot files left in TEMP'
if ($fail) { "$fail check(s) FAILED"; exit 1 } else { 'ALL CHECKS PASSED' }
