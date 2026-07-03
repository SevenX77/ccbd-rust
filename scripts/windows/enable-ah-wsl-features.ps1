#!/usr/bin/env pwsh
# SPDX-License-Identifier: MIT

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OperationId,

    [Parameter(Mandatory = $true)]
    [string]$ResultPath,

    [Parameter(Mandatory = $true)]
    [string[]]$FeatureName
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$modulePath = Join-Path $PSScriptRoot 'AhProvisioning.psm1'
Import-Module $modulePath -Force

function Write-AhElevatedResult {
    param(
        [Parameter(Mandatory = $true)]
        [object]$Result
    )

    $dir = Split-Path -Parent $ResultPath
    if (-not [string]::IsNullOrWhiteSpace($dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }

    $Result | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $ResultPath -Encoding UTF8
}

$features = @()
$hadSuccess = $false
$hadFailure = $false
$stderrTail = @()

foreach ($name in @($FeatureName)) {
    try {
        $result = Invoke-AhDismEnableFeature -Name $name
        $exitCode = [int]$result.exit_code
        $ok = $exitCode -eq 0 -or $exitCode -eq 3010
        if ($ok) {
            $hadSuccess = $true
        } else {
            $hadFailure = $true
        }

        $features += [ordered]@{
            name = $name
            exit_code = $exitCode
            arguments = @($result.arguments)
            status = if ($ok) { 'requested' } else { 'failed' }
        }

        if ($result.PSObject.Properties.Name -contains 'output') {
            $stderrTail += @($result.output | Select-Object -Last 20)
        }
    } catch {
        $hadFailure = $true
        $stderrTail += $_.Exception.Message
        $features += [ordered]@{
            name = $name
            exit_code = $null
            arguments = @(New-AhDismEnableFeatureArguments -Name $name)
            status = 'failed'
            error = $_.Exception.Message
        }
    }
}

$status = 'fail'
if ($hadSuccess -and $hadFailure) {
    $status = 'partial_enable'
} elseif ($hadSuccess) {
    $status = 'needs_windows_reboot'
}

$result = [ordered]@{
    operation_id = $OperationId
    status = $status
    features = @($features)
    dism_exit_codes = @($features | ForEach-Object { $_.exit_code })
    reboot_required = $hadSuccess
    partial_enable = ($status -eq 'partial_enable')
    stderr_tail = @($stderrTail | Select-Object -Last 20)
}

Write-AhElevatedResult -Result $result

if ($status -eq 'fail') {
    exit 1
}

exit 0
