#!/usr/bin/env pwsh
# SPDX-License-Identifier: MIT
#
# Release entry point for Windows onboarding.
# This ah-owned wrapper runs the cargo-dist PowerShell installer for native
# Windows ah/ahd, then invokes the shared WSL provisioning helper.

[CmdletBinding()]
param(
    [Alias('dist-installer-url')]
    [string]$DistInstallerUrl = 'https://github.com/SevenX77/ccbd-rust/releases/latest/download/ah-installer.ps1',

    [Alias('ah-install-url')]
    [string]$AhInstallUrl = 'https://github.com/SevenX77/ccbd-rust/releases/latest/download/ah-installer.sh',

    [Alias('expected-ah-version')]
    [string]$ExpectedAhVersion = $env:AH_SETUP_EXPECTED_VERSION,

    [Alias('distro')]
    [string]$Distro = 'Ubuntu',

    [Alias('yes')]
    [switch]$Yes,

    [Alias('json')]
    [switch]$Json,

    [Alias('dry-run')]
    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$modulePath = Join-Path $PSScriptRoot 'AhProvisioning.psm1'
$provisionScript = Join-Path $PSScriptRoot 'provision-ah-wsl.ps1'
Import-Module $modulePath -Force

$plan = New-AhWindowsInstallerHookPlan `
    -DistInstallerUrl $DistInstallerUrl `
    -AhInstallUrl $AhInstallUrl `
    -ExpectedAhVersion $ExpectedAhVersion `
    -Distro $Distro `
    -Yes:$Yes

if ($DryRun) {
    if ($Json) {
        $plan | ConvertTo-Json -Depth 16
    } else {
        Write-Output "ah Windows installer hook dry-run"
        Write-Output "dist installer: $($plan.dist_installer_url)"
        Write-Output "provision script: $($plan.provision_script) $($plan.provision_args -join ' ')"
        Write-Output "release-time validation required:"
        foreach ($item in @($plan.release_time_required)) {
            Write-Output "  - $item"
        }
    }
    exit 0
}

Write-Output "Installing native Windows ah/ahd via cargo-dist PowerShell installer..."
$distInstallerPath = Join-Path ([System.IO.Path]::GetTempPath()) "ah-dist-installer.$PID.ps1"
try {
    Invoke-RestMethod -Uri $DistInstallerUrl | Set-Content -LiteralPath $distInstallerPath -Encoding UTF8
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $distInstallerPath
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} finally {
    Remove-Item -LiteralPath $distInstallerPath -Force -ErrorAction SilentlyContinue
}

Write-Output "Starting WSL provisioning..."
$provisionArgs = @($plan.provision_args)
& $provisionScript @provisionArgs
exit $LASTEXITCODE
