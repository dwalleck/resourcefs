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
  [ordered]@{
    literal = $Literal
    environment = $environment
  } | ConvertTo-Json -Depth 5 -Compress | Set-Content -LiteralPath (Join-Path $Directory 'environment.json') -NoNewline
  while (-not (Test-Path -LiteralPath (Join-Path $Directory 'gate'))) {
    Start-Sleep -Milliseconds 10
  }
  $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $grandchild = Start-Process -FilePath $powershell -ArgumentList @(
    '-NoProfile',
    '-ExecutionPolicy', 'Bypass',
    '-File', $PSCommandPath,
    '-Mode', 'Grandchild',
    '-Directory', $Directory
  ) -PassThru
  Set-Content -LiteralPath (Join-Path $Directory 'spawned-grandchild.pid') -Value $grandchild.Id -NoNewline
  while ($true) {
    Add-Content -LiteralPath (Join-Path $Directory 'child.beat') -Value 'x' -NoNewline
    Start-Sleep -Milliseconds 20
  }
}

Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;

public sealed class ResourceFsJob : IDisposable {
    const UInt32 JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
    const Int32 JobObjectExtendedLimitInformation = 9;

    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        public Int64 PerProcessUserTimeLimit;
        public Int64 PerJobUserTimeLimit;
        public UInt32 LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public UInt32 ActiveProcessLimit;
        public UIntPtr Affinity;
        public UInt32 PriorityClass;
        public UInt32 SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct IO_COUNTERS {
        public UInt64 ReadOperationCount;
        public UInt64 WriteOperationCount;
        public UInt64 OtherOperationCount;
        public UInt64 ReadTransferCount;
        public UInt64 WriteTransferCount;
        public UInt64 OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
        public IO_COUNTERS IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr securityAttributes, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(IntPtr job, Int32 informationClass, IntPtr information, UInt32 length);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateJobObject(IntPtr job, UInt32 exitCode);

    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr handle);

    IntPtr handle;

    public ResourceFsJob() {
        handle = CreateJobObject(IntPtr.Zero, null);
        if (handle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        var limits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        int length = Marshal.SizeOf(limits);
        IntPtr pointer = Marshal.AllocHGlobal(length);
        try {
            Marshal.StructureToPtr(limits, pointer, false);
            if (!SetInformationJobObject(handle, JobObjectExtendedLimitInformation, pointer, (UInt32)length)) {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
        } finally {
            Marshal.FreeHGlobal(pointer);
        }
    }

    public void Assign(Process process) {
        if (!AssignProcessToJobObject(handle, process.Handle)) {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }

    public void Terminate() {
        if (!TerminateJobObject(handle, 137)) {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }
    }

    public void Dispose() {
        if (handle != IntPtr.Zero) {
            CloseHandle(handle);
            handle = IntPtr.Zero;
        }
    }
}
'@

$directory = Join-Path $env:TEMP ('resourcefs-job-probe-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $directory | Out-Null
$job = [ResourceFsJob]::new()
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
  $job.Assign($process)
  Set-Content -LiteralPath (Join-Path $directory 'gate') -Value 'go' -NoNewline

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
  $job.Terminate()
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
  $childAlive = $null -ne (Get-Process -Id $childPid -ErrorAction SilentlyContinue)
  $grandchildAlive = $null -ne (Get-Process -Id $grandchildPid -ErrorAction SilentlyContinue)
  $environmentNames = @($environment.environment.PSObject.Properties.Name | Sort-Object)
  [ordered]@{
    platform = [Environment]::OSVersion.VersionString
    argument = $environment.literal
    environmentNames = $environmentNames
    parentSecretPresent = $environmentNames -contains 'RFS_PARENT_SECRET_SENTINEL'
    childPid = $childPid
    grandchildPid = $grandchildPid
    childAlive = $childAlive
    grandchildAlive = $grandchildAlive
    heartbeatsBefore = $before
    heartbeatsAfter = $after
    heartbeatsStopped = ($before[0] -eq $after[0] -and $before[1] -eq $after[1])
    elapsedMs = $started.ElapsedMilliseconds
  } | ConvertTo-Json -Depth 5 -Compress
} finally {
  $job.Dispose()
  Remove-Item -LiteralPath $directory -Recurse -Force -ErrorAction SilentlyContinue
}
