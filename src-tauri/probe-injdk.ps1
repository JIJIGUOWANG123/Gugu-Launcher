$ProgressPreference = 'SilentlyContinue'
$t = Get-Date
try {
  $r = Invoke-WebRequest -Uri 'https://cdn.azul.com/zulu/bin/zulu21.54.107-ca-jdk21.0.13-win_x64.zip' -Method Head -TimeoutSec 30 -UseBasicParsing
  Write-Output ('HEAD ' + [int]$r.StatusCode + ' len=' + $r.Headers['Content-Length'] + ' ' + [math]::Round(((Get-Date)-$t).TotalMilliseconds) + 'ms')
} catch { Write-Output ('HEAD ERR ' + $_.Exception.Message) }
$t = Get-Date
try {
  $r2 = Invoke-WebRequest -Uri 'https://d10.injdk.cn/openjdk/oraclejdk/21/' -Method Head -TimeoutSec 30 -UseBasicParsing
  Write-Output ('injdk listing HEAD ' + [int]$r2.StatusCode + ' ' + [math]::Round(((Get-Date)-$t).TotalMilliseconds) + 'ms')
} catch { Write-Output ('ERR ' + $_.Exception.Message) }
