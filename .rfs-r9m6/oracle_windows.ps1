param(
  [ValidateSet('Parent', 'Child', 'Grandchild')]
  [string]$Mode = 'Parent',
  [string]$Directory = '',
  [string]$Literal = ''
)

$ErrorActionPreference = 'Stop'

if ($Mode -eq 'Grandchild') {
  Set-Content -LiteralPath (Join-Path $Directory 'grandchild.pid') -Value $PID -NoNewline
  while ($true) {
    Add-Content -LiteralPath (Join-Path $Directory 'grandchild.beat') -Value 'x' -NoNewline
    Start-Sleep -Milliseconds 20
  }
}

if ($Mode -eq 'Child') {
  Set-Content -LiteralPath (Join-Path $Directory 'child.pid') -Value $PID -NoNewline
  $environment = [ordered]@{}
  Get-ChildItem Env: | Sort-Object Name | ForEach-Object { $environment[$_.Name] = $_.Value }
  [ordered]@{ literal = $Literal; environment = $environment } |
    ConvertTo-Json -Depth 5 -Compress |
    Set-Content -LiteralPath (Join-Path $Directory 'environment.json') -NoNewline
  $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $grandchild = Start-Process -FilePath $powershell -ArgumentList @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath,
    '-Mode', 'Grandchild', '-Directory', $Directory
  ) -PassThru
  while ($true) {
    Add-Content -LiteralPath (Join-Path $Directory 'child.beat') -Value 'x' -NoNewline
    Start-Sleep -Milliseconds 20
  }
}

$directory = Join-Path $env:TEMP ('resourcefs-job-oracle-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory | Out-Null
try {
  $powershellDirectory = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0'
  $start = [Diagnostics.ProcessStartInfo]::new()
  $start.FileName = 'powershell.exe'
  $start.Arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -Mode Child -Directory "' + $directory + '" -Literal "literal;$HOME&|<>()"'
  $start.UseShellExecute = $false
  $start.CreateNoWindow = $true
  $start.EnvironmentVariables.Clear()
  $start.EnvironmentVariables['PATH'] = $powershellDirectory
  $start.EnvironmentVariables['SystemRoot'] = $env:SystemRoot
  $start.EnvironmentVariables['RFS_LITERAL'] = 'value with spaces;$HOME&|'
  $process = [Diagnostics.Process]::Start($start)

  $deadline = [DateTime]::UtcNow.AddSeconds(10)
  $required = @('child.pid', 'grandchild.pid', 'environment.json', 'child.beat', 'grandchild.beat')
  while ([DateTime]::UtcNow -lt $deadline) {
    if (($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $directory $_)) }).Count -eq 0) { break }
    Start-Sleep -Milliseconds 20
  }
  foreach ($name in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $directory $name))) { throw "fixture did not create $name" }
  }

  $childPid = [int](Get-Content -LiteralPath (Join-Path $directory 'child.pid') -Raw)
  $grandchildPid = [int](Get-Content -LiteralPath (Join-Path $directory 'grandchild.pid') -Raw)
  $environment = Get-Content -LiteralPath (Join-Path $directory 'environment.json') -Raw | ConvertFrom-Json
  $started = [Diagnostics.Stopwatch]::StartNew()
  Stop-Process -Id $grandchildPid -Force -ErrorAction SilentlyContinue
  Stop-Process -Id $childPid -Force -ErrorAction SilentlyContinue
  $process.WaitForExit(1000) | Out-Null
  $exitDeadline = [DateTime]::UtcNow.AddSeconds(1)
  while ([DateTime]::UtcNow -lt $exitDeadline) {
    $childAlive = $null -ne (Get-Process -Id $childPid -ErrorAction SilentlyContinue)
    $grandchildAlive = $null -ne (Get-Process -Id $grandchildPid -ErrorAction SilentlyContinue)
    if (-not $childAlive -and -not $grandchildAlive) { break }
    Start-Sleep -Milliseconds 20
  }
  $before = @(
    (Get-Item -LiteralPath (Join-Path $directory 'child.beat')).Length,
    (Get-Item -LiteralPath (Join-Path $directory 'grandchild.beat')).Length
  )
  Start-Sleep -Milliseconds 300
  $started.Stop()
  $after = @(
    (Get-Item -LiteralPath (Join-Path $directory 'child.beat')).Length,
    (Get-Item -LiteralPath (Join-Path $directory 'grandchild.beat')).Length
  )
  $environmentNames = @($environment.environment.PSObject.Properties.Name | Sort-Object)
  [ordered]@{
    platform = [Environment]::OSVersion.VersionString
    mechanism = 'explicit child/grandchild PID termination'
    argument = $environment.literal
    environmentNames = $environmentNames
    parentSecretPresent = $environmentNames -contains 'RFS_PARENT_SECRET_SENTINEL'
    childAlive = $null -ne (Get-Process -Id $childPid -ErrorAction SilentlyContinue)
    grandchildAlive = $null -ne (Get-Process -Id $grandchildPid -ErrorAction SilentlyContinue)
    heartbeatsBefore = $before
    heartbeatsAfter = $after
    heartbeatsStopped = ($before[0] -eq $after[0] -and $before[1] -eq $after[1])
    elapsedMs = $started.ElapsedMilliseconds
  } | ConvertTo-Json -Depth 5 -Compress
} finally {
  Stop-Process -Id $grandchildPid -Force -ErrorAction SilentlyContinue
  Stop-Process -Id $childPid -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $directory -Recurse -Force -ErrorAction SilentlyContinue
}
