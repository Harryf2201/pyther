# CPU-only smoke/parity check. Never starts the adapter or touches existing jobs.
param([string]$Candidate, [string]$Reference, [string]$Net, [string]$Output)
$ErrorActionPreference = 'Stop'
$parent = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$lab = Join-Path $parent 'testing'
if (-not $Candidate) { $Candidate = "$lab\engines\pawnstorm-datagen-gen8k.exe" }
if (-not $Reference) { $Reference = "$lab\engines\pawnstorm-xsearch5.exe" }
if (-not $Net) { $Net = "$lab\engines\x25-big7.nnue" }
if (-not $Output) { $Output = "$lab\coord\gen8-verify-$([guid]::NewGuid().ToString('N'))" }
if (Test-Path -LiteralPath $Output) { throw 'Verification output must be a new directory' }
New-Item -ItemType Directory -Path $Output | Out-Null
function Check-Health {
    if ((Get-Content "$lab\health\control.txt" -Raw) -match '(?i)pause') { throw 'Health control requests pause; rerun later with a new output' }
}
function Start-CheckedProcess($exe, $arguments) {
    Check-Health
    $si = New-Object Diagnostics.ProcessStartInfo
    $si.FileName = $exe
    $si.Arguments = $arguments
    $si.UseShellExecute = $false
    $si.CreateNoWindow = $true
    $si.RedirectStandardInput = $true
    $si.RedirectStandardOutput = $true
    $si.RedirectStandardError = $true
    $p = New-Object Diagnostics.Process
    $p.StartInfo = $si
    [void]$p.Start()
    $p.PriorityClass = 'BelowNormal'
    return $p
}
function Search($exe, $position, $externalNet) {
    $p = Start-CheckedProcess $exe ''
    $lines = New-Object 'Collections.Generic.List[string]'
    try {
        $commands = @('uci', 'setoption name Threads value 1', 'setoption name Hash value 16')
        if ($externalNet) { $commands += "setoption name EvalFile value $externalNet" }
        $commands += @('ucinewgame', "position $position", 'eval', 'go nodes 20000')
        foreach ($line in $commands) { $p.StandardInput.WriteLine($line) }
        while ($true) {
            $read = $p.StandardOutput.ReadLineAsync()
            if (-not $read.Wait(30000)) { throw 'UCI response timeout' }
            $line = $read.Result
            if ($null -eq $line) { throw 'Engine exited before bestmove' }
            $lines.Add($line)
            if ($line -match '^bestmove ') { break }
        }
        $p.StandardInput.WriteLine('quit')
        if (-not $p.WaitForExit(10000)) { throw 'Engine did not exit' }
        $err = $p.StandardError.ReadToEnd()
        if ($p.ExitCode -ne 0 -or $err) { throw "Engine error: $err" }
        $nnue = @($lines | Where-Object { $_ -match '^nnue:' })[-1]
        $info = @($lines | Where-Object { $_ -match '^info depth ' })[-1]
        return [pscustomobject]@{ Eval=$nnue; Info=$info; Bestmove=$line; Lines=@($lines) }
    } finally {
        # Only this verifier's newly created process may be terminated on failure.
        if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }
        $p.Dispose()
    }
}
$encoding = [Text.Encoding]::GetEncoding(28591)
$netBytes = [IO.File]::ReadAllBytes($Net)
if ($encoding.GetString([IO.File]::ReadAllBytes($Candidate)).IndexOf($encoding.GetString($netBytes), [StringComparison]::Ordinal) -lt 0) { throw 'Exact teacher bytes not embedded' }
$positions = @('startpos', 'startpos moves e2e4 c7c5 g1f3', 'fen r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1', 'fen 8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1')
$rows = @()
for ($i=0; $i -lt $positions.Count; $i++) {
    $a = Search $Candidate $positions[$i] ''
    $b = Search $Reference $positions[$i] $Net
    $a.Lines | Set-Content "$Output\candidate-$i.log"
    $b.Lines | Set-Content "$Output\reference-$i.log"
    $normalise = { param($s) (($s -replace ' time \d+', '') -replace ' nps \d+', '') }
    if ($a.Eval -ne $b.Eval) { throw "Static evaluation differs on position $i" }
    $aIterations = @($a.Lines | Where-Object { $_ -match '^info depth ' } | ForEach-Object { & $normalise $_ }) -join "`n"
    $bIterations = @($b.Lines | Where-Object { $_ -match '^info depth ' } | ForEach-Object { & $normalise $_ }) -join "`n"
    $rows += [pscustomobject]@{ Position=$positions[$i]; Eval=$a.Eval; Candidate=$a.Info; Reference=$b.Info; SearchEqual=($aIterations -eq $bIterations); BestmoveEqual=($a.Bestmove -eq $b.Bestmove) }
}
# Two one-game runs, using a small copied subset of the real attack seeds.
$seedFile = "$Output\attack-seeds.fen"
Get-Content "$lab\openings\attack-seeds.fen" -TotalCount 20 | Set-Content $seedFile -Encoding ascii
for ($i=0; $i -lt 2; $i++) {
    $dir = "$Output\game-$i"
    $deny = "$parent\gen8-datagen\openings\testbook-starts.fen"
    $arguments = 'datagen 1 1 5000 "{0}" 30 {1} --hard-nodes 100000 --fens "{2}" --fen-percent 100 --deny "{3}"' -f $dir, (8100+$i), $seedFile, $deny
    $p = Start-CheckedProcess $Candidate $arguments
    try {
        $stdout = $p.StandardOutput.ReadToEndAsync()
        $stderr = $p.StandardError.ReadToEndAsync()
        if (-not $p.WaitForExit(60000)) { throw 'Tiny datagen timeout' }
        $stdout.Result | Set-Content "$Output\datagen-$i.log"
        if ($p.ExitCode -ne 0 -or $stderr.Result) { throw "Datagen failed: $($stderr.Result)" }
    } finally { if (-not $p.HasExited) { $p.Kill(); $p.WaitForExit() }; $p.Dispose() }
    $records = @(Get-Content "$dir\data_00.txt")
    if (-not $records.Count -or @($records | Where-Object { $_ -notmatch '^[^|]+ [wb] [^|]+ \| -?\d+ \| (0\.0|0\.5|1\.0)$' }).Count) { throw 'Invalid datagen records' }
    $utils = "$parent\bullet\target\release\bullet-utils.exe"
    Check-Health
    $convert = Start-CheckedProcess $utils ('convert --from text --input "{0}" --output "{1}"' -f "$dir\data_00.txt", "$dir\validated.bin")
    try {
        $result = $convert.StandardOutput.ReadToEnd(); $err = $convert.StandardError.ReadToEnd()
        $convert.WaitForExit()
        $result | Set-Content "$Output\convert-$i.log"
        if ($convert.ExitCode -ne 0 -or $result -match 'error parsing' -or $err -or (Get-Item "$dir\validated.bin").Length -ne 32*$records.Count) { throw 'Bullet record validation failed' }
    } finally { $convert.Dispose() }
}
$result = [pscustomobject]@{ NetSHA256=(Get-FileHash $Net).Hash; ExeSHA256=(Get-FileHash $Candidate).Hash; Searches=$rows; Output=$Output }
$result | ConvertTo-Json -Depth 6 | Set-Content "$Output\summary.json"
$result | ConvertTo-Json -Depth 6
