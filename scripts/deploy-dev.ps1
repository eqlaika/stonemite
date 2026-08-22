param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9A-Za-z.+-]+$')]
    [string]$BuildLabel,

    [Parameter(Mandatory = $true)]
    [string]$RepoPath
)

$ErrorActionPreference = 'Stop'
$stageRoot = Split-Path -Parent $PSScriptRoot

if (-not (Test-Path -LiteralPath (Join-Path $stageRoot 'Cargo.toml'))) {
    throw "Deployment staging directory is incomplete: $stageRoot"
}
if (-not (Test-Path -LiteralPath $RepoPath)) {
    throw "Remote Stonemite checkout does not exist: $RepoPath"
}

$deployMutex = [System.Threading.Mutex]::new($false, 'Local\StonemiteDevDeploy')
$hasDeployLock = $false
try {
    try {
        $hasDeployLock = $deployMutex.WaitOne(0)
    } catch [System.Threading.AbandonedMutexException] {
        $hasDeployLock = $true
    }
    if (-not $hasDeployLock) {
        throw 'Another Stonemite development deployment is already running on this host'
    }

    # Give every staged source a fresh timestamp and copy even apparently
    # identical files. ZIP timestamps are coarse enough that relying on
    # Robocopy's normal size/time comparison can retain stale source bytes.
    $mirrorTimestamp = [DateTime]::UtcNow
    Get-ChildItem -LiteralPath $stageRoot -Recurse -File | ForEach-Object {
        $_.LastWriteTimeUtc = $mirrorTimestamp
    }

    Write-Host "Mirroring current working tree to $RepoPath..."
    $robocopyArguments = @(
        $stageRoot,
        $RepoPath,
        '/MIR',
        '/IS',
        '/IT',
        '/R:2',
        '/W:1',
        '/FFT',
        '/NFL',
        '/NDL',
        '/NJH',
        '/NJS',
        '/NP',
        '/XJ',
        '/XD',
        '.git',
        'target',
        'dist',
        '.pi',
        'node_modules',
        '/XF',
        'AGENTS.md',
        '.envrc'
    )
    & robocopy.exe @robocopyArguments
    $robocopyExitCode = $LASTEXITCODE
    if ($robocopyExitCode -ge 8) {
        throw "Source mirror failed with robocopy exit code $robocopyExitCode"
    }

    # The frontend is built on the local development host because Node is not
    # installed on the Windows host. General source mirroring excludes dist folders
    # so release artifacts survive, then this one embedded dist is mirrored
    # explicitly.
    $settingsDistSource = Join-Path $stageRoot 'settings-ui\dist'
    $settingsDistDestination = Join-Path $RepoPath 'settings-ui\dist'
    if (-not (Test-Path -LiteralPath (Join-Path $settingsDistSource 'index.html'))) {
        throw 'The deployment archive does not contain a built settings frontend'
    }
    & robocopy.exe $settingsDistSource $settingsDistDestination /MIR /IS /IT /R:2 /W:1 /FFT /NFL /NDL /NJH /NJS /NP
    $settingsMirrorExitCode = $LASTEXITCODE
    if ($settingsMirrorExitCode -ge 8) {
        throw "Settings frontend mirror failed with robocopy exit code $settingsMirrorExitCode"
    }

    $buildTarget = Join-Path $RepoPath 'target\dev-build'
    $builtExecutable = Join-Path $buildTarget 'debug\stonemite.exe'
    $deployDirectory = Join-Path $RepoPath 'target\dev'
    $deployedExecutable = Join-Path $deployDirectory 'stonemite.exe'
    $pendingExecutable = Join-Path $deployDirectory 'stonemite.exe.new'

    Write-Host "Building $BuildLabel on the remote Windows host..."
    Push-Location $RepoPath
    try {
        $env:STONEMITE_BUILD_LABEL = $BuildLabel
        $env:CARGO_TARGET_DIR = $buildTarget
        & cargo build -p trusik
        if ($LASTEXITCODE -ne 0) {
            throw "trusik development build failed with exit code $LASTEXITCODE"
        }
        & cargo build -p stonemite
        if ($LASTEXITCODE -ne 0) {
            throw "Stonemite development build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Remove-Item Env:STONEMITE_BUILD_LABEL -ErrorAction SilentlyContinue
        Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
        Pop-Location
    }

    if (-not (Test-Path -LiteralPath $builtExecutable)) {
        throw "Development build did not produce $builtExecutable"
    }

    # Build away from the live executable so a failed compilation leaves the
    # currently running Stonemite instance untouched.
    $running = @(
        Get-CimInstance -ClassName Win32_Process -Filter "Name = 'stonemite.exe'" |
            Where-Object { $_.ExecutablePath }
    )
    $settingsProcessIds = @(
        $running |
            Where-Object { $_.CommandLine -match '(?i)(?:^|\s)--settings(?:\s|$)' } |
            ForEach-Object { [int]$_.ProcessId }
    )
    $trayProcesses = @(
        $running |
            Where-Object { $_.CommandLine -notmatch '(?i)(?:^|\s)--settings(?:\s|$)' }
    )
    if ($trayProcesses.Count -gt 1) {
        throw 'Multiple Stonemite tray processes are running; refusing an ambiguous replacement'
    }

    if ($trayProcesses.Count -eq 1) {
        $trayProcess = $trayProcesses[0]
        $trayProcessId = [int]$trayProcess.ProcessId
        Write-Host "Asking Stonemite tray process $trayProcessId to exit cleanly..."

        $quitTaskName = $null
        try {
            if ([int]$trayProcess.SessionId -eq 0) {
                # A session-0 process can receive the existing --quit helper
                # directly from this SSH session.
                [void](Start-Process -FilePath $trayProcess.ExecutablePath -ArgumentList '--quit' -Wait -PassThru)
            } else {
                # Window lookup is scoped to a Windows session. Run --quit
                # through an interactive task so it can find the tray window.
                $quitTaskName = 'Stonemite Development Quit'
                $quitTime = (Get-Date).AddMinutes(1).ToString('HH:mm')
                $quitAction = "`"$($trayProcess.ExecutablePath)`" --quit"
                & schtasks.exe /Create /TN $quitTaskName /TR $quitAction /SC ONCE /ST $quitTime /F /IT /RL LIMITED
                if ($LASTEXITCODE -ne 0) {
                    throw "Could not create the interactive quit task (exit $LASTEXITCODE)"
                }
                & schtasks.exe /Run /TN $quitTaskName
                if ($LASTEXITCODE -ne 0) {
                    throw "Could not start the interactive quit task (exit $LASTEXITCODE)"
                }
            }

            $quitDeadline = (Get-Date).AddSeconds(15)
            while ((Get-Process -Id $trayProcessId -ErrorAction SilentlyContinue) -and
                   (Get-Date) -lt $quitDeadline) {
                Start-Sleep -Milliseconds 100
            }
            if (Get-Process -Id $trayProcessId -ErrorAction SilentlyContinue) {
                throw "Stonemite tray process $trayProcessId did not exit cleanly; refusing to force-terminate it"
            }
        } finally {
            if ($quitTaskName) {
                & cmd.exe /D /C "schtasks.exe /Delete /TN `"$quitTaskName`" /F >NUL 2>&1"
            }
        }
    }

    # The settings window is an isolated same-executable subprocess and can
    # remain after the tray exits. Only close PIDs identified as settings
    # processes before shutdown; never force an unknown or newly launched tray.
    $settingsProcesses = @(
        $settingsProcessIds | ForEach-Object {
            Get-Process -Id $_ -ErrorAction SilentlyContinue
        }
    )
    foreach ($process in $settingsProcesses) {
        if ($process.MainWindowHandle -ne 0) {
            [void]$process.CloseMainWindow()
        }
    }

    $closeDeadline = (Get-Date).AddSeconds(5)
    do {
        $settingsProcesses = @(
            $settingsProcessIds | ForEach-Object {
                Get-Process -Id $_ -ErrorAction SilentlyContinue
            }
        )
        if ($settingsProcesses.Count -eq 0) {
            break
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $closeDeadline)

    if ($settingsProcesses.Count -gt 0) {
        Write-Warning 'Force-terminating remaining Stonemite settings processes.'
        $settingsProcesses | Stop-Process -Force
        $settingsProcesses | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue
    }

    $unexpectedProcesses = @(
        Get-CimInstance -ClassName Win32_Process -Filter "Name = 'stonemite.exe'"
    )
    if ($unexpectedProcesses.Count -gt 0) {
        $unexpectedIds = ($unexpectedProcesses | ForEach-Object { $_.ProcessId }) -join ', '
        throw "Stonemite process(es) appeared during replacement ($unexpectedIds); refusing to overwrite the deployed executable"
    }

    New-Item -ItemType Directory -Force -Path $deployDirectory | Out-Null
    Copy-Item -LiteralPath $builtExecutable -Destination $pendingExecutable -Force
    Remove-Item -LiteralPath $deployedExecutable -Force -ErrorAction SilentlyContinue
    Move-Item -LiteralPath $pendingExecutable -Destination $deployedExecutable

    # SSH runs in Windows session 0. Launch through an interactive scheduled
    # task so the tray application starts in the logged-in desktop session and
    # survives the SSH connection closing.
    Write-Host "Starting $deployedExecutable in the interactive Windows session..."
    $launchTaskName = 'Stonemite Development Launch'
    $launchTime = (Get-Date).AddMinutes(1).ToString('HH:mm')
    $taskAction = "`"$deployedExecutable`""
    $launched = $null
    try {
        & schtasks.exe /Create /TN $launchTaskName /TR $taskAction /SC ONCE /ST $launchTime /F /IT /RL LIMITED
        if ($LASTEXITCODE -ne 0) {
            throw "Could not create the interactive launch task (exit $LASTEXITCODE)"
        }
        & schtasks.exe /Run /TN $launchTaskName
        if ($LASTEXITCODE -ne 0) {
            throw "Could not start the interactive launch task (exit $LASTEXITCODE)"
        }

        $launchDeadline = (Get-Date).AddSeconds(10)
        do {
            $launched = Get-CimInstance -ClassName Win32_Process -Filter "Name = 'stonemite.exe'" |
                Where-Object { $_.ExecutablePath -eq $deployedExecutable } |
                Select-Object -First 1
            if ($launched) {
                break
            }
            Start-Sleep -Milliseconds 100
        } while ((Get-Date) -lt $launchDeadline)

        if (-not $launched) {
            throw 'The interactive launch task did not start Stonemite within 10 seconds'
        }
        Start-Sleep -Seconds 1
        if (-not (Get-Process -Id $launched.ProcessId -ErrorAction SilentlyContinue)) {
            throw 'Deployed Stonemite exited immediately after launch'
        }
    } finally {
        # cmd suppresses the expected error when a failed create left no task.
        & cmd.exe /D /C "schtasks.exe /Delete /TN `"$launchTaskName`" /F >NUL 2>&1"
    }

    Write-Host "Running Stonemite $BuildLabel (PID $($launched.ProcessId))."
} finally {
    if ($hasDeployLock) {
        [void]$deployMutex.ReleaseMutex()
    }
    $deployMutex.Dispose()
}
