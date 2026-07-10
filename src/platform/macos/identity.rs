//! macOS daemon identity compile skeleton.

pub fn detect_current_service_unit() -> Option<String> {
    None
}

pub fn detect_current_scope_or_service() -> Option<String> {
    None
}

pub fn detect_current_service_unit_from_cgroup(_cgroup: &str) -> Option<String> {
    None
}

pub fn is_daemon_service_unit(unit: &str) -> bool {
    unit == "ahd.service" || (unit.starts_with("ah-") && unit.ends_with(".service"))
}

pub fn unescape_systemd_unit_segment(segment: &str) -> String {
    segment.to_string()
}
