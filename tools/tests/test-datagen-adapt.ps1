# Execute only the ownership predicate extracted from the adapter AST; never run its loop.
$ErrorActionPreference = 'Stop'
$tokens=$null; $parseErrors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot '..\datagen-adapt.ps1'),[ref]$tokens,[ref]$parseErrors)
if ($parseErrors.Count) { throw 'Adapter parser errors' }
$fn=$ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Test-OwnedGenerator' },$true)
. ([scriptblock]::Create($fn.Extent.Text))
$exePath='C:\lab\engines\pawnstorm-datagen-gen8k.exe'
$outputPath='C:\lab\datagen\gen8k-big7-n5000'
function Assert-Owned($path,$command,$expected) {
    $actual=Test-OwnedGenerator ([pscustomobject]@{ExecutablePath=$path;CommandLine=$command})
    if ($actual -ne $expected) { throw "Unexpected ownership: $command" }
}
Assert-Owned $exePath ('"'+$exePath+'" datagen 6 1000000000 5000 "'+$outputPath+'" --fens seeds.fen') $true
Assert-Owned $exePath ($exePath+' datagen 1 1 5000 '+$outputPath) $true
Assert-Owned $exePath ($exePath+' datagen 1 1 5000 '+$outputPath+'-other') $false
Assert-Owned $exePath ($exePath+' datagen 1 1 5000 C:\lab\datagen\gen7k-v20-5k') $false
Assert-Owned 'C:\lab\engines\other.exe' ($exePath+' datagen 1 1 5000 '+$outputPath) $false
Assert-Owned $exePath ($exePath+' bench 10') $false
Assert-Owned $exePath '' $false
Assert-Owned '' ($exePath+' datagen 1 1 5000 '+$outputPath) $false
$exePath='C:\lab space\engines\gen8.exe'; $outputPath='C:\lab space\datagen\gen8'
Assert-Owned $exePath ('"'+$exePath+'" datagen 1 1 5000 "'+$outputPath+'"') $true
# Execute just the busy assignment with a fake SPSA fastchess process.
$busy=$ast.Find({ param($node) $node -is [Management.Automation.Language.AssignmentStatementAst] -and $node.Left.Extent.Text -eq '$tournament' },$true)
function Get-Process { param([string[]]$Name,$ErrorAction) if ('fastchess' -like $Name[0]) { [pscustomobject]@{ProcessName='fastchess';Purpose='spsa'} } }
Invoke-Expression $busy.Extent.Text
if (-not $tournament) { throw 'SPSA fastchess was not considered busy' }
'Adapter ownership/busy checks passed (no processes started or stopped)'
