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

        $envelope = Invoke-AhPhase2Provisioning -Check -SelectedDistro 'Ubuntu'

        $envelope.overall_status | Should -Be 'pass'
        @($envelope.steps).Count | Should -Be 2
        Should -Invoke -ModuleName AhProvisioning Get-AhWindowsOptionalFeature -Times 2 -Exactly
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
