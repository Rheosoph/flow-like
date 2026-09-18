use sysinfo::{Components, Disks, Networks, System};

pub fn info() {
    let mut sys = System::new_all();
    sys.refresh_all();

    // RAM and swap information:
    tracing::debug!(bytes = sys.total_memory(), "total memory");
    tracing::debug!(bytes = sys.used_memory(), "used memory");
    tracing::debug!(bytes = sys.total_swap(), "total swap");
    tracing::debug!(bytes = sys.used_swap(), "used swap");

    // Display system information:
    tracing::debug!(name = ?System::name(), "System name");
    tracing::debug!(kernel_version = ?System::kernel_version(), "System kernel version");
    tracing::debug!(os_version = ?System::os_version(), "System OS version");
    tracing::debug!(host_name = ?System::host_name(), "System host name");

    // Number of CPUs:
    tracing::debug!(cpus = sys.cpus().len(), "NB CPUs");

    // Display processes ID, name na disk usage:
    for (pid, process) in sys.processes() {
        tracing::trace!(
            %pid,
            name = ?process.name(),
            disk_usage = ?process.disk_usage(),
            "process"
        );
    }

    // We display all disks' information:
    let disks = Disks::new_with_refreshed_list();
    for disk in &disks {
        tracing::trace!(?disk, "disk");
    }

    // Network interfaces name, total data received and total data transmitted:
    let networks = Networks::new_with_refreshed_list();
    for (interface_name, data) in &networks {
        tracing::trace!(
            interface = %interface_name,
            received_bytes = data.total_received(),
            transmitted_bytes = data.total_transmitted(),
            "network interface"
        );
        // If you want the amount of data received/transmitted since last call
        // to `Networks::refresh`, use `received`/`transmitted`.
    }

    // Components temperature:
    let components = Components::new_with_refreshed_list();
    for component in &components {
        tracing::trace!(?component, "component");
    }
}

pub fn get_ram() -> flow_like_types::Result<u64> {
    let mut sys = System::new_all();
    sys.refresh_all();
    Ok(sys.total_memory())
}

pub fn get_cores() -> flow_like_types::Result<u64> {
    let mut sys = System::new_all();
    sys.refresh_all();
    Ok(sys.cpus().len() as u64)
}
