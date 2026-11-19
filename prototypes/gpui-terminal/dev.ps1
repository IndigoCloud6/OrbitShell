$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
$target = Join-Path $PSScriptRoot 'target'
$exeName = 'orbitshell-gpui-terminal.exe'
$buildExe = Join-Path $target "debug/$exeName"
$work = Join-Path $repo 'work/gpui-dev'
$slots = @('a', 'b' | ForEach-Object { Join-Path $work "$_/$exeName" })
$watchedFiles = @(
    $manifest
    (Join-Path $PSScriptRoot 'Cargo.lock')
    (Join-Path $PSScriptRoot 'build.rs')
    (Join-Path $repo 'crates/orbitshell-ssh/Cargo.toml')
    (Join-Path $repo 'assets/orbitshell.svg')
)
$watchedDirectories = @(
    (Join-Path $PSScriptRoot 'src')
    (Join-Path $PSScriptRoot 'assets')
    (Join-Path $repo 'crates/orbitshell-ssh/src')
)

function Get-SourceStamp {
    $files = foreach ($path in $watchedFiles) {
        Get-Item -LiteralPath $path -ErrorAction SilentlyContinue
    }
    $files += foreach ($path in $watchedDirectories) {
        Get-ChildItem -LiteralPath $path -File -Recurse -ErrorAction SilentlyContinue
    }
    ($files | Sort-Object FullName | ForEach-Object {
        '{0}|{1}|{2}' -f $_.FullName, $_.Length, $_.LastWriteTimeUtc.Ticks
    }) -join "`n"
}

function Stop-Workbench([System.Diagnostics.Process]$process, [string]$expectedPath) {
    if ($process -and -not $process.HasExited) {
        if ($process.Path -ne $expectedPath) {
            throw "Refusing to close a different executable: $($process.Id)"
        }
        $null = $process.CloseMainWindow()
        if (-not $process.WaitForExit(4000)) {
            Stop-Process -InputObject $process
            if (-not $process.WaitForExit(4000)) { throw 'The previous GPUI process did not exit.' }
        }
    }
}

function Start-Workbench([string]$path) {
    $start = [System.Diagnostics.ProcessStartInfo]::new($path)
    $start.WorkingDirectory = $repo
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $process = [System.Diagnostics.Process]::Start($start)
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        if ($process.HasExited) { throw "GPUI exited on startup with code $($process.ExitCode)." }
        $process.Refresh()
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and $process.MainWindowTitle) {
            Write-Host "Opened GPUI window (PID $($process.Id))."
            return $process
        }
        Start-Sleep -Milliseconds 200
    }
    Stop-Workbench $process $path
    throw 'GPUI did not open a visible window.'
}

$null = New-Item -ItemType Directory -Force -Path $work
try {
    $lock = [System.IO.File]::Open((Join-Path $work 'watcher.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
} catch [System.IO.IOException] {
    throw "Cannot acquire the GPUI dev lock in $work. Another watcher may already be running."
}

$window = $null
$lastWorkingExe = $null
try {
    foreach ($path in $slots) {
        $null = New-Item -ItemType Directory -Force -Path (Split-Path -Parent $path)
    }
    $expectedPaths = @($buildExe) + $slots
    foreach ($previous in Get-CimInstance -ClassName Win32_Process -Filter "Name = '$exeName'") {
        $path = $previous.ExecutablePath
        if ($path -notin $expectedPaths) { continue }
        $quotedPath = '"' + $path + '"'
        if ($previous.CommandLine -ne $quotedPath -and $previous.CommandLine -ne $path) {
            throw "The GPUI process at $path was launched with a different command line. Close it first."
        }
        $process = Get-Process -Id $previous.ProcessId -ErrorAction SilentlyContinue
        Stop-Workbench $process $path
    }
    if (Test-Path -LiteralPath $buildExe -PathType Leaf) {
        Copy-Item -LiteralPath $buildExe -Destination $slots[0] -Force
        $lastWorkingExe = $slots[0]
    }

    Set-Location -LiteralPath $repo
    Write-Host "Watching GPUI source files and assets in $repo."
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $observedStamp = Get-SourceStamp
    $attemptedStamp = $null
    $stableSince = -400
    while (-not $window -or -not $window.HasExited) {
        $stamp = Get-SourceStamp
        if ($stamp -cne $observedStamp) {
            $observedStamp = $stamp
            $stableSince = $clock.ElapsedMilliseconds
        }
        if ($attemptedStamp -cne $observedStamp -and $clock.ElapsedMilliseconds -ge $stableSince + 400) {
            $attemptedStamp = $observedStamp
            Write-Host 'Building GPUI workbench...'
            & cargo build --locked --manifest-path $manifest --target-dir $target
            if ($window -and $window.HasExited) { break }
            if ($LASTEXITCODE -ne 0) {
                Write-Warning 'Build failed. Watching for the next source change.'
                if (-not $window -and $lastWorkingExe) {
                    $window = Start-Workbench $lastWorkingExe
                }
            } elseif ((Get-SourceStamp) -ceq $attemptedStamp) {
                $nextExe = if ($lastWorkingExe -eq $slots[0]) { $slots[1] } else { $slots[0] }
                Copy-Item -LiteralPath $buildExe -Destination $nextExe -Force
                Stop-Workbench $window $lastWorkingExe
                $window = $null
                try {
                    $window = Start-Workbench $nextExe
                    $lastWorkingExe = $nextExe
                } catch {
                    Write-Warning "New window failed to start. $($_.Exception.Message)"
                    if ($lastWorkingExe) { $window = Start-Workbench $lastWorkingExe }
                }
            }
        }
        Start-Sleep -Milliseconds 250
    }
    Write-Host 'GPUI window closed. Stopping the source watcher.'
} finally {
    try {
        Stop-Workbench $window $lastWorkingExe
    } finally {
        $lock.Dispose()
    }
}
