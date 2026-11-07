# Frozen external-pool match. External games remain under LabRoot/runs only.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9 .,+()/_-]{3,120}$')][string]$Name,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9._-]{1,60}$')][string]$Dev,
    [Parameter(Mandatory)][string]$DevRef,
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9._-]{1,60}$')][string]$Base,
    [Parameter(Mandatory)][string]$BaseRef,
    [ValidatePattern('^\d+(\.\d+)?\+\d+(\.\d+)?$')][string]$TC = '8+0.08',
    [ValidateRange(1,250)][int]$BlocksPerOpponentPerShard = 125,
    [ValidateRange(1,8)][int]$Jobs = 8,
    [ValidateSet('xengine-pool-v1')][string]$Pool = 'xengine-pool-v1',
    [ValidateRange(1,9007199254740991)][long]$Seed = 2026110701,
    [ValidatePattern('^[A-Za-z0-9._-]{1,60}\.(pgn|epd)$')][string]$Book = '8moves_v3.pgn',
    [string]$Notes = '',
    [long]$RunId = 0
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\paths.ps1"
Set-Location $RepoRoot
function Invoke-Gh([string[]]$Arguments) {
    $result = & gh @Arguments
    if ($LASTEXITCODE -ne 0) { throw 'GitHub command failed. Queue must stop for review.' }
    return $result
}
function Hash([string]$Path) { (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() }
$root = Join-Path $LabRoot 'cloud-xengine'
New-Item -ItemType Directory -Force $root | Out-Null
$id = $RunId
if (-not $id) {
    # Unique release assets are never clobbered, including the request itself.
    $key = 'xengine-' + [guid]::NewGuid().ToString('N')
    $tmp = Join-Path $root $key
    New-Item -ItemType Directory $tmp | Out-Null
    $poolFile = Join-Path $RepoRoot '.github\xengine\pool-v1.json'
    $locked = Get-Content $poolFile -Raw | ConvertFrom-Json
    if (@($locked.engines | Where-Object { -not $_.binary_sha256 }).Count) { throw 'Pin the complete pool lock before dispatching matches' }
    $active = Invoke-Gh -Arguments @('run','list','--workflow','xengine-match.yml','-L','30','--json','status') | ConvertFrom-Json
    if (@($active | Where-Object { $_.status -ne 'completed' }).Count) { throw 'An xengine run is already active' }
    $request = [ordered]@{ name=$Name; tc=$TC; jobs=$Jobs; blocks=$BlocksPerOpponentPerShard; pool=$Pool; pool_hash=(Hash $poolFile); seed=$Seed; notes=$Notes }
    foreach ($pair in @(@('candidate',$Dev,$DevRef),@('release',$Base,$BaseRef))) {
        $role,$spec,$ref = $pair
        $sha = (& git rev-parse --verify "$ref^{commit}").Trim()
        if ($LASTEXITCODE -ne 0 -or $sha -notmatch '^[a-f0-9]{40}$') { throw 'Cannot resolve code commit' }
        # Code must already be published. This driver never pushes unrelated refs.
        $null = Invoke-Gh -Arguments @('api',"repos/{owner}/{repo}/commits/$sha",'--jq','.sha')
        $entry = [ordered]@{ spec=$spec; ref=$sha }
        foreach ($kind in 'nnue','options','ref') {
            $asset = "$key-$role.$kind"
            $path = Join-Path $tmp $asset
            if ($kind -eq 'ref') { [IO.File]::WriteAllText($path,$sha) }
            else { Copy-Item -LiteralPath "$LabRoot\engines\$spec.$kind" -Destination $path }
            $field = if ($kind -eq 'ref') { 'ref_file' } else { $kind }
            $entry[$field] = @{ asset=$asset; sha256=(Hash $path) }
            $null = Invoke-Gh -Arguments @('release','upload','testkit',$path)
        }
        $request[$role] = $entry
    }
    $null = Invoke-Gh -Arguments @('release','download','testkit','--pattern',$Book,'--dir',$tmp)
    $request.book = @{ asset=$Book; sha256=(Hash (Join-Path $tmp $Book)) }
    $requestAsset = "$key.json"
    [IO.File]::WriteAllText((Join-Path $tmp $requestAsset),($request | ConvertTo-Json -Depth 10))
    $null = Invoke-Gh -Arguments @('release','upload','testkit',(Join-Path $tmp $requestAsset))
    $null = Invoke-Gh -Arguments @('workflow','run','xengine-match.yml','-f','mode=match','-f',"request=$requestAsset",'-f',"label=$key")
    for ($i=0; $i -lt 30 -and -not $id; $i++) {
        Start-Sleep 10
        $runs = Invoke-Gh -Arguments @('run','list','--workflow','xengine-match.yml','-L','20','--json','databaseId,displayTitle') | ConvertFrom-Json
        $id = ($runs | Where-Object { $_.displayTitle -eq "xengine match $key" } | Select-Object -First 1).databaseId
    }
    if (-not $id) { throw "Could not identify dispatch $key. Stop and reattach manually." }
} else {
    $tmp = Join-Path $root "run-$id"
    New-Item -ItemType Directory -Force $tmp | Out-Null
}
"cloud run $id started or reattached"
# Run-ID destination makes reattachment idempotent and visible to the dashboard.
$runDir = Join-Path $LabRoot "runs\$(Get-Date -Format yyyy-MM-dd)_xengine-$id"
New-Item -ItemType Directory -Force $runDir | Out-Null
$meta = @("name=$Name",'type=xengine',"main=$Dev","base=$Base","tc=$TC", "total=$($Jobs*$BlocksPerOpponentPerShard*16)",
    "created=$([DateTimeOffset]::Now.ToUnixTimeSeconds())", "run_id=$id", "pool=$Pool", "notes=$Notes")
if (-not (Test-Path "$runDir\meta.txt")) { $meta | Set-Content "$runDir\meta.txt" -Encoding utf8 }
$failed = $false
do {
    $rv = Invoke-Gh -Arguments @('run','view',"$id",'--json','status,conclusion,workflowName') | ConvertFrom-Json
    if ($rv.workflowName -ne 'xengine-match') { throw 'Run ID belongs to another workflow' }
    "$(Get-Date -Format o) cloud run $id $($rv.status)" | Add-Content "$runDir\progress.log"
    if ($rv.status -ne 'completed') { Start-Sleep 60 }
} while ($rv.status -ne 'completed')
# Always retain failure artifacts. Download only game shards, never engine binaries to this PC.
$artifactDir = Join-Path $tmp 'shards'
New-Item -ItemType Directory -Force $artifactDir | Out-Null
try { $null = Invoke-Gh -Arguments @('run','download',"$id",'--pattern','xengine-shard-*','--dir',$artifactDir) }
catch { $failed = $true }
if ($rv.conclusion -ne 'success' -or $failed) {
    @{ type='xengine'; valid=$false; decision='fail'; run_id=$id; error="GitHub conclusion $($rv.conclusion), artifacts downloaded=$(-not $failed)" } |
        ConvertTo-Json | Set-Content "$runDir\results.json" -Encoding utf8
    'Failed external check' | Set-Content "$runDir\output.txt"
    throw "cloud run $id failed. Artifacts retained for diagnosis. Stop dispatching."
}
$python = if (Get-Command py -ErrorAction SilentlyContinue) { 'py' } else { 'python3' }
& $python "$PSScriptRoot\xengine-analysis.py" --artifacts $artifactDir --output $runDir
if ($LASTEXITCODE -ne 0) { throw 'External analysis rejected the run' }
$result = Get-Content "$runDir\results.json" -Raw | ConvertFrom-Json
# Reattachment uses the archived request, never the new command line, for final provenance.
$req = $result.inputs
@("name=$($req.name)",'type=xengine',"main=$($req.candidate.spec)","base=$($req.release.spec)","tc=$($req.tc)",
  "total=$($result.games)","actual=$($result.games)","created=$([DateTimeOffset]::Now.ToUnixTimeSeconds())",
  "run_id=$id","pool=$($req.pool)","pool_hash=$($result.pool_hash)","schedule_hash=$($result.schedule_hash)","notes=$($req.notes)") |
  Set-Content "$runDir\meta.txt" -Encoding utf8
@("Elo: $($result.pool_elo); CI95: $($result.ci95 -join ', '); $($result.decision)","Games: $($result.games), complete paired external-pool games",'Finished match') |
    Set-Content "$runDir\output.txt" -Encoding utf8
Get-Content "$runDir\output.txt"
