//! Linux daemon identity from systemd cgroups.

pub fn detect_current_service_unit() -> Option<String> {
    std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|cgroup| detect_current_service_unit_from_cgroup(&cgroup))
}

pub fn detect_current_scope_or_service() -> Option<String> {
    std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|cgroup| {
            cgroup
                .lines()
                .flat_map(|line| line.split('/'))
                .map(unescape_systemd_unit_segment)
                .filter(|segment| segment.ends_with(".scope") || segment.ends_with(".service"))
                .last()
        })
}

pub fn detect_current_service_unit_from_cgroup(cgroup: &str) -> Option<String> {
    cgroup
        .lines()
        .flat_map(|line| line.split('/'))
        .map(unescape_systemd_unit_segment)
        .filter(|segment| is_daemon_service_unit(segment))
        .last()
}

pub fn is_daemon_service_unit(unit: &str) -> bool {
    unit == "ahd.service" || (unit.starts_with("ah-") && unit.ends_with(".service"))
}

pub fn unescape_systemd_unit_segment(segment: &str) -> String {
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
