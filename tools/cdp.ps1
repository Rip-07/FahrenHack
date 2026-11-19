# Minimal Chrome DevTools Protocol client for the WebView2 inside the FahrenHack window.
# Start the app with $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223', then:
#   . .\tools\cdp.ps1 ; $ws = Get-CdpPage 9223 ; Invoke-CdpEval $ws 'document.title'

function Get-CdpPage([int]$port) {
  for ($i = 0; $i -lt 40; $i++) {
    try {
      $t = @(Invoke-RestMethod "http://127.0.0.1:$port/json" -TimeoutSec 2 | ForEach-Object { $_ }) |
        Where-Object { $_.type -eq 'page' -and $_.url -like 'http://127.0.0.1:*' } | Select-Object -First 1
      if ($t) { return $t.webSocketDebuggerUrl }
    } catch { }
    Start-Sleep -Milliseconds 250
  }
  throw "no CDP page on port $port"
}

function Invoke-Cdp([string]$ws, [string]$method, $params = @{}) {
  $client = [System.Net.WebSockets.ClientWebSocket]::new()
  $ct = [Threading.CancellationToken]::None
  $client.ConnectAsync([Uri]$ws, $ct).Wait()
  $msg = @{ id = 1; method = $method; params = $params } | ConvertTo-Json -Depth 20 -Compress
  $bytes = [Text.Encoding]::UTF8.GetBytes($msg)
  $client.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait()
  $buf = New-Object byte[] 1048576
  $ms = New-Object System.IO.MemoryStream
  $obj = $null
  while ($true) {
    $res = $client.ReceiveAsync([ArraySegment[byte]]::new($buf), $ct).Result
    $ms.Write($buf, 0, $res.Count)
    if ($res.EndOfMessage) {
      $text = [Text.Encoding]::UTF8.GetString($ms.ToArray())
      $ms.SetLength(0)
      $obj = $text | ConvertFrom-Json
      if ($obj.id -eq 1) { break }
    }
  }
  $client.Dispose()
  return $obj
}

function Invoke-CdpEval([string]$ws, [string]$expr) {
  $r = Invoke-Cdp $ws 'Runtime.evaluate' @{ expression = $expr; returnByValue = $true; awaitPromise = $true }
  if ($r.result.exceptionDetails) { return "EXCEPTION: " + $r.result.exceptionDetails.exception.description }
  return $r.result.result.value
}

function Save-CdpScreenshot([string]$ws, [string]$path) {
  $r = Invoke-Cdp $ws 'Page.captureScreenshot' @{ format = 'png' }
  [IO.File]::WriteAllBytes($path, [Convert]::FromBase64String($r.result.data))
}
