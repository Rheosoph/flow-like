use std::{collections::BTreeMap, path::Path};

/// Cgroup v2 block I/O counts all descendant processes. Missing accounting is
/// unavailable; an existing empty io.stat means no block-device I/O yet.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_cgroup_io(value: &str) -> Option<(u64, u64)> {
    if value.len() > 64 * 1024 {
        return None;
    }
    let mut total = (0u64, 0u64);
    let mut devices = std::collections::HashSet::new();
    for line in value.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let device = fields.next()?;
        let (major, minor) = device.split_once(':')?;
        major.parse::<u32>().ok()?;
        minor.parse::<u32>().ok()?;
        if !devices.insert(device) {
            return None;
        }
        let mut read = None;
        let mut written = None;
        for field in fields {
            let (name, number) = field.split_once('=')?;
            let number = number.parse::<u64>().ok()?;
            match name {
                "rbytes" if read.is_none() => read = Some(number),
                "wbytes" if written.is_none() => written = Some(number),
                "rbytes" | "wbytes" => return None,
                _ => (),
            }
        }
        total.0 = total.0.checked_add(read?)?;
        total.1 = total.1.checked_add(written?)?;
    }
    Some(total)
}

#[derive(Default)]
pub(crate) struct NetworkWindow {
    previous: BTreeMap<String, (u64, u64)>,
}

impl NetworkWindow {
    pub(crate) fn observe(&mut self, values: BTreeMap<String, (u64, u64)>) -> serde_json::Value {
        let counters = values
            .iter()
            .try_fold((0u64, 0u64), |sum, (name, current)| {
                let old = self.previous.get(name)?;
                Some((
                    sum.0.checked_add(current.0.checked_sub(old.0)?)?,
                    sum.1.checked_add(current.1.checked_sub(old.1)?)?,
                ))
            })
            .filter(|_| !values.is_empty());
        self.previous = values;
        serde_json::json!({"basis":"all_host_interfaces", "interfaces":self.previous.len(),
            "received_bytes":counters.map(|v|v.0), "transmitted_bytes":counters.map(|v|v.1)})
    }
}

pub(crate) fn volume_space(path: &Path) -> Option<serde_json::Value> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut value = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
            return None;
        }
        let value = unsafe { value.assume_init() };
        let block = u64::try_from(value.f_frsize).ok()?;
        Some(
            serde_json::json!({"basis":"agent_state_volume", "total_bytes":u64::try_from(value.f_blocks).ok()?.checked_mul(block)?,
            "available_bytes":u64::try_from(value.f_bavail).ok()?.checked_mul(block)?}),
        )
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn block_io_rejects_ambiguous_overflowing_and_partial_accounting() {
        assert_eq!(
            parse_cgroup_io("8:0 rbytes=10 wbytes=20 rios=1\n8:16 wbytes=4 rbytes=3\n"),
            Some((13, 24))
        );
        assert_eq!(parse_cgroup_io(""), Some((0, 0)));
        for value in [
            "8:0 rbytes=1",
            "8:0 rbytes=1 rbytes=2 wbytes=0",
            "8:0 rbytes=1 wbytes=0\n8:0 rbytes=2 wbytes=0",
            "8:0 rbytes=18446744073709551615 wbytes=0\n8:1 rbytes=1 wbytes=0",
            "invalid rbytes=0 wbytes=0",
        ] {
            assert_eq!(parse_cgroup_io(value), None);
        }
    }
    #[test]
    fn network_restart_and_new_interfaces_do_not_fabricate_deltas() {
        let mut window = NetworkWindow::default();
        let values = |rx, tx| BTreeMap::from([("ethernet".to_owned(), (rx, tx))]);
        assert!(window.observe(values(100, 200))["received_bytes"].is_null());
        assert_eq!(window.observe(values(115, 203))["received_bytes"], 15);
        assert!(window.observe(values(1, 2))["received_bytes"].is_null());
        let mut changed = values(2, 3);
        changed.insert("new".into(), (50, 50));
        assert!(window.observe(changed)["transmitted_bytes"].is_null());
        assert!(window.observe(BTreeMap::new())["received_bytes"].is_null());
    }
    #[test]
    fn volume_metric_contains_capacity_without_local_paths() {
        let directory = tempfile::tempdir().unwrap();
        let value = volume_space(directory.path()).unwrap();
        assert!(value["total_bytes"].as_u64().unwrap() > 0);
        assert!(
            !value
                .to_string()
                .contains(directory.path().to_str().unwrap())
        );
        assert!(volume_space(&directory.path().join("absent")).is_none());
    }
}
