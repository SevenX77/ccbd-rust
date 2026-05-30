pub fn detect_current_service_unit() -> Option<String> {
    std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|cgroup| detect_current_service_unit_from_cgroup(&cgroup))
}

pub fn detect_current_service_unit_from_cgroup(cgroup: &str) -> Option<String> {
    cgroup
        .lines()
        .flat_map(|line| line.split('/'))
        .map(unescape_systemd_unit_segment)
        .filter(|segment| is_daemon_service_unit(segment))
        .last()
}

fn is_daemon_service_unit(unit: &str) -> bool {
    unit == "ahd.service" || (unit.starts_with("ah-") && unit.ends_with(".service"))
}

fn unescape_systemd_unit_segment(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = String::with_capacity(segment.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1] == b'x'
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 2..i + 4])
            && let Ok(value) = u8::from_str_radix(hex, 16)
        {
            out.push(value as char);
            i += 4;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::detect_current_service_unit_from_cgroup;

    #[test]
    fn detects_ccbd_service_with_user_manager_prefix() {
        let cgroup = "0::/user.slice/user-1001.slice/user@1001.service/app.slice/ahd.service";

        assert_eq!(
            detect_current_service_unit_from_cgroup(cgroup).as_deref(),
            Some("ahd.service")
        );
    }

    #[test]
    fn detects_ah_project_service_with_user_manager_prefix() {
        let cgroup = "0::/user.slice/user-1001.slice/user@1001.service/app.slice/ah-p1.service";

        assert_eq!(
            detect_current_service_unit_from_cgroup(cgroup).as_deref(),
            Some("ah-p1.service")
        );
    }

    #[test]
    fn ignores_scope_after_daemon_service() {
        let cgroup = "0::/user.slice/user-1001.slice/user@1001.service/app.slice/ahd.service/session-1.scope";

        assert_eq!(
            detect_current_service_unit_from_cgroup(cgroup).as_deref(),
            Some("ahd.service")
        );
    }

    #[test]
    fn user_manager_service_only_is_not_daemon_unit() {
        let cgroup = "0::/user.slice/user-1001.slice/user@1001.service";

        assert_eq!(detect_current_service_unit_from_cgroup(cgroup), None);
    }

    #[test]
    fn session_scope_and_slices_are_not_daemon_units() {
        for cgroup in [
            "0::/init.scope",
            "0::/user.slice/user-1001.slice",
            "0::/user.slice/user-1001.slice/user@1001.service/app.slice/app-org.gnome.Terminal.slice/vte-spawn.scope",
        ] {
            assert_eq!(detect_current_service_unit_from_cgroup(cgroup), None);
        }
    }

    #[test]
    fn unescapes_systemd_unit_segments_before_matching() {
        let cgroup = "0::/user.slice/user-1001.slice/user@1001.service/app.slice/ah\\x2dp1.service";

        assert_eq!(
            detect_current_service_unit_from_cgroup(cgroup).as_deref(),
            Some("ah-p1.service")
        );
    }
}
