# Read positional arguments directly: PowerShell's parameter binder treats
# GNU-style doctor flags as unbound named parameters instead of positional
# values, which would separate them from the config path and reorder input.
$Mode = if ($args.Count -gt 0) { [string]$args[0] } else { "serve" }
$SecondArg = if ($args.Count -gt 1) { [string]$args[1] } else { $null }
$ThirdArg = if ($args.Count -gt 2) { [string]$args[2] } else { $null }

$ErrorActionPreference = "Stop"
# Preserve native command exit codes even when the caller enabled PowerShell's
# opt-in conversion of nonzero native exits into terminating errors.
$PSNativeCommandUseErrorActionPreference = $false

if ($args.Count -gt 3) {
    [Console]::Error.WriteLine("too many arguments for mode: $Mode")
    exit 2
}

switch -CaseSensitive ($Mode) {
    "serve" { }
    "init" { }
    "migrate" { }
    "doctor" { }
    "bootstrap-local" { }
    default {
        [Console]::Error.WriteLine("unsupported mode: $Mode")
        [Console]::Error.WriteLine("usage: pwsh -File .\scripts\agent-llm-mm.ps1 [serve|init|migrate|doctor|bootstrap-local] [config_path]")
        exit 2
    }
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $scriptDir "..")).Path
$doctorMode = $null
$configPath = $null

if ($Mode -eq "doctor") {
    if ($SecondArg -and $SecondArg.StartsWith("--")) {
        $doctorMode = $SecondArg
        $configPath = $ThirdArg
    }
    else {
        $doctorMode = "--read-only"
        $configPath = $SecondArg
        if ($args.Count -gt 2) {
            [Console]::Error.WriteLine("too many arguments for mode: doctor")
            exit 2
        }
    }
    if ($doctorMode -cnotin @("--read-only", "--allow-bootstrap")) {
        [Console]::Error.WriteLine("unsupported doctor mode: $doctorMode")
        [Console]::Error.WriteLine("usage: pwsh -File .\scripts\agent-llm-mm.ps1 doctor [--read-only|--allow-bootstrap] [config_path]")
        exit 2
    }
}
else {
    $configPath = $SecondArg
    if ($args.Count -gt 2) {
        [Console]::Error.WriteLine("too many arguments for mode: $Mode")
        exit 2
    }
}

$hadConfigPath = Test-Path Env:AGENT_LLM_MM_CONFIG
$previousConfigPath = $env:AGENT_LLM_MM_CONFIG
Push-Location -LiteralPath $projectRoot
try {
    if ($Mode -eq "bootstrap-local") {
        $targetPath = if ($configPath) { $configPath } else { "agent-llm-mm.local.toml" }
        # .NET File.Copy resolves relative paths against the process directory,
        # not PowerShell's Push-Location. Resolve explicitly before copying.
        $targetPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($targetPath)
        $sourcePath = Join-Path $projectRoot "examples/agent-llm-mm.dev.example.toml"
        $targetParent = Split-Path -Parent $targetPath
        if (-not $targetParent) {
            $targetParent = "."
        }

        if (Test-Path -LiteralPath $targetPath) {
            [Console]::Error.WriteLine("target already exists; refusing to overwrite: $targetPath")
            exit 1
        }
        if (-not (Test-Path -LiteralPath $targetParent -PathType Container)) {
            [Console]::Error.WriteLine("parent directory does not exist: $targetParent")
            exit 1
        }

        try {
            [System.IO.File]::Copy($sourcePath, $targetPath, $false)
        }
        catch [System.IO.IOException] {
            if (Test-Path -LiteralPath $targetPath) {
                [Console]::Error.WriteLine("target already exists; refusing to overwrite: $targetPath")
            }
            else {
                [Console]::Error.WriteLine("cannot create local config: $($_.Exception.Message)")
            }
            exit 1
        }
        $quotedTargetPath = "'" + ($targetPath -replace "'", "''") + "'"
        Write-Output "created local config: $targetPath"
        Write-Output "Next commands:"
        Write-Output "  pwsh -File .\scripts\agent-llm-mm.ps1 init $quotedTargetPath"
        Write-Output "  pwsh -File .\scripts\agent-llm-mm.ps1 doctor --read-only $quotedTargetPath"
        Write-Output "  pwsh -File .\scripts\agent-llm-mm.ps1 serve $quotedTargetPath"
        exit 0
    }

    if ($configPath) {
        $resolvedConfigPath = (Resolve-Path -LiteralPath $configPath).Path
        $env:AGENT_LLM_MM_CONFIG = $resolvedConfigPath
    }

    if ($Mode -eq "doctor") {
        & cargo run --quiet --bin agent_llm_mm -- $Mode $doctorMode
    }
    else {
        & cargo run --quiet --bin agent_llm_mm -- $Mode
    }
    exit $LASTEXITCODE
}
finally {
    if ($hadConfigPath) {
        $env:AGENT_LLM_MM_CONFIG = $previousConfigPath
    }
    else {
        Remove-Item Env:AGENT_LLM_MM_CONFIG -ErrorAction SilentlyContinue
    }
    Pop-Location
}
