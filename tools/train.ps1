# Starts a GPU training run in the background, logging where the dashboard can see it.
# Usage: .\tools\train.ps1 -Name net1 -Data gen1 [-Superbatches 40] [-Wdl 0.4] [-Inputs 768|kb10|kb4]
#   -Data 'gen5,gen6' or -Data parts.txt (paths in list relative to list file).
#   -Inputs kb10 = 10 king buckets, mirrored (needs an engine built from the kb branch).
param(
    [Parameter(Mandatory)][string]$Name,
    [Parameter(Mandatory)][string]$Data,
    [int]$Superbatches = 40,
    [string]$Wdl = '0.4',   # constant, or 'start-end' to rise linearly over training
    [int]$Hidden = 512,
    [int]$Buckets = 1,
    [ValidateSet('768', 'kb10', 'kb4', 'ml', 'ml32', 'mlkb4', 'mlthr')][string]$Inputs = '768',
    [string]$Val = '',   # held-out bulletformat file (name in the training data folder, or a full path): reports validation loss
    [switch]$Build
)

# The driver supports CUDA 13.3, so pin the toolkit (compile-time) and its DLLs (run-time) to it.
$cuda = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3'
$env:CUDA_PATH = $cuda
$env:Path = "$cuda\bin\x64;$cuda\bin;" + [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User') + ";$env:USERPROFILE\.cargo\bin"

$trainer = "$PSScriptRoot\..\trainer"
if ($Build -or -not (Test-Path "$trainer\target\release\trainer.exe")) {
    Push-Location $trainer
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "Trainer build failed" }
    } finally { Pop-Location }
}

. "$PSScriptRoot\paths.ps1"
# paths.ps1 refreshes PATH; restore the selected CUDA DLL directories afterward.
$env:Path = "$cuda\bin\x64;$cuda\bin;$env:Path"
$Root = "$LabRoot\training"
$dir = "$Root\$Name"
if (Test-Path "$dir\log.txt") { throw "Training run '$Name' already exists" }
New-Item -ItemType Directory -Force $dir | Out-Null
# List-file entries are resolved by the trainer relative to the list file.
if ([IO.Path]::GetExtension($Data) -eq '.txt') {
    $dataPath = if (Test-Path -LiteralPath $Data) { (Resolve-Path -LiteralPath $Data).Path } else { "$Root\data\$Data" }
    if (-not (Test-Path -LiteralPath $dataPath)) { throw "No data list at $dataPath" }
} else {
    $dataPath = (($Data -split ',') | ForEach-Object {
        $part = $_.Trim()
        $file = if ($part.EndsWith('.bin')) { $part } else { "$part.bin" }
        $path = if ([IO.Path]::IsPathRooted($file)) { $file } else { "$Root\data\$file" }
        if (-not (Test-Path -LiteralPath $path)) { throw "No prepared data at $path (run prepare-data.ps1)" }
        $path
    }) -join ','
}

$extra = @()
if ($Val) {
    $valPath = if (Test-Path $Val) { $Val } else { "$Root\data\$Val.bin" }
    if (-not (Test-Path $valPath)) { throw "No validation data at $valPath" }
    $extra = @('"val={0}"' -f $valPath)
}
$p = Start-Process -FilePath "$trainer\target\release\trainer.exe" `
    -ArgumentList (@($Name, ('"{0}"' -f $dataPath), ('"{0}"' -f "$dir\checkpoints"), $Superbatches, $Wdl, $Hidden, $Buckets, $Inputs) + $extra) `
    -RedirectStandardOutput "$dir\log.txt" -RedirectStandardError "$dir\err.txt" -WindowStyle Hidden -PassThru
$p.PriorityClass = 'BelowNormal'
"Training '$Name' started (pid $($p.Id)) -> $dir"
