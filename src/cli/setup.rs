use crate::cli::doctor::{DoctorCheck, system_binary_checks};
use crate::cli::prereq::{
    FixOwner, PrereqStatus, RuntimePrerequisite, runtime_prerequisite_from_doctor,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetupOptions {
    pub check: bool,
    pub fix: bool,
    pub yes: bool,
    pub json: bool,
    pub resume: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupOverallStatus {
    Pass,
    Warn,
    Fail,
    Fixed,
    NeedsWslShutdown,
    NeedsWindowsReboot,
    NeedsDistroReopen,
    NeedsDistroFirstLaunch,
    Unsupported,
    PermissionDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupPhase {
    Phase0Check,
    Phase1Distro,
    Phase2WindowsHost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextAction {
    pub kind: String,
    pub message: String,
    pub command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupEnvelope {
    pub schema_version: u32,
    pub operation_id: String,
    pub overall_status: SetupOverallStatus,
    pub phase: SetupPhase,
    pub selected_distro: Option<String>,
    pub next_action: Option<NextAction>,
    pub resume_command: Option<String>,
    pub steps: Vec<RuntimePrerequisite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupRun {
    pub envelope: SetupEnvelope,
    pub output: String,
    pub exit_code: i32,
}

pub fn run_setup(options: SetupOptions) -> Result<SetupRun, serde_json::Error> {
    let checks = collect_setup_doctor_checks();
    let selected_distro = std::env::var("WSL_DISTRO_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let envelope = build_setup_envelope(
        setup_prerequisites_from_checks(checks),
        selected_distro,
        uuid::Uuid::new_v4().to_string(),
        options,
    );
    let exit_code = exit_code_for_overall_status(envelope.overall_status);
    let output = if options.json {
        format!("{}\n", serde_json::to_string_pretty(&envelope)?)
    } else {
        render_setup_text(&envelope, options)
    };
    Ok(SetupRun {
        envelope,
        output,
        exit_code,
    })
}

pub fn collect_setup_doctor_checks() -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    checks.extend(system_binary_checks());
    checks.extend(crate::cli::wsl::wsl_onboarding_checks());
    checks
}

pub fn setup_prerequisites_from_checks(checks: Vec<DoctorCheck>) -> Vec<RuntimePrerequisite> {
    checks
        .into_iter()
        .map(runtime_prerequisite_from_doctor)
        .filter(|step| step.owner == FixOwner::AhRuntime)
        .collect()
}

pub fn build_setup_envelope(
    steps: Vec<RuntimePrerequisite>,
    selected_distro: Option<String>,
    operation_id: String,
    options: SetupOptions,
) -> SetupEnvelope {
    let overall_status = overall_status_for_steps(&steps);
    let next_action = next_action_for(overall_status, options);

    SetupEnvelope {
        schema_version: 1,
        operation_id,
        overall_status,
        phase: SetupPhase::Phase0Check,
        selected_distro,
        next_action,
        // next_action.command is the immediate command to run now; resume_command
        // is only populated after a boundary exit leaves persisted setup state.
        resume_command: None,
        steps,
    }
}

fn overall_status_for_steps(steps: &[RuntimePrerequisite]) -> SetupOverallStatus {
    if steps.iter().any(|step| step.status == PrereqStatus::Fail) {
        SetupOverallStatus::Fail
    } else if steps.iter().any(|step| step.status == PrereqStatus::Warn) {
        SetupOverallStatus::Warn
    } else {
        SetupOverallStatus::Pass
    }
}

fn next_action_for(
    overall_status: SetupOverallStatus,
    options: SetupOptions,
) -> Option<NextAction> {
    match overall_status {
        SetupOverallStatus::Fail if options.fix => Some(NextAction {
            kind: "phase1_required".to_string(),
            message: "ah setup --fix was requested, but Phase 0 is read-only; no changes were applied."
                .to_string(),
            command: Some("ah setup --check".to_string()),
        }),
        SetupOverallStatus::Fail => Some(NextAction {
            kind: "run_fix".to_string(),
            message: "Run ah setup --fix after Phase 1 support lands, or apply the listed manual guidance."
                .to_string(),
            command: Some("ah setup --fix".to_string()),
        }),
        SetupOverallStatus::Warn => Some(NextAction {
            kind: "review_warnings".to_string(),
            message: "Review warnings; Phase 0 did not change machine state.".to_string(),
            command: Some("ah setup --check".to_string()),
        }),
        _ => None,
    }
}

pub fn exit_code_for_overall_status(status: SetupOverallStatus) -> i32 {
    match status {
        SetupOverallStatus::Pass | SetupOverallStatus::Warn | SetupOverallStatus::Fixed => 0,
        SetupOverallStatus::Fail | SetupOverallStatus::PermissionDenied => 1,
        SetupOverallStatus::Unsupported => 2,
        SetupOverallStatus::NeedsWslShutdown => 10,
        SetupOverallStatus::NeedsWindowsReboot => 11,
        SetupOverallStatus::NeedsDistroReopen | SetupOverallStatus::NeedsDistroFirstLaunch => 12,
    }
}

pub fn render_setup_text(envelope: &SetupEnvelope, options: SetupOptions) -> String {
    let mut out = String::new();
    out.push_str("ah setup plan (read-only)\n");
    if options.fix {
        out.push_str("Phase 0 accepts --fix for CLI compatibility, but does not mutate machine state.\n");
    } else if options.check {
        out.push_str("Check mode: no fixes were applied.\n");
    } else {
        out.push_str("Default mode: no fixes were applied. Use --fix in Phase 1 to apply supported fixes.\n");
    }
    if options.yes {
        out.push_str("--yes has no effect in Phase 0 because no mutation is implemented.\n");
    }
    if options.resume {
        out.push_str("--resume found no Phase 0 boundary state; probes were re-run from current machine state.\n");
    }
    out.push_str(&format!("overall_status: {:?}\n", envelope.overall_status));

    for step in &envelope.steps {
        out.push_str(&format!("- {}: {:?} - {}", step.id, step.status, step.detail));
        if let Some(suggestion) = &step.suggestion {
            out.push_str(&format!("\n  suggestion: {suggestion}"));
        }
        out.push('\n');
    }

    if let Some(action) = &envelope.next_action {
        out.push_str(&format!("next_action: {}\n", action.message));
        if let Some(command) = &action.command {
            out.push_str(&format!("next_command: {command}\n"));
        }
    }
    out.push_str("status_check: ah setup --check\n");
    out
}

#[cfg(test)]
mod tests {
    use super::{
        SetupOptions, SetupOverallStatus, build_setup_envelope, exit_code_for_overall_status,
        setup_prerequisites_from_checks,
    };
    use crate::cli::doctor::{DoctorCheck, DoctorStatus};
    use crate::cli::prereq::{FixOwner, PrereqStatus};
    use crate::cli::wsl::{
        SystemdUserStatus, TmuxCheck, WslDetection, wsl_onboarding_checks_from,
    };

    #[test]
    fn setup_filters_to_ah_runtime_prerequisites() {
        let steps = setup_prerequisites_from_checks(vec![
            DoctorCheck {
                name: "wsl:tmux".to_string(),
                status: DoctorStatus::Fail,
                detail: "not found".to_string(),
                suggestion: Some("install tmux".to_string()),
            },
            DoctorCheck {
                name: "provider:claude".to_string(),
                status: DoctorStatus::Warn,
                detail: "missing auth".to_string(),
                suggestion: None,
            },
            DoctorCheck {
                name: "daemon".to_string(),
                status: DoctorStatus::Fail,
                detail: "not running".to_string(),
                suggestion: None,
            },
        ]);

        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].id, "wsl:tmux");
        assert_eq!(steps[0].owner, FixOwner::AhRuntime);
        assert_eq!(steps[0].status, PrereqStatus::Fail);
    }

    #[test]
    fn json_envelope_has_phase0_contract_fields() {
        let steps = setup_prerequisites_from_checks(vec![DoctorCheck {
            name: "wsl:systemd-user".to_string(),
            status: DoctorStatus::Fail,
            detail: "not running".to_string(),
            suggestion: Some("enable systemd".to_string()),
        }]);
        let envelope = build_setup_envelope(
            steps,
            Some("Ubuntu".to_string()),
            "op-test".to_string(),
            SetupOptions {
                check: true,
                json: true,
                ..SetupOptions::default()
            },
        );
        let value = serde_json::to_value(&envelope).unwrap();

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["operation_id"], "op-test");
        assert_eq!(value["overall_status"], "fail");
        assert_eq!(value["phase"], "phase0_check");
        assert_eq!(value["selected_distro"], "Ubuntu");
        assert_eq!(value["resume_command"], serde_json::Value::Null);
        assert!(value["next_action"]["command"].as_str().is_some());
        assert_eq!(value["steps"][0]["id"], "wsl:systemd-user");
    }

    #[test]
    fn setup_status_matches_doctor_for_injected_wsl_checks() {
        let doctor_checks = wsl_onboarding_checks_from(
            WslDetection {
                is_wsl: true,
                source: Some("env"),
                container_hint: false,
            },
            SystemdUserStatus::MissingTool,
            TmuxCheck {
                available: false,
                detail: "not found in PATH".to_string(),
            },
        );
        let systemd_status = doctor_checks
            .iter()
            .find(|check| check.name == "wsl:systemd-user")
            .map(|check| check.status)
            .unwrap();
        let tmux_status = doctor_checks
            .iter()
            .find(|check| check.name == "wsl:tmux")
            .map(|check| check.status)
            .unwrap();

        let setup_steps = setup_prerequisites_from_checks(doctor_checks);

        assert_eq!(systemd_status, DoctorStatus::Fail);
        assert_eq!(tmux_status, DoctorStatus::Fail);
        assert_eq!(
            setup_steps
                .iter()
                .find(|step| step.id == "wsl:systemd-user")
                .map(|step| step.status),
            Some(PrereqStatus::Fail)
        );
        assert_eq!(
            setup_steps
                .iter()
                .find(|step| step.id == "wsl:tmux")
                .map(|step| step.status),
            Some(PrereqStatus::Fail)
        );
    }

    #[test]
    fn fix_placeholder_is_read_only_and_machine_distinguishable() {
        let steps = setup_prerequisites_from_checks(vec![DoctorCheck {
            name: "binary:tmux".to_string(),
            status: DoctorStatus::Fail,
            detail: "not found in PATH".to_string(),
            suggestion: Some("install tmux".to_string()),
        }]);
        let envelope = build_setup_envelope(
            steps,
            None,
            "op-test".to_string(),
            SetupOptions {
                fix: true,
                ..SetupOptions::default()
            },
        );

        assert_eq!(envelope.overall_status, SetupOverallStatus::Fail);
        assert_eq!(exit_code_for_overall_status(envelope.overall_status), 1);
        assert_eq!(
            envelope.next_action.as_ref().map(|action| action.kind.as_str()),
            Some("phase1_required")
        );
        assert_eq!(envelope.resume_command, None);
    }
}
