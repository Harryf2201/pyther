# Starts one batch of cloud data generation on GitHub Actions (see .github/workflows/datagen.yml).
# Refuses to start if a batch is already running or queued, so only one batch is ever in flight.
# Usage: .\tools\cloud-datagen.ps1 -Ref gen8-datagen -Jobs 14 -Minutes 330 -SoftNodes 5000 -HardNodes 100000 -Eval nnue -Chain 0
# -Jobs 0 = auto: the workflow picks 20 when no test match is running or queued, else 14 (6 runners stay free for a test).
param(
    [ValidateRange(0, 20)][int]$Jobs = 0,
    [ValidateRange(10, 340)][int]$Minutes = 330,
    [int]$SoftNodes = 5000,
    [ValidateRange(1, 2000000000)][int]$HardNodes = 100000,
    [ValidatePattern('^[A-Za-z0-9._/-]+$')][string]$Ref = 'gen8-datagen',
    # Follow-up batches GitHub starts by itself after this one (e.g. -Chain 1 = two batches back to back).
    [ValidateRange(0, 3)][int]$Chain = 0,
    # This generation branch embeds big7 (1024); nnue produces gen8, classical is a diagnostic fallback.
    [ValidateSet('classical', 'nnue')][string]$Eval = 'nnue'
)
$ErrorActionPreference = 'Stop'
if ($SoftNodes -le 0 -or $HardNodes -lt $SoftNodes) { throw 'Require 0 < SoftNodes <= HardNodes' }
. "$PSScriptRoot\paths.ps1"
if (Test-Path "$LabRoot\datagen\cloud-STOPPED.txt") {
    "GitHub reported a problem earlier, so cloud batches are on hold:"
    Get-Content "$LabRoot\datagen\cloud-STOPPED.txt"
    "Delete $LabRoot\datagen\cloud-STOPPED.txt once it's resolved."
    return
}
Push-Location $RepoRoot
try {
    # Parentheses make PowerShell 5 enumerate the JSON array instead of passing it as one object.
    $active = (gh run list --workflow datagen.yml --limit 20 --json databaseId,status | ConvertFrom-Json) |
        Where-Object { $_.status -in 'queued', 'in_progress', 'waiting', 'pending', 'requested' }
    if ($active) {
        "A cloud batch is already in progress (run $($active[0].databaseId)). Not starting another."
        return
    }
    $before = @(gh run list --workflow datagen.yml --limit 1 --json databaseId | ConvertFrom-Json).databaseId
    # Pin dispatch to the resolved commit; a moved branch must not silently change the recipe.
    $commit = gh api "repos/{owner}/{repo}/commits/$Ref" --jq '.sha'
    if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[0-9a-f]{40}$') { throw 'Cannot resolve remote Ref' }
    $beforeTime = [DateTimeOffset]::UtcNow
    gh workflow run datagen.yml --ref $Ref -f source_commit=$commit -f jobs=$(if ($Jobs) { $Jobs } else { 'auto' }) -f minutes=$Minutes -f soft_nodes=$SoftNodes -f hard_nodes=$HardNodes -f chain=$Chain -f eval=$Eval
    if ($LASTEXITCODE -ne 0) { throw "GitHub refused to start the workflow (see the message above)." }
    # Wait for the new run to appear.
    $run = $null
    for ($i = 0; $i -lt 20 -and -not $run; $i++) {
        Start-Sleep 3
        $latest = @(gh run list --workflow datagen.yml --limit 10 --json databaseId,url,headSha,event,createdAt | ConvertFrom-Json) |
            Where-Object { $_.databaseId -notin $before -and $_.headSha -eq $commit -and $_.event -eq 'workflow_dispatch' -and [DateTimeOffset]::Parse($_.createdAt) -ge $beforeTime.AddSeconds(-5) } |
            Sort-Object databaseId -Descending | Select-Object -First 1
        if ($latest) { $run = $latest }
    }
    if (-not $run) { throw "Started the workflow but couldn't find its run yet; check the Actions tab." }
    $dir = "$LabRoot\datagen\cloud-$($run.databaseId)"
    New-Item -ItemType Directory -Force $dir | Out-Null
    # Per-shard status/artifact provenance contains authoritative net SHA256 and executable SHA256.
    [IO.File]::WriteAllLines("$dir\launch.txt", [string[]]@("jobs=$Jobs", "minutes=$Minutes", "soft_nodes=$SoftNodes", "hard_nodes=$HardNodes", "eval=$Eval", "chain=$Chain", "ref=$Ref", "commit=$commit", "gen=$(if ($Eval -eq 'nnue') { 8 } else { 1 })", "nnue_width=1024", "source_tag=gen8o-big7-cloud-n$SoftNodes"))
    "Started cloud batch: $Jobs jobs x $Minutes min$(if ($Chain) { ", then $Chain more batch(es) back to back" }) -> $($run.url)"
    "The dashboard shows it once tools\cloud-sync.ps1 is running (it downloads data as jobs finish)."
} finally {
    Pop-Location
}
