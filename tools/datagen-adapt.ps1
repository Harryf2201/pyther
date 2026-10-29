# Keeps local self-play data generation sized to what else is running: few threads while
# tournaments play (they need the CPU and the heat budget), many when the CPU is free.
# Restarts are cheap: datagen resumes its counts and files exactly where it stopped.
# The watchdog still pauses everything if temperatures get too high.
# Usage (hidden, background): .\tools\datagen-adapt.ps1 [-Busy 4] [-Idle 14]
param(
    [int]$Busy = 4,
    [int]$Idle = 14,
    [string]$Name = 'gen3-v13-5k',
    [string]$Exe = 'pawnstorm-datagen-gen3',
    [int64]$Target = 1000000000,
    # Extra datagen flags, e.g. the real-opening starts: --openings <file> --opening-percent 50 --deny <file>
    [string[]]$Extra = @()
)
# (-File passes '-Extra a,b,c' as one string: split it.)
$Extra = @($Extra | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
. "$PSScriptRoot\paths.ps1"
if ($Name -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$' -or $Name -in '.', '..') { throw 'Name must be one lab directory name' }
if ($Exe -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') { throw 'Exe must be an engine basename, without .exe' }
$dir = "$LabRoot\datagen\$Name"
$exePath = [IO.Path]::GetFullPath("$LabRoot\engines\$Exe.exe")
$outputPath = [IO.Path]::GetFullPath($dir).TrimEnd('\')
$log = "$LabRoot\datagen\adapt.log"
function Log($m) { Add-Content $log "$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') $m" -Encoding utf8 }

function Test-OwnedGenerator($process) {
    # Both the executable AND datagen's positional output directory must match.
    # Do not infer ownership from a shared pawnstorm-datagen* process name.
    if (-not $process.ExecutablePath -or -not $process.CommandLine -or
        -not [string]::Equals($process.ExecutablePath, $exePath, [StringComparison]::OrdinalIgnoreCase)) { return $false }
    $match = [regex]::Match($process.CommandLine, '^\s*(?:"[^"]+"|\S+)\s+datagen\s+\d+\s+\d+\s+\d+\s+(?:"(?<out>[^"]+)"|(?<out>\S+))(?=\s|$)')
    if (-not $match.Success) { return $false }
    try { $out = [IO.Path]::GetFullPath($match.Groups['out'].Value).TrimEnd('\') } catch { return $false }
    return [string]::Equals($out, $outputPath, [StringComparison]::OrdinalIgnoreCase)
}
function Get-OwnedGenerators {
    Get-CimInstance Win32_Process -Filter "Name='$Exe.exe'" | Where-Object { Test-OwnedGenerator $_ }
}

function Start-Datagen($threads) {
    foreach ($owned in @(Get-OwnedGenerators)) {
        # Recheck the PID and creation time before stopping it (PID reuse).
        $live = Get-CimInstance Win32_Process -Filter "ProcessId=$($owned.ProcessId)"
        if ($live -and $live.CreationDate -eq $owned.CreationDate -and (Test-OwnedGenerator $live)) {
            Stop-Process -Id $live.ProcessId -Force -ErrorAction Stop
        }
    }
    Start-Sleep 2
    New-Item -ItemType Directory -Force $dir | Out-Null
    $p = Start-Process $exePath -ArgumentList (@('datagen', $threads, $Target, 5000, ('"{0}"' -f $dir)) + $Extra) `
        -RedirectStandardOutput "$dir\stdout.txt" -RedirectStandardError "$dir\stderr.txt" -WindowStyle Hidden -PassThru
    Start-Sleep 1
    $p.PriorityClass = 'BelowNormal'
    Log "datagen restarted with $threads threads"
}

Log 'datagen-adapt started'
$idleSince = $null
while ($true) {
    Set-Content "$LabRoot\datagen\adapt.heartbeat" (Get-Date -Format o)
    # (PowerShell names are case-insensitive, so this must not be called $busy: it would overwrite $Busy.)
    # Includes SPSA's fastchess, not just match.ps1 tournaments. GPU loaders also count.
    $tournament = [bool](Get-Process fastchess*, trainer -ErrorAction SilentlyContinue)
    $current = 0
    if (Test-Path "$dir\status.txt") { $current = [int](((Get-Content "$dir\status.txt" | Select-String '^threads=') -replace 'threads=', '')) }
    $running = [bool](@(Get-OwnedGenerators).Count)
    if ($tournament) {
        $idleSince = $null
        if (-not $running -or $current -ne $Busy) { Start-Datagen $Busy }
    } else {
        # Wait 2 minutes of no tournaments before scaling up (gauntlets often start back to back).
        if (-not $idleSince) { $idleSince = Get-Date }
        if (((Get-Date) - $idleSince).TotalSeconds -ge 120 -and (-not $running -or $current -ne $Idle)) { Start-Datagen $Idle }
    }
    Start-Sleep 30
}

