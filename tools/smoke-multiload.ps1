# Short GPU smoke test; creates only disposable artifacts under trainer/target.
param([string]$Validation = "$PSScriptRoot\..\..\testing\training\data\val-gen6.bin")
$ErrorActionPreference = 'Stop'
$trainer = [IO.Path]::GetFullPath("$PSScriptRoot\..\trainer")
$run = Join-Path $trainer ("target\multiload-smoke-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force $run | Out-Null
$source = [IO.File]::OpenRead((Resolve-Path -LiteralPath $Validation).Path)
try {
    foreach ($part in @('a', 'b')) {
        $bytes = New-Object byte[] (32 * 65536)
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $n = $source.Read($bytes, $offset, $bytes.Length - $offset)
            if ($n -eq 0) { throw 'Validation input too short' }
            $offset += $n
        }
        [IO.File]::WriteAllBytes("$run\$part.bin", $bytes)
    }
} finally { $source.Dispose() }
@('a.bin', 'b.bin') | Set-Content "$run\parts.txt"
$env:CUDA_PATH = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3'
$env:Path = "$env:CUDA_PATH\bin\x64;$env:CUDA_PATH\bin;$env:USERPROFILE\.cargo\bin;$env:Path"
$arguments = @('multiload-smoke', ('"{0}"' -f "$run\parts.txt"), ('"{0}"' -f "$run\checkpoints"),
    1, '0.55', 128, 8, 'ml', 'batch-size=1024', 'batches=1024', ('"val={0}"' -f (Resolve-Path -LiteralPath $Validation).Path))
$p = Start-Process "$trainer\target\release\trainer.exe" -ArgumentList $arguments -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput "$run\stdout.log" -RedirectStandardError "$run\stderr.log"
$p.PriorityClass = 'BelowNormal'
$null = $p.Handle # Retain the process handle so PowerShell 5.1 preserves ExitCode.
$p.WaitForExit()
$p.Refresh()
if ($p.ExitCode -ne 0) { throw "Trainer failed ($($p.ExitCode)); see $run" }
$stdout = Get-Content "$run\stdout.log" -Raw
if ($stdout -notmatch 'Training on NVIDIA') { throw 'GPU training not confirmed' }
$losses = @(Get-Content "$run\checkpoints\multiload-smoke-1\log.txt" | ForEach-Object { [double](($_ -split ',')[2]) })
if ($losses[-1] -ge $losses[0]) { throw "Loss did not decrease: $($losses[0]) -> $($losses[-1]); see $run" }
"GPU smoke passed: loss $($losses[0]) -> $($losses[-1]); logs: $run"

