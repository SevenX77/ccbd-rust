Set-StrictMode -Version Latest

$script:SchemaVersion = 1
$script:PhaseName = 'phase2_windows_host'
$script:RequiredFeatureNames = @(
    'Microsoft-Windows-Subsystem-Linux',
    'VirtualMachinePlatform'
)

function Get-AhRequiredFeatureNames {
    [CmdletBinding()]
    param()

    return @($script:RequiredFeatureNames)
}

function New-AhOperationId {
    [CmdletBinding()]
    param()

    return [guid]::NewGuid().ToString()
}

function Get-AhDefaultStatePath {
    [CmdletBinding()]
    param()

    $root = $env:LOCALAPPDATA
    if ([string]::IsNullOrWhiteSpace($root)) {
        $root = Join-Path ([System.IO.Path]::GetTempPath()) 'ah-localappdata'
    }

    return Join-Path (Join-Path $root 'ah') 'setup-state.json'
}

function Read-AhSetupState {
    [CmdletBinding()]
    param(
        [string]$Path = (Get-AhDefaultStatePath)
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return $null
    }

    $raw = Get-Content -LiteralPath $Path -Raw
    if ([string]::IsNullOrWhiteSpace($raw)) {
        return $null
    }

    return $raw | ConvertFrom-Json
}

function Write-AhSetupState {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [object]$State,

        [string]$Path = (Get-AhDefaultStatePath)
    )

    $dir = Split-Path -Parent $Path
    if (-not [string]::IsNullOrWhiteSpace($dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }

    $tmp = "$Path.tmp.$PID"
    $State | ConvertTo-Json -Depth 16 | Set-Content -LiteralPath $tmp -Encoding UTF8
    Move-Item -LiteralPath $tmp -Destination $Path -Force
}

function New-AhSetupStep {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Id,

        [Parameter(Mandatory = $true)]
        [string]$Status,

        [string]$Owner = 'AhRuntime',
        [bool]$FixAvailable = $false,
        [string]$Privilege = 'user',
        [string]$Boundary = 'windows-host',
        [string]$Restart = 'none',
        [string]$Detail = '',
        [string]$Suggestion = '',
        [string]$ResumeToken = $null
    )

    return [ordered]@{
        id           = $Id
        status       = $Status
        owner        = $Owner
        fix_available = $FixAvailable
        privilege    = $Privilege
        boundary     = $Boundary
        restart      = $Restart
        detail       = $Detail
        suggestion   = $Suggestion
        resume_token = $ResumeToken
    }
}

function New-AhNextAction {
    [CmdletBinding()]
    param(
        [string]$Kind = 'none',
        [string]$Message = '',
        [string]$Command = $null
    )

    return [ordered]@{
        kind    = $Kind
        message = $Message
        command = $Command
    }
}

function Get-AhResumeCommand {
    [CmdletBinding()]
    param(
        [string]$ScriptPath = '.\provision-ah-wsl.ps1'
    )

    return "powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ScriptPath --resume"
}

function New-AhSetupEnvelope {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$OperationId,

        [Parameter(Mandatory = $true)]
        [string]$OverallStatus,

        [string]$SelectedDistro = $null,

        [object]$NextAction = $null,

        [string]$ResumeCommand = $null,

        [object[]]$Steps = @()
    )

    if ($null -eq $NextAction) {
        $NextAction = New-AhNextAction
    }

    return [ordered]@{
        schema_version = $script:SchemaVersion
        operation_id   = $OperationId
        overall_status = $OverallStatus
        phase          = $script:PhaseName
        selected_distro = $SelectedDistro
        next_action    = $NextAction
        resume_command = $ResumeCommand
        steps          = @($Steps)
    }
}

function New-AhWindowsHostState {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$OperationId,

        [string]$SelectedDistro = 'Ubuntu',
        [string]$PendingRestart = 'none',
        [string]$LastCompletedStep = $null,
        [object]$FeatureSteps = $null,
        [object]$AhInstall = $null,
        [string]$LastError = $null
    )

    $now = [DateTimeOffset]::UtcNow.ToString('o')
    if ($null -eq $FeatureSteps) {
        $FeatureSteps = [ordered]@{}
    }
    if ($null -eq $AhInstall) {
        $AhInstall = [ordered]@{}
    }

    return [ordered]@{
        schema_version = $script:SchemaVersion
        operation_id   = $OperationId
        helper_version = 'p2-0'
        phase          = $script:PhaseName
        boundary       = 'windows-host'
        selected_distro = $SelectedDistro
        requested_default_wsl_version = 2
        feature_steps  = $FeatureSteps
        selected_distro_wsl_version = $null
        pending_restart = $PendingRestart
        ah_install     = $AhInstall
        last_completed_step = $LastCompletedStep
        created_at     = $now
        updated_at     = $now
        last_error     = $LastError
    }
}

function Get-AhWindowsOptionalFeature {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $feature = Get-WindowsOptionalFeature -Online -FeatureName $Name
    return [pscustomobject]@{
        Name = $Name
        State = [string]$feature.State
    }
}

function New-AhDismEnableFeatureArguments {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    return @(
        '/online',
        '/enable-feature',
        "/featurename:$Name",
        '/all',
        '/norestart'
    )
}

function Invoke-AhDismEnableFeature {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    $arguments = New-AhDismEnableFeatureArguments -Name $Name
    $output = & dism.exe @arguments 2>&1
    $exitCode = if ($null -ne $LASTEXITCODE) { [int]$LASTEXITCODE } else { 1 }

    return [pscustomobject]@{
        feature = $Name
        arguments = @($arguments)
        exit_code = $exitCode
        output = @($output | ForEach-Object { [string]$_ })
    }
}

function Invoke-AhWsl {
    [CmdletBinding()]
    param(
        [string[]]$Arguments = @()
    )

    $output = & wsl.exe @Arguments 2>&1
    $exitCode = if ($null -ne $LASTEXITCODE) { [int]$LASTEXITCODE } else { 1 }

    return [pscustomobject]@{
        arguments = @($Arguments)
        exit_code = $exitCode
        output = @($output | ForEach-Object { [string]$_ })
    }
}

function Read-AhLxssRegistry {
    [CmdletBinding()]
    param(
        [string]$DistroName = 'Ubuntu'
    )

    throw "Read-AhLxssRegistry is a host wrapper and is not implemented in P2-0. Tests must mock it."
}

function Start-AhElevatedFeatureChild {
    [CmdletBinding()]
    param(
        [string[]]$FeatureNames = @(),
        [Parameter(Mandatory = $true)]
        [string]$OperationId,
        [Parameter(Mandatory = $true)]
        [string]$ResultPath
    )

    $command = New-AhElevatedFeatureChildCommand `
        -FeatureNames $FeatureNames `
        -OperationId $OperationId `
        -ResultPath $ResultPath

    $resultDir = Split-Path -Parent $ResultPath
    if (-not [string]::IsNullOrWhiteSpace($resultDir)) {
        New-Item -ItemType Directory -Path $resultDir -Force | Out-Null
    }

    try {
        $process = Start-Process `
            -FilePath $command.FilePath `
            -ArgumentList $command.ArgumentList `
            -Verb RunAs `
            -Wait `
            -PassThru
    } catch {
        return [pscustomobject]@{
            status = 'permission_denied'
            exit_code = $null
            result_path = $ResultPath
            error = $_.Exception.Message
        }
    }

    if ($null -eq $process) {
        return [pscustomobject]@{
            status = 'permission_denied'
            exit_code = $null
            result_path = $ResultPath
            error = 'elevated child did not start'
        }
    }

    if (Test-Path -LiteralPath $ResultPath) {
        return Get-Content -LiteralPath $ResultPath -Raw | ConvertFrom-Json
    }

    if ($process.ExitCode -eq 0) {
        return [pscustomobject]@{
            status = 'reprobe_required'
            exit_code = $process.ExitCode
            result_path = $ResultPath
        }
    }

    return [pscustomobject]@{
        status = 'fail'
        exit_code = $process.ExitCode
        result_path = $ResultPath
        error = 'elevated child exited without result file'
    }
}

function New-AhElevatedFeatureChildCommand {
    [CmdletBinding()]
    param(
        [string[]]$FeatureNames = @(),
        [Parameter(Mandatory = $true)]
        [string]$OperationId,
        [Parameter(Mandatory = $true)]
        [string]$ResultPath,
        [string]$ChildScriptPath = (Join-Path $PSScriptRoot 'enable-ah-wsl-features.ps1')
    )

    $arguments = @(
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-File',
        $ChildScriptPath,
        '-OperationId',
        $OperationId,
        '-ResultPath',
        $ResultPath
    )

    foreach ($feature in @($FeatureNames)) {
        $arguments += '-FeatureName'
        $arguments += $feature
    }

    return [pscustomobject]@{
        FilePath = 'powershell.exe'
        ArgumentList = @($arguments)
    }
}

function Get-AhFeatureStatusValue {
    [CmdletBinding()]
    param(
        [object]$Feature
    )

    if ($null -eq $Feature) {
        return 'Unknown'
    }
    if ($Feature -is [string]) {
        return $Feature
    }
    if ($Feature.PSObject.Properties.Name -contains 'State') {
        return [string]$Feature.State
    }
    if ($Feature.PSObject.Properties.Name -contains 'Status') {
        return [string]$Feature.Status
    }
    return 'Unknown'
}

function New-AhNeedsWindowsRebootEnvelope {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [string]$OperationId,

        [string]$SelectedDistro = 'Ubuntu',
        [string]$Reason = 'Windows features are pending reboot.',
        [string]$StatePath = (Get-AhDefaultStatePath)
    )

    $resume = Get-AhResumeCommand
    $step = New-AhSetupStep `
        -Id 'windows:wsl-feature' `
        -Status 'needs_windows_reboot' `
        -FixAvailable $true `
        -Privilege 'admin' `
        -Restart 'needs_windows_reboot' `
        -Detail $Reason `
        -Suggestion "Reboot Windows, then rerun: $resume"

    $next = New-AhNextAction `
        -Kind 'reboot_windows' `
        -Message "Windows must reboot before WSL provisioning can continue. State: $StatePath" `
        -Command 'Restart-Computer'

    return New-AhSetupEnvelope `
        -OperationId $OperationId `
        -OverallStatus 'needs_windows_reboot' `
        -SelectedDistro $SelectedDistro `
        -NextAction $next `
        -ResumeCommand $resume `
        -Steps @($step)
}

function Invoke-AhPhase2Provisioning {
    [CmdletBinding()]
    param(
        [switch]$Check,
        [switch]$Fix,
        [switch]$Yes,
        [switch]$Resume,
        [switch]$DryRun,
        [string]$SelectedDistro = 'Ubuntu',
        [string]$StatePath = (Get-AhDefaultStatePath)
    )

    $state = $null
    if ($Resume) {
        $state = Read-AhSetupState -Path $StatePath
    }

    $operationId = New-AhOperationId
    if ($null -ne $state -and ($state.PSObject.Properties.Name -contains 'operation_id')) {
        $operationId = [string]$state.operation_id
    }

    if ($DryRun) {
        $step = New-AhSetupStep `
            -Id 'p2-0:contract' `
            -Status 'pass' `
            -Detail 'Dry-run validated the Phase 2 contract without probing host state.'
        return New-AhSetupEnvelope `
            -OperationId $operationId `
            -OverallStatus 'pass' `
            -SelectedDistro $SelectedDistro `
            -Steps @($step)
    }

    if ($null -ne $state -and ($state.PSObject.Properties.Name -contains 'pending_restart') -and $state.pending_restart -eq 'windows_reboot') {
        return New-AhNeedsWindowsRebootEnvelope `
            -OperationId $operationId `
            -SelectedDistro $SelectedDistro `
            -Reason 'Previous run stopped at a Windows reboot boundary.' `
            -StatePath $StatePath
    }

    $featureNames = Get-AhRequiredFeatureNames

    $featureStatuses = @()
    foreach ($featureName in $featureNames) {
        $feature = Get-AhWindowsOptionalFeature -Name $featureName
        $featureStatuses += [pscustomobject]@{
            Name = $featureName
            State = Get-AhFeatureStatusValue -Feature $feature
        }
    }

    $pending = @($featureStatuses | Where-Object { $_.State -eq 'EnablePending' })
    if ($pending.Count -gt 0) {
        $newState = New-AhWindowsHostState `
            -OperationId $operationId `
            -SelectedDistro $SelectedDistro `
            -PendingRestart 'windows_reboot' `
            -LastCompletedStep 'windows_feature_enable'
        Write-AhSetupState -State $newState -Path $StatePath
        return New-AhNeedsWindowsRebootEnvelope `
            -OperationId $operationId `
            -SelectedDistro $SelectedDistro `
            -Reason 'One or more Windows features are EnablePending.' `
            -StatePath $StatePath
    }

    $unknown = @($featureStatuses | Where-Object { $_.State -ne 'Enabled' -and $_.State -ne 'Disabled' -and $_.State -ne 'EnablePending' })
    if ($unknown.Count -gt 0) {
        $steps = foreach ($item in $unknown) {
            New-AhSetupStep `
                -Id "windows:feature:$($item.Name)" `
                -Status 'fail' `
                -FixAvailable $false `
                -Privilege 'user' `
                -Detail "Windows optional feature probe returned unknown state '$($item.State)'." `
                -Suggestion 'Inspect Windows optional feature status, then rerun this helper.'
        }
        return New-AhSetupEnvelope `
            -OperationId $operationId `
            -OverallStatus 'fail' `
            -SelectedDistro $SelectedDistro `
            -Steps @($steps)
    }

    $disabled = @($featureStatuses | Where-Object { $_.State -ne 'Enabled' })
    if ($disabled.Count -gt 0 -and -not $Fix) {
        $steps = foreach ($item in $disabled) {
            New-AhSetupStep `
                -Id "windows:feature:$($item.Name)" `
                -Status 'fail' `
                -FixAvailable $true `
                -Privilege 'admin' `
                -Detail "Windows optional feature is $($item.State)." `
                -Suggestion 'Run this helper with --fix to enable missing WSL2 features.'
        }
        return New-AhSetupEnvelope `
            -OperationId $operationId `
            -OverallStatus 'fail' `
            -SelectedDistro $SelectedDistro `
            -Steps @($steps)
    }

    if ($disabled.Count -gt 0 -and $Fix) {
        $resultPath = Join-Path (Split-Path -Parent $StatePath) "setup-elevated-result.$operationId.json"
        $child = Start-AhElevatedFeatureChild `
            -FeatureNames @($disabled.Name) `
            -OperationId $operationId `
            -ResultPath $resultPath

        $childStatus = 'fail'
        if ($null -ne $child -and ($child.PSObject.Properties.Name -contains 'status')) {
            $childStatus = [string]$child.status
        }

        if ($childStatus -eq 'permission_denied') {
            $step = New-AhSetupStep `
                -Id 'windows:feature-elevation' `
                -Status 'permission_denied' `
                -FixAvailable $true `
                -Privilege 'admin' `
                -Detail 'User cancelled or denied UAC elevation.'
            return New-AhSetupEnvelope `
                -OperationId $operationId `
                -OverallStatus 'permission_denied' `
                -SelectedDistro $SelectedDistro `
                -Steps @($step)
        }

        if ($childStatus -eq 'needs_windows_reboot' -or $childStatus -eq 'partial_enable') {
            $featureSteps = [ordered]@{}
            if ($null -ne $child -and ($child.PSObject.Properties.Name -contains 'features')) {
                foreach ($feature in @($child.features)) {
                    $featureSteps[$feature.name] = $feature
                }
            }
            $newState = New-AhWindowsHostState `
                -OperationId $operationId `
                -SelectedDistro $SelectedDistro `
                -PendingRestart 'windows_reboot' `
                -LastCompletedStep 'windows_feature_enable' `
                -FeatureSteps $featureSteps
            if ($childStatus -eq 'partial_enable') {
                $newState['partial_enable'] = $true
            }
            Write-AhSetupState -State $newState -Path $StatePath
            return New-AhNeedsWindowsRebootEnvelope `
                -OperationId $operationId `
                -SelectedDistro $SelectedDistro `
                -Reason "Elevated feature child returned $childStatus." `
                -StatePath $StatePath
        }

        if ($childStatus -eq 'reprobe_required') {
            $reprobedStatuses = @()
            foreach ($featureName in $featureNames) {
                $feature = Get-AhWindowsOptionalFeature -Name $featureName
                $reprobedStatuses += [pscustomobject]@{
                    Name = $featureName
                    State = Get-AhFeatureStatusValue -Feature $feature
                }
            }

            $reprobedPending = @($reprobedStatuses | Where-Object { $_.State -eq 'EnablePending' })
            if ($reprobedPending.Count -gt 0) {
                $newState = New-AhWindowsHostState `
                    -OperationId $operationId `
                    -SelectedDistro $SelectedDistro `
                    -PendingRestart 'windows_reboot' `
                    -LastCompletedStep 'windows_feature_enable'
                Write-AhSetupState -State $newState -Path $StatePath
                return New-AhNeedsWindowsRebootEnvelope `
                    -OperationId $operationId `
                    -SelectedDistro $SelectedDistro `
                    -Reason 'Elevated child returned success without result file; re-probe found EnablePending.' `
                    -StatePath $StatePath
            }

            $reprobedNotEnabled = @($reprobedStatuses | Where-Object { $_.State -ne 'Enabled' })
            if ($reprobedNotEnabled.Count -eq 0) {
                $featureStatuses = $reprobedStatuses
                $disabled = @()
            } else {
                $step = New-AhSetupStep `
                    -Id 'windows:feature-enable' `
                    -Status 'fail' `
                    -FixAvailable $true `
                    -Privilege 'admin' `
                    -Detail 'Elevated child returned success without a result file; re-probe did not find all features enabled or pending.'
                return New-AhSetupEnvelope `
                    -OperationId $operationId `
                    -OverallStatus 'fail' `
                    -SelectedDistro $SelectedDistro `
                    -Steps @($step)
            }
        }

        if ($disabled.Count -gt 0) {
            $step = New-AhSetupStep `
                -Id 'windows:feature-enable' `
                -Status 'fail' `
                -FixAvailable $true `
                -Privilege 'admin' `
                -Detail "Elevated feature child returned $childStatus."
            return New-AhSetupEnvelope `
                -OperationId $operationId `
                -OverallStatus 'fail' `
                -SelectedDistro $SelectedDistro `
                -Steps @($step)
        }
    }

    $passSteps = foreach ($item in $featureStatuses) {
        New-AhSetupStep `
            -Id "windows:feature:$($item.Name)" `
            -Status 'pass' `
            -Detail "Windows optional feature is $($item.State)."
    }

    $wslStatus = Invoke-AhWsl -Arguments @('--status')
    if ($wslStatus.exit_code -ne 0) {
        $steps = @($passSteps)
        $steps += New-AhSetupStep `
            -Id 'windows:wsl-status' `
            -Status 'fail' `
            -Detail "wsl.exe --status failed with exit code $($wslStatus.exit_code)." `
            -Suggestion 'Inspect WSL status in PowerShell, then rerun this helper.'
        return New-AhSetupEnvelope `
            -OperationId $operationId `
            -OverallStatus 'fail' `
            -SelectedDistro $SelectedDistro `
            -Steps @($steps)
    }

    $passSteps += New-AhSetupStep `
        -Id 'windows:wsl-status' `
        -Status 'pass' `
        -Detail 'wsl.exe --status completed.'

    return New-AhSetupEnvelope `
        -OperationId $operationId `
        -OverallStatus 'pass' `
        -SelectedDistro $SelectedDistro `
        -Steps @($passSteps)
}

Export-ModuleMember -Function @(
    'New-AhOperationId',
    'Get-AhDefaultStatePath',
    'Read-AhSetupState',
    'Write-AhSetupState',
    'New-AhSetupStep',
    'New-AhNextAction',
    'Get-AhResumeCommand',
    'New-AhSetupEnvelope',
    'New-AhWindowsHostState',
    'Get-AhRequiredFeatureNames',
    'Get-AhWindowsOptionalFeature',
    'New-AhDismEnableFeatureArguments',
    'Invoke-AhDismEnableFeature',
    'Invoke-AhWsl',
    'Read-AhLxssRegistry',
    'Start-AhElevatedFeatureChild',
    'New-AhElevatedFeatureChildCommand',
    'Invoke-AhPhase2Provisioning'
)
