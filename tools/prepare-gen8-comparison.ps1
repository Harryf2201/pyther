# Produce an exact-size replacement mix from immutable, shuffled 32-byte Bullet parts.
# Control reuses BaseList; candidate samples the old parts and adds fresh own-engine data.
param(
    [Parameter(Mandatory)][string]$BaseList,
    [Parameter(Mandatory)][string[]]$Fresh,
    [Parameter(Mandatory)][string]$Output,
    [ValidateRange(0.001,0.9)][double]$Fraction=0.1,
    [int]$Seed=8100
)
$ErrorActionPreference='Stop'
[Diagnostics.Process]::GetCurrentProcess().PriorityClass='BelowNormal'
$lab=Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) 'testing'
function Check-Health {
    if ((Get-Content "$lab\health\control.txt" -Raw) -match '(?i)pause') { throw 'Health requests pause; leave output for inspection and use a new output on retry' }
}
Check-Health
if (Test-Path -LiteralPath $Output) { throw 'Output must be a new directory' }
$BaseList=(Resolve-Path -LiteralPath $BaseList).Path
$baseDir=Split-Path $BaseList -Parent
$old=@(Get-Content -LiteralPath $BaseList | ForEach-Object {
    $line=$_.Trim()
    if ($line -and -not $line.StartsWith('#')) {
        if ($line -match '\s+[0-9.]+$') { throw 'Use an unweighted base list' }
        if (-not [IO.Path]::IsPathRooted($line)) { $line=Join-Path $baseDir $line }
        (Resolve-Path -LiteralPath $line).Path
    }
})
$freshPaths=@($Fresh | ForEach-Object { (Resolve-Path -LiteralPath $_).Path })
if (-not $old.Count -or -not $freshPaths.Count) { throw 'Empty data input' }
$oldCount=[long]0; $freshCount=[long]0
foreach($file in $old+$freshPaths) {
    $len=(Get-Item -LiteralPath $file).Length
    if ($len -eq 0 -or $len%32) { throw "Invalid 32-byte file: $file" }
}
foreach($file in $old) { $oldCount += (Get-Item -LiteralPath $file).Length/32 }
foreach($file in $freshPaths) { $freshCount += (Get-Item -LiteralPath $file).Length/32 }
$replace=[long][math]::Floor($oldCount*$Fraction)
if ($freshCount -lt $replace) { throw "Need $replace fresh positions; have $freshCount" }
$drive = New-Object IO.DriveInfo ([IO.Path]::GetPathRoot([IO.Path]::GetFullPath($Output)))
if ($drive.AvailableFreeSpace -lt $oldCount*32 + 8GB) { throw 'Not enough free space for a full candidate plus 8 GiB reserve' }
New-Item -ItemType Directory -Path $Output | Out-Null
if (-not ('Gen8BinarySample' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
public static class Gen8BinarySample {
    public static void Write(string input, string output, long take, int seed, string health) {
        var original = new FileInfo(input);
        long length = original.Length;
        DateTime modified = original.LastWriteTimeUtc;
        long left = length / 32;
        var rng = new Random(seed);
        byte[] bytes = new byte[32 * 16384], selected = new byte[bytes.Length];
        using (var src = new FileStream(input, FileMode.Open, FileAccess.Read, FileShare.Read))
        using (var dst = new FileStream(output, FileMode.CreateNew, FileAccess.Write, FileShare.None)) {
            while (left > 0) {
                if (File.ReadAllText(health).IndexOf("pause", StringComparison.OrdinalIgnoreCase) >= 0)
                    throw new IOException("Health requests pause; partial output is not a completed mix");
                int count = (int)Math.Min(left, 16384), need = count * 32, got = 0, used = 0;
                while (got < need) { int n=src.Read(bytes,got,need-got); if(n==0) throw new EndOfStreamException(); got+=n; }
                for (int i=0; i<count; i++) {
                    bool keep = take == left || (take > 0 && rng.NextDouble() < (double)take/left);
                    if (keep) { Buffer.BlockCopy(bytes,i*32,selected,used,32); used+=32; take--; }
                    left--;
                }
                dst.Write(selected,0,used);
            }
            if (take != 0 || src.Length != length || File.GetLastWriteTimeUtc(input) != modified)
                throw new IOException("Input changed or sample incomplete");
        }
    }
}
'@
}
function Sample-Group($paths,[long]$wanted,[long]$total,$prefix) {
    $left=$total; $remaining=$wanted; $index=0
    foreach($path in $paths) {
        Check-Health
        $count=[long]((Get-Item -LiteralPath $path).Length/32)
        $take=if ($count -eq $left) { $remaining } else { [long][math]::Floor([double]$remaining*$count/$left) }
        $left-=$count; $remaining-=$take; $index++
        if ($take -gt 0) {
            $dest=Join-Path $Output "$prefix-$index.bin"
            [Gen8BinarySample]::Write($path,$dest,$take,($Seed+$index),"$lab\health\control.txt")
            if ((Get-Item -LiteralPath $dest).Length -ne $take*32) { throw 'Wrong output count' }
            (Resolve-Path -LiteralPath $dest).Path
        }
    }
}
$candidate=@(Sample-Group $old ($oldCount-$replace) $oldCount 'old') + @(Sample-Group $freshPaths $replace $freshCount 'fresh')
[IO.File]::WriteAllLines((Join-Path $Output 'control.txt'),[string[]]$old)
[IO.File]::WriteAllLines((Join-Path $Output 'candidate.txt'),[string[]]$candidate)
[pscustomobject]@{Total=$oldCount;Replaced=$replace;Fraction=$Fraction;Seed=$Seed;BaseListSHA256=(Get-FileHash $BaseList).Hash;Fresh=$freshPaths;Control='control.txt';Candidate='candidate.txt'} | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $Output 'manifest.json')
"Ready: $oldCount positions per arm, $replace replaced; $Output"
