# SPDX-License-Identifier: MIT

BeforeAll {
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..' '..')).Path
    $ModulePath = Join-Path $RepoRoot 'scripts/windows/AhProvisioning.psm1'
    Import-Module $ModulePath -Force
}

Describe 'Req1 Phase 2 P2-0 contract' {
    It 'renders the required JSON envelope fields' {
        $step = New-AhSetupStep -Id 'contract:test' -Status 'pass'
        $envelope = New-AhSetupEnvelope `
            -OperationId 'op-test' `
            -OverallStatus 'pass' `
            -SelectedDistro 'Ubuntu' `
            -Steps @($step)

        $json = $envelope | ConvertTo-Json -Depth 16 | ConvertFrom-Json
        $json.schema_version | Should -Be 1
        $json.operation_id | Should -Be 'op-test'
        $json.overall_status | Should -Be 'pass'
        $json.phase | Should -Be 'phase2_windows_host'
        $json.selected_distro | Should -Be 'Ubuntu'
        $json.PSObject.Properties.Name | Should -Contain 'next_action'
        $json.PSObject.Properties.Name | Should -Contain 'resume_command'
        $json.PSObject.Properties.Name | Should -Contain 'steps'
    }

    It 'returns pass when mocked feature probes are enabled' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Enabled' }
        }
        Mock -ModuleName AhProvisioning Invoke-AhWsl {
            if ($Arguments[0] -eq '--status') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('Default Version: 2') }
            }
            if ($Arguments[0] -eq '--set-default-version') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('The operation completed successfully.') }
            }
            if ($Arguments[0] -eq '-l') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('  NAME      STATE           VERSION', '* Ubuntu    Stopped         2') }
            }
            throw "unexpected wsl args: $($Arguments -join ' ')"
        }
        Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild {
            throw 'should not elevate when features are enabled'
        }

        $envelope = Invoke-AhPhase2Provisioning -Check -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'pass'
        @($envelope.steps).Count | Should -Be 5
        Should -Invoke -ModuleName AhProvisioning Get-AhWindowsOptionalFeature -Times 2 -Exactly
        Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 1 -Exactly -ParameterFilter {
            $Arguments.Count -eq 1 -and $Arguments[0] -eq '--status'
        }
        Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 0 -Exactly -ParameterFilter {
            $Arguments.Count -eq 2 -and $Arguments[0] -eq '--set-default-version' -and $Arguments[1] -eq '2'
        }
        Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 1 -Exactly -ParameterFilter {
            $Arguments.Count -eq 2 -and $Arguments[0] -eq '-l' -and $Arguments[1] -eq '-v'
        }
        Should -Invoke -ModuleName AhProvisioning Start-AhElevatedFeatureChild -Times 0 -Exactly
    }

    It 'sets WSL default version to 2 only after feature probes pass' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Enabled' }
        }
        Mock -ModuleName AhProvisioning Invoke-AhWsl {
            if ($Arguments[0] -eq '--status') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('Default Version: 1') }
            }
            if ($Arguments[0] -eq '--set-default-version') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('The operation completed successfully.') }
            }
            if ($Arguments[0] -eq '-l') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('  NAME      STATE           VERSION', '* Ubuntu    Stopped         2') }
            }
            throw "unexpected wsl args: $($Arguments -join ' ')"
        }

        $envelope = Invoke-AhPhase2Provisioning -Check -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'pass'
        Should -Invoke -ModuleName AhProvisioning Get-AhWindowsOptionalFeature -Times 2 -Exactly
        Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 1 -Exactly -ParameterFilter {
            $Arguments.Count -eq 2 -and $Arguments[0] -eq '--set-default-version' -and $Arguments[1] -eq '2'
        }
    }

    It 'resumes after reboot by re-probing features and continuing to WSL status' {
        $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
        $statePath = Join-Path $temp 'setup-state.json'
        try {
            $state = New-AhWindowsHostState `
                -OperationId 'op-resume' `
                -SelectedDistro 'Ubuntu' `
                -PendingRestart 'windows_reboot' `
                -LastCompletedStep 'windows_feature_enable'
            Write-AhSetupState -State $state -Path $statePath

            Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
                [pscustomobject]@{ State = 'Enabled' }
            }
            Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild {
                throw 'should not repeat DISM after reboot when features are enabled'
            }
            Mock -ModuleName AhProvisioning Invoke-AhWsl {
                if ($Arguments[0] -eq '--status') {
                    return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('Default Version: 2') }
                }
                if ($Arguments[0] -eq '--set-default-version') {
                    return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('The operation completed successfully.') }
                }
                if ($Arguments[0] -eq '-l') {
                    return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('  NAME      STATE           VERSION', '* Ubuntu    Stopped         2') }
                }
                throw "unexpected wsl args: $($Arguments -join ' ')"
            }

            $envelope = Invoke-AhPhase2Provisioning -Resume -SelectedDistro 'Ubuntu' -StatePath $statePath

            $envelope.operation_id | Should -Be 'op-resume'
            $envelope.overall_status | Should -Be 'pass'
            Should -Invoke -ModuleName AhProvisioning Get-AhWindowsOptionalFeature -Times 2 -Exactly
            Should -Invoke -ModuleName AhProvisioning Start-AhElevatedFeatureChild -Times 0 -Exactly
            Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 1 -Exactly -ParameterFilter {
                $Arguments.Count -eq 1 -and $Arguments[0] -eq '--status'
            }
        } finally {
            Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'returns fail plan when mocked features are disabled without fix' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Disabled' }
        }

        $envelope = Invoke-AhPhase2Provisioning -Check -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'fail'
        @($envelope.steps)[0].fix_available | Should -BeTrue
        @($envelope.steps)[0].privilege | Should -Be 'admin'
    }

    It 'returns a reboot boundary with distinct next and resume commands for EnablePending' {
        $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
        $statePath = Join-Path $temp 'setup-state.json'
        New-Item -ItemType Directory -Path $temp -Force | Out-Null
        try {
            Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
                [pscustomobject]@{ State = 'EnablePending' }
            }
            Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild {
                throw 'should not elevate for EnablePending'
            }
            Mock -ModuleName AhProvisioning Invoke-AhWsl {
                throw 'should not call wsl for EnablePending'
            }

            $envelope = Invoke-AhPhase2Provisioning -Check -SelectedDistro 'Ubuntu' -StatePath $statePath

            $envelope.overall_status | Should -Be 'needs_windows_reboot'
            $envelope.next_action.command | Should -Be 'Restart-Computer'
            $envelope.resume_command | Should -Not -BeNullOrEmpty
            $envelope.next_action.command | Should -Not -Be $envelope.resume_command
            Test-Path -LiteralPath $statePath | Should -BeTrue
            Should -Invoke -ModuleName AhProvisioning Get-AhWindowsOptionalFeature -Times 2 -Exactly
            Should -Invoke -ModuleName AhProvisioning Start-AhElevatedFeatureChild -Times 0 -Exactly
            Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 0 -Exactly
        } finally {
            Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'maps mocked UAC denial to permission_denied' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Disabled' }
        }
        Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild {
            [pscustomobject]@{ status = 'permission_denied' }
        }

        $envelope = Invoke-AhPhase2Provisioning -Fix -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'permission_denied'
        @($envelope.steps)[0].status | Should -Be 'permission_denied'
    }

    It 'builds the exact elevated child command for disabled features with fix' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Disabled' }
        }
        Mock -ModuleName AhProvisioning Invoke-AhWsl {
            throw 'should not call wsl in same pass as feature enablement'
        }
        Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild {
            [pscustomobject]@{
                status = 'needs_windows_reboot'
                features = @(
                    [pscustomobject]@{ name = 'Microsoft-Windows-Subsystem-Linux'; status = 'requested'; exit_code = 0 },
                    [pscustomobject]@{ name = 'VirtualMachinePlatform'; status = 'requested'; exit_code = 0 }
                )
            }
        }

        $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
        $statePath = Join-Path $temp 'setup-state.json'
        try {
            $envelope = Invoke-AhPhase2Provisioning -Fix -SelectedDistro 'Ubuntu' -StatePath $statePath

            $envelope.overall_status | Should -Be 'needs_windows_reboot'
            Should -Invoke -ModuleName AhProvisioning Start-AhElevatedFeatureChild -Times 1 -Exactly -ParameterFilter {
                $FeatureNames.Count -eq 2 -and
                    $FeatureNames -contains 'Microsoft-Windows-Subsystem-Linux' -and
                    $FeatureNames -contains 'VirtualMachinePlatform' -and
                    $OperationId -and
                    $ResultPath -like '*setup-elevated-result*.json'
            }
            Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 0 -Exactly
            Test-Path -LiteralPath $statePath | Should -BeTrue
        } finally {
            Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'builds DISM enable-feature arguments with /all and /norestart' {
        $args = New-AhDismEnableFeatureArguments -Name 'VirtualMachinePlatform'

        $args | Should -Contain '/online'
        $args | Should -Contain '/enable-feature'
        $args | Should -Contain '/featurename:VirtualMachinePlatform'
        $args | Should -Contain '/all'
        $args | Should -Contain '/norestart'
    }

    It 'builds elevated child command with exact feature names and result file' {
        $command = New-AhElevatedFeatureChildCommand `
            -FeatureNames @('Microsoft-Windows-Subsystem-Linux', 'VirtualMachinePlatform') `
            -OperationId 'op-child' `
            -ResultPath 'C:\Users\user\AppData\Local\ah\setup-elevated-result.op-child.json' `
            -ChildScriptPath 'C:\repo\scripts\windows\enable-ah-wsl-features.ps1'

        $command.FilePath | Should -Be 'powershell.exe'
        $command.ArgumentList | Should -Contain '-NoProfile'
        $command.ArgumentList | Should -Contain '-ExecutionPolicy'
        $command.ArgumentList | Should -Contain 'Bypass'
        $command.ArgumentList | Should -Contain '-File'
        $command.ArgumentList | Should -Contain 'C:\repo\scripts\windows\enable-ah-wsl-features.ps1'
        $command.ArgumentList | Should -Contain '-OperationId'
        $command.ArgumentList | Should -Contain 'op-child'
        $command.ArgumentList | Should -Contain '-ResultPath'
        $command.ArgumentList | Should -Contain 'C:\Users\user\AppData\Local\ah\setup-elevated-result.op-child.json'
        $command.ArgumentList | Should -Contain '-FeatureName'
        $command.ArgumentList | Should -Contain 'Microsoft-Windows-Subsystem-Linux'
        $command.ArgumentList | Should -Contain 'VirtualMachinePlatform'
    }

    It 'parses WSL distro list output and selected distro version' {
        $distros = ConvertFrom-AhWslDistroList -Lines @(
            "  NAME      STATE           VERSION",
            "* Ubuntu    Running         1",
            "  Debian    Stopped         2"
        )
        $selected = Find-AhWslDistro -Distros $distros -SelectedDistro 'Ubuntu'

        @($distros).Count | Should -Be 2
        $selected.name | Should -Be 'Ubuntu'
        $selected.version | Should -Be 1
        $selected.default | Should -BeTrue
    }

    It 'returns WSL1 conversion plan without fix and does not continue to first-launch probes' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Enabled' }
        }
        Mock -ModuleName AhProvisioning Read-AhLxssRegistry {
            throw 'should not probe first-launch while selected distro is WSL1'
        }
        Mock -ModuleName AhProvisioning Invoke-AhWsl {
            if ($Arguments[0] -eq '--status') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('Default Version: 2') }
            }
            if ($Arguments[0] -eq '--set-default-version') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('The operation completed successfully.') }
            }
            if ($Arguments[0] -eq '-l') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('  NAME      STATE           VERSION', '* Ubuntu    Stopped         1') }
            }
            throw "unexpected wsl args: $($Arguments -join ' ')"
        }

        $envelope = Invoke-AhPhase2Provisioning -Check -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'fail'
        @($envelope.steps)[-1].id | Should -Be 'windows:wsl-distro-version'
        @($envelope.steps)[-1].detail | Should -Match 'Back up important distro data'
        @($envelope.steps)[-1].suggestion | Should -Match 'wsl.exe --set-version Ubuntu 2'
        Should -Invoke -ModuleName AhProvisioning Read-AhLxssRegistry -Times 0 -Exactly
        Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 0 -Exactly -ParameterFilter {
            $Arguments.Count -ge 1 -and $Arguments[0] -eq '--set-version'
        }
    }

    It 'runs WSL1 conversion with fix, records resumable state, and does not continue' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Enabled' }
        }
        Mock -ModuleName AhProvisioning Read-AhLxssRegistry {
            throw 'should not probe first-launch while selected distro is WSL1'
        }
        Mock -ModuleName AhProvisioning Invoke-AhWsl {
            if ($Arguments[0] -eq '--status') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('Default Version: 2') }
            }
            if ($Arguments[0] -eq '--set-default-version') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('The operation completed successfully.') }
            }
            if ($Arguments[0] -eq '-l') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('  NAME      STATE           VERSION', '* Ubuntu    Stopped         1') }
            }
            if ($Arguments[0] -eq '--set-version') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('Conversion in progress, this may take a few minutes...') }
            }
            throw "unexpected wsl args: $($Arguments -join ' ')"
        }

        $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
        $statePath = Join-Path $temp 'setup-state.json'
        try {
            $envelope = Invoke-AhPhase2Provisioning -Fix -SelectedDistro 'Ubuntu' -StatePath $statePath
            $state = Read-AhSetupState -Path $statePath

            $envelope.overall_status | Should -Be 'fixed'
            @($envelope.steps)[-1].status | Should -Be 'fixed'
            @($envelope.steps)[-1].detail | Should -Match 'Back up important distro data'
            $state.pending_conversion | Should -Be 'wsl2'
            $state.selected_distro_wsl_version | Should -Be 1
            Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 1 -Exactly -ParameterFilter {
                $Arguments.Count -eq 3 -and $Arguments[0] -eq '--set-version' -and $Arguments[1] -eq 'Ubuntu' -and $Arguments[2] -eq '2'
            }
            Should -Invoke -ModuleName AhProvisioning Read-AhLxssRegistry -Times 0 -Exactly
        } finally {
            Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'keeps WSL1 conversion failure resumable and does not continue' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Enabled' }
        }
        Mock -ModuleName AhProvisioning Read-AhLxssRegistry {
            throw 'should not probe first-launch while selected distro is WSL1'
        }
        Mock -ModuleName AhProvisioning Invoke-AhWsl {
            if ($Arguments[0] -eq '--status') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('Default Version: 2') }
            }
            if ($Arguments[0] -eq '--set-default-version') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('The operation completed successfully.') }
            }
            if ($Arguments[0] -eq '-l') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 0; output = @('  NAME      STATE           VERSION', '* Ubuntu    Stopped         1') }
            }
            if ($Arguments[0] -eq '--set-version') {
                return [pscustomobject]@{ arguments = @($Arguments); exit_code = 1; output = @('Conversion failed') }
            }
            throw "unexpected wsl args: $($Arguments -join ' ')"
        }

        $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
        $statePath = Join-Path $temp 'setup-state.json'
        try {
            $envelope = Invoke-AhPhase2Provisioning -Fix -SelectedDistro 'Ubuntu' -StatePath $statePath
            $state = Read-AhSetupState -Path $statePath

            $envelope.overall_status | Should -Be 'fail'
            @($envelope.steps)[-1].detail | Should -Match 'failed with exit code 1'
            @($envelope.steps)[-1].detail | Should -Match 'Back up important distro data'
            $state.pending_conversion | Should -Be 'wsl2'
            Should -Invoke -ModuleName AhProvisioning Read-AhLxssRegistry -Times 0 -Exactly
        } finally {
            Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'maps partial enable to reboot boundary with partial_enable state' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Disabled' }
        }
        Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild {
            [pscustomobject]@{
                status = 'partial_enable'
                features = @(
                    [pscustomobject]@{ name = 'Microsoft-Windows-Subsystem-Linux'; status = 'requested'; exit_code = 0 },
                    [pscustomobject]@{ name = 'VirtualMachinePlatform'; status = 'failed'; exit_code = 1 }
                )
            }
        }

        $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
        $statePath = Join-Path $temp 'setup-state.json'
        try {
            $envelope = Invoke-AhPhase2Provisioning -Fix -SelectedDistro 'Ubuntu' -StatePath $statePath
            $state = Read-AhSetupState -Path $statePath

            $envelope.overall_status | Should -Be 'needs_windows_reboot'
            $state.pending_restart | Should -Be 'windows_reboot'
            $state.partial_enable | Should -BeTrue
        } finally {
            Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'maps elevated child DISM failure with no progress to fail' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature {
            [pscustomobject]@{ State = 'Disabled' }
        }
        Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild {
            [pscustomobject]@{ status = 'fail'; error = 'dism failed' }
        }

        $envelope = Invoke-AhPhase2Provisioning -Fix -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'fail'
        @($envelope.steps)[0].id | Should -Be 'windows:feature-enable'
    }

    It 'round-trips state while preserving unknown fields' {
        $temp = Join-Path ([System.IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
        $statePath = Join-Path $temp 'setup-state.json'
        try {
            $state = New-AhWindowsHostState `
                -OperationId 'op-state' `
                -SelectedDistro 'Ubuntu' `
                -PendingRestart 'windows_reboot'
            $state['future_field'] = 'preserved'

            Write-AhSetupState -State $state -Path $statePath
            $read = Read-AhSetupState -Path $statePath

            $read.operation_id | Should -Be 'op-state'
            $read.pending_restart | Should -Be 'windows_reboot'
            $read.future_field | Should -Be 'preserved'
        } finally {
            Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    It 'dry-run does not call host command wrappers' {
        Mock -ModuleName AhProvisioning Get-AhWindowsOptionalFeature { throw 'should not probe features in dry-run' }
        Mock -ModuleName AhProvisioning Invoke-AhWsl { throw 'should not run wsl in dry-run' }
        Mock -ModuleName AhProvisioning Start-AhElevatedFeatureChild { throw 'should not elevate in dry-run' }

        $envelope = Invoke-AhPhase2Provisioning -DryRun -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'pass'
        Should -Invoke -ModuleName AhProvisioning Get-AhWindowsOptionalFeature -Times 0 -Exactly
        Should -Invoke -ModuleName AhProvisioning Invoke-AhWsl -Times 0 -Exactly
        Should -Invoke -ModuleName AhProvisioning Start-AhElevatedFeatureChild -Times 0 -Exactly
    }

    It 'keeps production orchestration behind wrapper functions' {
        $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..' '..')).Path
        $entrypoint = Get-Content -LiteralPath (Join-Path $RepoRoot 'scripts/windows/provision-ah-wsl.ps1') -Raw
        $entrypoint | Should -Not -Match '(?i)\bdism\.exe\b'
        $entrypoint | Should -Not -Match '(?i)\bwsl\.exe\b'

        $module = Get-Content -LiteralPath (Join-Path $RepoRoot 'scripts/windows/AhProvisioning.psm1') -Raw
        $module | Should -Match 'function Get-AhWindowsOptionalFeature'
        $module | Should -Match 'function Invoke-AhWsl'
        $module | Should -Match 'function Start-AhElevatedFeatureChild'
    }
}
