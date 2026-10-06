use super::{Capabilities, PlacementResources};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

const QUOTA_TYPE: i32 = 2; // Linux PRJQUOTA; QCMD shifts the operation by eight.
const GET_XATTR: libc::c_ulong = 0x801c_581f;
const PROJECT_INHERIT: u32 = 0x200;

#[repr(C)]
#[derive(Default)]
struct FsXattr {
    flags: u32,
    extent_size: u32,
    extents: u32,
    project: u32,
    cow_extent_size: u32,
    padding: [u8; 8],
}

fn landlock_abi() -> Result<i32> {
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<u8>(),
            0,
            1,
        )
    };
    ensure!(
        abi >= 6,
        "Linux sandbox requires Landlock ABI 6 (Linux 6.12+, e.g. Ubuntu 24.04 HWE); kernel/LSM support is unavailable"
    );
    Ok(abi as i32)
}

fn bubblewrap() -> Result<PathBuf> {
    let path = PathBuf::from("/usr/bin/bwrap");
    let metadata = std::fs::symlink_metadata(&path)
        .context("Install the distribution's bubblewrap package")?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == 0
            && metadata.mode() & 0o022 == 0,
        "Bubblewrap must be a root-owned, non-writable /usr/bin/bwrap executable"
    );
    Ok(path)
}

fn read(path: &Path) -> Result<String> {
    read_bounded(path, 16 * 1024)
}

fn read_bounded(path: &Path, limit: u64) -> Result<String> {
    let mut text = String::new();
    File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut text)?;
    ensure!(text.len() as u64 <= limit, "Oversized kernel control value");
    Ok(text)
}

fn cgroup_root(state_dir: &Path) -> Result<PathBuf> {
    let root = super::host_configuration(state_dir)?.cgroup_root.context(
        "Set FLOW_LIKE_DEVICE_CGROUP_ROOT to this agent's dedicated, delegated cgroup v2 directory",
    )?;
    ensure!(
        root.is_absolute() && root.canonicalize()? == root,
        "Cgroup root must be canonical and absolute"
    );
    let file = File::open(&root)?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    ensure!(
        unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } == 0
            && unsafe { stat.assume_init() }.f_type == 0x6367_7270,
        "Resource enforcement requires a cgroup v2 filesystem"
    );
    let enabled = read(&root.join("cgroup.subtree_control"))?;
    ensure!(
        ["cpu", "memory", "pids"]
            .iter()
            .all(|name| enabled.split_whitespace().any(|value| value == *name)),
        "Delegate and enable the cpu, memory, and pids cgroup v2 controllers"
    );
    ensure!(
        read(&root.join("cgroup.procs"))?.trim().is_empty(),
        "The delegated workload cgroup must not contain the agent itself"
    );
    capacity(&root)?;
    Ok(root)
}

fn cpu(value: &str) -> Result<u64> {
    let mut parts = value.split_whitespace();
    let quota: u64 = parts
        .next()
        .context("Missing CPU quota")?
        .parse()
        .context("CPU admission requires a finite cpu.max")?;
    let period: u64 = parts.next().context("Missing CPU period")?.parse()?;
    ensure!(
        quota > 0 && period > 0 && parts.next().is_none(),
        "Invalid cgroup CPU budget"
    );
    Ok(quota.checked_mul(1000).context("CPU budget overflow")? / period)
}

fn capacity(root: &Path) -> Result<(u64, u64, u64)> {
    let cpu = cpu(&read(&root.join("cpu.max"))?)?;
    let memory = read(&root.join("memory.max"))?
        .trim()
        .parse()
        .context("Admission requires a finite parent memory.max")?;
    let pids = read(&root.join("pids.max"))?
        .trim()
        .parse()
        .context("Admission requires a finite parent pids.max")?;
    Ok((cpu, memory, pids))
}

pub(super) fn capabilities(state_dir: &Path) -> Capabilities {
    let abi = landlock_abi();
    let bwrap = bubblewrap();
    let cgroup = cgroup_root(state_dir);
    let reason = abi
        .as_ref()
        .err()
        .or(bwrap.as_ref().err())
        .or(cgroup.as_ref().err())
        .map(ToString::to_string);
    Capabilities {
        platform: "linux",
        landlock_abi: abi.ok(),
        bubblewrap: bwrap.ok(),
        cgroup_root: cgroup.ok(),
        sandbox_available: reason.is_none(),
        require_isolation: false,
        placement_preflight_required: true,
        network_boundary: super::NETWORK_BOUNDARY,
        reason,
        disk_requirement: "ext4 mounted prjquota; distinct inherited project ID; finite enforced byte and inode hard quotas",
    }
}

fn mount_field(value: &str) -> Result<PathBuf> {
    let mut result = Vec::new();
    let bytes = value.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'\\' {
            ensure!(offset + 3 < bytes.len(), "Invalid mountinfo escape");
            let digits = &bytes[offset + 1..offset + 4];
            ensure!(
                digits[0] <= b'3' && digits.iter().all(|v| (b'0'..=b'7').contains(v)),
                "Invalid mountinfo escape"
            );
            result.push((digits[0] - b'0') * 64 + (digits[1] - b'0') * 8 + digits[2] - b'0');
            offset += 4;
        } else {
            result.push(bytes[offset]);
            offset += 1;
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(result).into())
}

fn project_attribute(path: &Path) -> Result<(File, FsXattr)> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Quota tree cannot contain symlinks, sockets, or special files during admission"
    );
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Quota inode changed during admission"
    );
    let mut attributes = FsXattr::default();
    ensure!(
        unsafe { libc::ioctl(file.as_raw_fd(), GET_XATTR, &mut attributes) } == 0,
        "Filesystem does not expose project-quota membership for {}",
        path.display()
    );
    Ok((file, attributes))
}

struct ProjectQuota {
    root: File,
    project: u32,
    inode_limit: u64,
}

/// Checks the mount, the root's inherited project and its enforced limits. This
/// is O(1); `verify_quota_tree` inspects existing inodes once per data version.
fn quota_root(data: &Path, limit: u64) -> Result<ProjectQuota> {
    let canonical = data.canonicalize()?;
    ensure!(
        canonical == data,
        "Quota data root must be canonical and contain no symlink aliases"
    );
    let mut mountinfo = String::new();
    File::open("/proc/self/mountinfo")?
        .take(4 * 1024 * 1024 + 1)
        .read_to_string(&mut mountinfo)?;
    ensure!(
        mountinfo.len() <= 4 * 1024 * 1024,
        "Mount inventory is too large"
    );
    let mut selected = None;
    for line in mountinfo.lines() {
        let Some((before, after)) = line.split_once(" - ") else {
            continue;
        };
        let left: Vec<_> = before.split_whitespace().collect();
        let right: Vec<_> = after.split_whitespace().collect();
        if left.len() < 6 || right.len() < 3 {
            continue;
        }
        let path = mount_field(left[4])?;
        if data.starts_with(&path)
            && selected
                .as_ref()
                .is_none_or(|(old, _, _): &(PathBuf, &str, &str)| {
                    path.as_os_str().len() > old.as_os_str().len()
                })
        {
            selected = Some((path, right[0], right[2]));
        }
    }
    let (_, filesystem, options) = selected.context("Data mount was not found")?;
    ensure!(
        filesystem == "ext4"
            && options.split(',').any(|v| v == "prjquota")
            && !options.split(',').any(|v| v.contains("noenforce")),
        "Hard disk isolation requires an ext4 mount with enforced prjquota"
    );
    let (file, attributes) = project_attribute(data)?;
    ensure!(
        attributes.project > 0 && attributes.flags & PROJECT_INHERIT != 0,
        "Provision a nonzero inherited ext4 project quota on the placement data root"
    );
    let project = attributes.project;
    let mut quota = std::mem::MaybeUninit::<libc::dqblk>::zeroed();
    ensure!(
        unsafe {
            libc::syscall(
                libc::SYS_quotactl_fd,
                file.as_raw_fd(),
                (libc::Q_GETQUOTA << 8) | QUOTA_TYPE,
                project,
                quota.as_mut_ptr(),
            )
        } == 0,
        "Cannot verify enforced project quota; grant the agent read access to project quota information"
    );
    let quota = unsafe { quota.assume_init() };
    let hard_bytes = quota
        .dqb_bhardlimit
        .checked_mul(1024)
        .context("Disk quota overflow")?;
    ensure!(
        hard_bytes > 0
            && hard_bytes <= limit
            && quota.dqb_ihardlimit > 0
            && quota.dqb_ihardlimit <= limit / 4096,
        "Project quota must enforce at most the requested bytes and at most disk_bytes/4096 inodes"
    );
    ensure!(
        quota.dqb_curspace <= hard_bytes && quota.dqb_curinodes <= quota.dqb_ihardlimit,
        "Placement already exceeds its hard disk quota"
    );
    Ok(ProjectQuota {
        root: file,
        project,
        inode_limit: quota.dqb_ihardlimit,
    })
}

fn inspection_denied(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
}

/// Existing files may predate inheritance. Verify every regular file and
/// directory before granting write access, with bounded memory and a finite
/// number of metadata reads. Symlinks, FIFOs, sockets and device nodes cannot
/// be opened for the project ioctl; the kernel gives them the inherited project
/// of the verified directory that holds them, and they cannot be re-projected.
fn verify_quota_tree(data: &Path, quota: ProjectQuota) -> Result<()> {
    let ProjectQuota {
        root,
        project,
        inode_limit,
    } = quota;
    let mut visited = 0_u64;
    let mut uninspected = 0_u64;
    let entries = std::fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd()))?;
    let mut stack = vec![(root, entries)];
    while let Some((_, directory)) = stack.last_mut() {
        let Some(entry) = directory.next() else {
            stack.pop();
            continue;
        };
        let entry = entry?;
        visited += 1;
        ensure!(
            visited <= inode_limit.min(1_000_000),
            "Quota admission tree exceeds its inode inspection bound"
        );
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                uninspected += 1;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() && !metadata.is_dir() {
            continue;
        }
        let (file, attribute) = match project_attribute(&path) {
            Ok(value) => value,
            Err(error) if inspection_denied(&error) => {
                uninspected += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        ensure!(
            attribute.project == project
                && (!metadata.is_dir() || attribute.flags & PROJECT_INHERIT != 0),
            "Existing placement inode escapes its inherited project quota"
        );
        if metadata.is_dir() {
            ensure!(stack.len() < 64, "Quota tree exceeds 64 directory levels");
            // Keep the verified directory open while walking. A worker cannot
            // redirect admission through a symlink after we checked an inode.
            let entries = std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
            stack.push((file, entries));
        }
    }
    if uninspected > 0 {
        tracing::warn!(
            data = %data.display(),
            uninspected,
            "Quota admission skipped entries the agent cannot read; they keep the project inherited at creation"
        );
    }
    Ok(())
}

/// The managed project root holds every revision and the online cache, each
/// with its own placements' secrets, so a sandbox may only mount one of them.
fn ensure_single_project_tree(project: &Path, state: &Path) -> Result<()> {
    let managed = state.join("projects");
    ensure!(
        project != managed && project.parent() != Some(managed.as_path()),
        "Sandboxed placements must run an imported project revision; {} also contains other placements' secrets",
        project.display()
    );
    Ok(())
}

/// Admission runs once per placement data version, before the supervisor spawns
/// any replica of that version, so each spawn only repeats the O(1) root checks.
pub(super) fn preflight(
    resources: &PlacementResources,
    state_dir: &Path,
    project: &Path,
    data: &Path,
) -> Result<()> {
    resources.validate()?;
    landlock_abi()?;
    bubblewrap()?;
    cgroup_root(state_dir)?;
    ensure_single_project_tree(&project.canonicalize()?, &state_dir.canonicalize()?)?;
    let quota = quota_root(data, resources.disk_bytes.context("Missing disk bound")?)?;
    verify_quota_tree(data, quota)
}

/// A private directory the sandbox can always mask or bind, so secrets published
/// after spawn stay hidden. A project the agent cannot write can never receive
/// agent-published secrets and needs no mask.
fn secrets_directory(parent: &Path, name: &str) -> Result<Option<PathBuf>> {
    crate::config::ensure_unaliased_child(parent, name)?;
    let path = parent.join(name);
    match std::fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => File::open(parent)?.sync_all()?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Create sandbox secrets directory {}", path.display()));
        }
    }
    ensure!(
        path.canonicalize()? == path && std::fs::symlink_metadata(&path)?.is_dir(),
        "Invalid sandbox secrets directory {}",
        path.display()
    );
    Ok(Some(path))
}

pub(super) fn sample(
    pid: u32,
    placement: &str,
    slot: u8,
    state_dir: &Path,
) -> Result<super::ResourceSample> {
    let membership = read(&PathBuf::from(format!("/proc/{pid}/cgroup")))?;
    let path = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .context("Workload has no cgroup v2 membership")?;
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid cgroup name")?;
    let suffix = format!(
        "-{}-{slot}",
        &blake3::hash(placement.as_bytes()).to_hex()[..24]
    );
    ensure!(
        name.strip_prefix('p')
            .and_then(|name| name.strip_suffix(&suffix))
            .and_then(|id| id.parse::<u32>().ok())
            .is_some_and(|id| id > 0),
        "Workload cgroup does not match its placement"
    );
    let root = cgroup_root(state_dir)?.join(name);
    ensure!(
        read_bounded(&root.join("cgroup.procs"), 1024 * 1024)?
            .split_whitespace()
            .any(|value| value.parse::<u32>().ok() == Some(pid)),
        "Workload left its placement cgroup"
    );
    let usage = read(&root.join("cpu.stat"))?
        .lines()
        .find_map(|line| {
            line.strip_prefix("usage_usec ")
                .and_then(|v| v.parse::<u64>().ok())
        })
        .context("Missing cgroup CPU accounting")?;
    Ok(super::ResourceSample {
        generation: root.metadata()?.ino(),
        cpu_microseconds: usage,
        memory_bytes: read(&root.join("memory.current"))?.trim().parse()?,
        processes_and_threads: read(&root.join("pids.current"))?.trim().parse()?,
        io_bytes: read(&root.join("io.stat"))
            .ok()
            .and_then(|value| crate::telemetry::resources::parse_cgroup_io(&value)),
    })
}

pub(super) fn disk_sample(data: &Path) -> Result<super::DiskSample> {
    ensure!(
        data.canonicalize()? == data,
        "Disk accounting root contains a symlink alias"
    );
    let (file, attributes) = project_attribute(data)?;
    ensure!(
        attributes.project > 0 && attributes.flags & PROJECT_INHERIT != 0,
        "Placement disk accounting requires an inherited project quota"
    );
    let mut value = std::mem::MaybeUninit::<libc::dqblk>::zeroed();
    ensure!(
        unsafe {
            libc::syscall(
                libc::SYS_quotactl_fd,
                file.as_raw_fd(),
                (libc::Q_GETQUOTA << 8) | QUOTA_TYPE,
                attributes.project,
                value.as_mut_ptr(),
            )
        } == 0,
        "Project disk usage is unavailable"
    );
    let value = unsafe { value.assume_init() };
    let limit_bytes = value
        .dqb_bhardlimit
        .checked_mul(1024)
        .context("Disk accounting overflow")?;
    ensure!(
        limit_bytes > 0 && value.dqb_ihardlimit > 0,
        "Project disk quota has no hard limits"
    );
    Ok(super::DiskSample {
        used_bytes: value.dqb_curspace,
        limit_bytes,
        used_inodes: value.dqb_curinodes,
        limit_inodes: value.dqb_ihardlimit,
    })
}

pub(super) struct Sandbox {
    cgroup: Cgroup,
    program: File,
    scopes: File,
    preserve_listener: bool,
}
struct Cgroup {
    root: PathBuf,
    procs: File,
}

impl Cgroup {
    fn new(
        resources: &PlacementResources,
        state_dir: &Path,
        project: u32,
        placement: &str,
        slot: u8,
    ) -> Result<Self> {
        let root = cgroup_root(state_dir)?;
        let maximum = capacity(&root)?;
        let identity = blake3::hash(placement.as_bytes()).to_hex().to_string();
        let name = format!("p{project}-{}-{slot}", &identity[..24]);
        let path = root.join(&name);
        if path.try_exists()?
            && read(&path.join("cgroup.events"))?
                .lines()
                .any(|line| line == "populated 0")
        {
            // A killed worker may outlive the previous lease's Drop by a few
            // scheduler ticks. Reclaim only our own empty slot on its next start.
            std::fs::remove_dir(&path).context("Remove empty previous placement cgroup")?;
        }
        let mut used = (0_u64, 0_u64, 0_u64);
        let mut children = 0;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            children += 1;
            ensure!(
                children <= 1024,
                "Cgroup admission inventory exceeds 1024 placements"
            );
            let entry_name = entry.file_name().to_string_lossy().into_owned();
            ensure!(
                !entry_name.starts_with(&format!("p{project}-"))
                    || entry_name.starts_with(&format!("p{project}-{}-", &identity[..24])),
                "Different placements cannot share a disk project quota"
            );
            let limits = capacity(&entry.path())?;
            used.0 = used
                .0
                .checked_add(limits.0)
                .context("CPU admission overflow")?;
            used.1 = used
                .1
                .checked_add(limits.1)
                .context("Memory admission overflow")?;
            used.2 = used
                .2
                .checked_add(limits.2)
                .context("Process admission overflow")?;
        }
        let requested = (
            u64::from(resources.cpu_millis.unwrap()),
            resources.memory_bytes.unwrap(),
            u64::from(resources.max_processes.unwrap()),
        );
        ensure!(
            used.0.saturating_add(requested.0) <= maximum.0
                && used.1.saturating_add(requested.1) <= maximum.1
                && used.2.saturating_add(requested.2) <= maximum.2,
            "Placement exceeds the agent's remaining CPU, memory, or process admission budget"
        );
        std::fs::create_dir(&path).context(
            "Create exclusive placement cgroup (remove stale empty groups before restarting)",
        )?;
        let result = (|| -> Result<Self> {
            for (name, value) in [
                ("cpu.max", format!("{} 1000000", requested.0 * 1000)),
                ("memory.max", requested.1.to_string()),
                ("memory.swap.max", "0".into()),
                ("memory.oom.group", "1".into()),
                ("pids.max", requested.2.to_string()),
            ] {
                std::fs::write(path.join(name), &value)?;
                ensure!(
                    read(&path.join(name))?.trim() == value,
                    "Kernel did not accept {name}"
                );
            }
            let procs = OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(path.join("cgroup.procs"))?;
            Ok(Self {
                root: path.clone(),
                procs,
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir(&path);
        }
        result
    }
    fn signal(&self, force: bool) -> Result<()> {
        if force {
            std::fs::write(self.root.join("cgroup.kill"), "1")?;
            return Ok(());
        }
        let own_status = read(Path::new("/proc/self/status"))?;
        let own_depth = own_status
            .lines()
            .find_map(|line| line.strip_prefix("NSpid:"))
            .context("Cannot identify agent PID namespace")?
            .split_whitespace()
            .count();
        for pid in read_bounded(&self.root.join("cgroup.procs"), 1024 * 1024)?.split_whitespace() {
            let pid: libc::pid_t = pid.parse()?;
            let status = match read(&PathBuf::from(format!("/proc/{pid}/status"))) {
                Ok(value) => value,
                Err(_) => continue,
            };
            // Leave bubblewrap's host monitor and namespace init alive while
            // the workload and its descendants finish their graceful drain.
            let namespaced = status.lines().find_map(|line| line.strip_prefix("NSpid:"));
            if namespaced.is_some_and(|value| {
                value.split_whitespace().count() > own_depth
                    && value.split_whitespace().last() != Some("1")
            }) {
                let result = unsafe { libc::kill(pid, libc::SIGTERM) };
                ensure!(
                    result == 0
                        || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH),
                    "Cannot signal isolated workload"
                );
            }
        }
        Ok(())
    }
}
impl Drop for Cgroup {
    fn drop(&mut self) {
        let _ = std::fs::write(self.root.join("cgroup.kill"), "1");
        let _ = std::fs::remove_dir(&self.root);
    }
}

fn filter() -> Vec<libc::sock_filter> {
    let statement = |code, k| libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    };
    let jump = |k, jt, jf| libc::sock_filter {
        code: 0x15,
        jt,
        jf,
        k,
    };
    let deny = statement(0x06, 0x0005_0000 | libc::EPERM as u32);
    #[cfg(target_arch = "x86_64")]
    let architecture = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    let architecture = 0xc000_00b7;
    let mut program = vec![
        statement(0x20, 4),
        jump(architecture, 1, 0),
        statement(0x06, 0x8000_0000),
        statement(0x20, 0),
    ];
    #[cfg(target_arch = "x86_64")]
    program.extend([
        libc::sock_filter {
            code: 0x45,
            jt: 0,
            jf: 1,
            k: 0x4000_0000,
        },
        statement(0x06, 0x8000_0000),
    ]);
    // clone3's opaque argument structure cannot be filtered safely. Returning
    // ENOSYS lets glibc use clone for ordinary threads without new namespaces.
    program.extend([
        jump(libc::SYS_clone3 as u32, 0, 1),
        statement(0x06, 0x0005_0000 | libc::ENOSYS as u32),
    ]);
    program.extend([
        jump(libc::SYS_clone as u32, 0, 4),
        statement(0x20, 16),
        libc::sock_filter {
            code: 0x45,
            jt: 0,
            jf: 1,
            k: 0x7e02_0000,
        },
        deny,
        statement(0x20, 0),
    ]);
    for number in [
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_pidfd_getfd,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_open_by_handle_at,
        libc::SYS_name_to_handle_at,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_userfaultfd,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_quotactl,
        libc::SYS_quotactl_fd,
        libc::SYS_reboot,
        libc::SYS_kexec_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_move_mount,
        libc::SYS_open_tree,
        libc::SYS_mount_setattr,
    ] {
        program.extend([jump(number as u32, 0, 1), deny]);
    }
    // Prevent quota-inheritance changes and terminal injection. These ioctl
    // requests do not need to inspect pointed-to user memory.
    program.extend([
        jump(libc::SYS_ioctl as u32, 0, 10),
        statement(0x20, 24),
        jump(0x401c_5820, 0, 1),
        deny,
        jump(0x4008_6602, 0, 1),
        deny,
        jump(0x4004_6602, 0, 1),
        deny,
        jump(0x5412, 0, 1),
        deny,
        statement(0x20, 0),
    ]);
    program.push(statement(0x06, 0x7fff_0000));
    program
}

fn scope_rules() -> Result<File> {
    #[repr(C)]
    struct Rules {
        fs: u64,
        net: u64,
        scopes: u64,
    }
    let rules = Rules {
        fs: 0,
        net: 0,
        scopes: 3,
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &rules,
            std::mem::size_of::<Rules>(),
            0,
        )
    };
    ensure!(
        fd >= 0,
        "Cannot create mandatory Landlock IPC/signal scopes"
    );
    Ok(unsafe { File::from_raw_fd(fd as i32) })
}

fn filter_file() -> Result<File> {
    policy_file(&filter())
}

/// A sealed memory file with the seccomp program, which bubblewrap reads by descriptor.
fn policy_file(filters: &[libc::sock_filter]) -> Result<File> {
    let name = CString::new("flow-like-worker-seccomp")?;
    let fd = unsafe {
        libc::syscall(
            libc::SYS_memfd_create,
            name.as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    ensure!(fd >= 0, "Cannot create sandbox seccomp policy");
    let mut file = unsafe { File::from_raw_fd(fd as i32) };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            filters.as_ptr().cast::<u8>(),
            filters.len() * std::mem::size_of::<libc::sock_filter>(),
        )
    };
    file.write_all(bytes)?;
    file.seek(SeekFrom::Start(0))?;
    ensure!(
        unsafe {
            libc::fcntl(
                fd as i32,
                libc::F_ADD_SEALS,
                libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE,
            )
        } == 0,
        "Cannot seal sandbox policy"
    );
    Ok(file)
}

pub(super) fn command(
    config: &crate::config::PlacementConfig,
    state: &Path,
    data: &Path,
    executable: &Path,
    slot: u8,
) -> Result<(tokio::process::Command, Sandbox)> {
    let resources = config
        .resources
        .as_ref()
        .context("Missing sandbox limits")?;
    resources.validate()?;
    landlock_abi()?;
    let project_id = quota_root(data, resources.disk_bytes.context("Missing disk bound")?)?.project;
    let project = config.project_path.canonicalize()?;
    let state = state.canonicalize()?;
    ensure_single_project_tree(&project, &state)?;
    ensure!(
        !["/usr", "/bin", "/lib", "/lib64", "/etc/ssl/certs"]
            .iter()
            .any(|path| state.starts_with(path)),
        "Agent state must not be inside a public runtime mount"
    );
    ensure!(
        project == config.project_path && project != state && !state.starts_with(&project),
        "Project runtime mount would expose agent state"
    );
    ensure!(
        data.canonicalize()? == data && data.starts_with(&state) && !project.starts_with(data),
        "Invalid private placement data mount"
    );
    let metadata = std::fs::metadata(executable)?;
    ensure!(
        metadata.is_file() && metadata.mode() & 0o022 == 0,
        "Runtime binary must not be group- or world-writable"
    );
    let scratch = data.join(".sandbox-tmp");
    crate::outbox::private_directory(&scratch)?;
    let project_id_after = project_attribute(&scratch)?.1.project;
    ensure!(
        project_id_after == project_id,
        "Sandbox temporary directory escapes the disk quota"
    );
    let sandbox = Sandbox {
        cgroup: Cgroup::new(resources, &state, project_id, &config.id, slot)?,
        program: filter_file()?,
        scopes: scope_rules()?,
        preserve_listener: config.hosting.is_some(),
    };
    let mut command = tokio::process::Command::new(bubblewrap()?);
    command.args([
        "--unshare-user",
        "--unshare-ipc",
        "--unshare-pid",
        "--unshare-uts",
        "--unshare-cgroup",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--disable-userns",
    ]);
    for path in ["/usr", "/bin", "/lib", "/lib64"] {
        if Path::new(path).exists() {
            command.arg("--ro-bind").arg(path).arg(path);
        }
    }
    for path in [
        "/etc/ssl/certs",
        "/etc/hosts",
        "/etc/resolv.conf",
        "/etc/nsswitch.conf",
        "/etc/ld.so.cache",
    ] {
        if Path::new(path).exists() {
            command.arg("--ro-bind").arg(path).arg(path);
        }
    }
    command
        .arg("--ro-bind")
        .arg(executable)
        .arg("/run/flow-like-worker")
        .arg("--ro-bind")
        .arg(&project)
        .arg(&project);
    // A revision can be reused by several placements, and siblings may publish
    // secrets after this worker starts. Mask the shared directory whether or not
    // it has contents yet; the bind mount shows only this placement's own.
    if let Some(secrets) = secrets_directory(&project, ".secrets")? {
        command.arg("--tmpfs").arg(&secrets);
        if let Some(own) = secrets_directory(&secrets, &config.id)? {
            command.arg("--ro-bind").arg(&own).arg(&own);
        }
        command.arg("--remount-ro").arg(&secrets);
    }
    command
        .arg("--ro-bind")
        .arg(
            data.parent()
                .context("Missing data binding")?
                .join("binding.json"),
        )
        .arg(data.parent().unwrap().join("binding.json"))
        .arg("--bind")
        .arg(data)
        .arg(data)
        .args(model_asset_mounts(config, data)?)
        .arg("--bind")
        .arg(&scratch)
        .arg("/tmp")
        .args([
            "--proc",
            "/proc",
            "--remount-ro",
            "/proc",
            "--dev",
            "/dev",
            "--remount-ro",
            "/",
        ])
        .arg("--chdir")
        .arg(data)
        .args([
            "--setenv",
            "HOME",
            "/tmp",
            "--setenv",
            "TMPDIR",
            "/tmp",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
            "--seccomp",
        ])
        .arg(sandbox.program.as_raw_fd().to_string())
        .args(["--", "/run/flow-like-worker"]);
    Ok((command, sandbox))
}

/// `--ro-bind <blob> <mount point>` for each model-store file the workload reads directly;
/// the agent left an empty mount point for each inside the placement data.
fn model_asset_mounts(
    config: &crate::config::PlacementConfig,
    data: &Path,
) -> Result<Vec<std::ffi::OsString>> {
    #[cfg(feature = "runtime")]
    {
        let binds = crate::dependencies::model_asset_binds(config, data)?;
        Ok(binds
            .into_iter()
            .flat_map(|(blob, mount_point)| {
                [
                    "--ro-bind".into(),
                    blob.into_os_string(),
                    mount_point.into_os_string(),
                ]
            })
            .collect())
    }
    #[cfg(not(feature = "runtime"))]
    {
        let _ = (config, data);
        Ok(Vec::new())
    }
}

impl Sandbox {
    pub(super) fn attach(&self, command: &mut tokio::process::Command) -> Result<()> {
        let group = self.cgroup.procs.as_raw_fd();
        let scopes = self.scopes.as_raw_fd();
        let program = self.program.as_raw_fd();
        let preserve_listener = self.preserve_listener;
        // Only async-signal-safe syscalls are used after fork. Bubblewrap installs
        // seccomp after its trusted namespace/mount setup, just before exec.
        unsafe {
            command.pre_exec(move || {
                if libc::write(group, b"0".as_ptr().cast(), 1) != 1
                    || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                    || libc::syscall(libc::SYS_landlock_restrict_self, scopes, 0) != 0
                    // Clear ambient descriptor inheritance, including any FD
                    // opened by a native dependency without CLOEXEC. Only the
                    // broker, read-only lock, optional listener and policy pass.
                    || libc::syscall(libc::SYS_close_range, 5_u32, u32::MAX, 4_u32) != 0
                    || (preserve_listener && libc::fcntl(5, libc::F_SETFD, 0) != 0)
                    || libc::fcntl(program, libc::F_SETFD, 0) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }
    pub(super) fn signal(&self, force: bool) -> Result<()> {
        self.cgroup.signal(force)
    }
}

const LANDLOCK_NET_BIND_TCP: u64 = 1;
const LANDLOCK_NET_CONNECT_TCP: u64 = 1 << 1;
const LANDLOCK_RULE_NET_PORT: libc::c_int = 2;
/// What GPU drivers enumerate devices through.
const ENGINE_SYSFS: [&str; 5] = [
    "/sys/dev",
    "/sys/devices",
    "/sys/bus/pci",
    "/sys/class/drm",
    "/sys/module/nvidia",
];
const ENGINE_CONFIG: [&str; 3] = ["/etc/ld.so.cache", "/etc/vulkan", "/etc/OpenCL"];

/// An engine may bind its own loopback port and connect nowhere over TCP.
fn engine_rules(port: u16) -> Result<File> {
    #[repr(C)]
    struct Rules {
        fs: u64,
        net: u64,
        scopes: u64,
    }
    #[repr(C)]
    struct NetPort {
        allowed_access: u64,
        port: u64,
    }
    let rules = Rules {
        fs: 0,
        net: LANDLOCK_NET_BIND_TCP | LANDLOCK_NET_CONNECT_TCP,
        scopes: 3,
    };
    // SAFETY: the kernel reads `size_of::<Rules>()` bytes of the live struct.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &rules,
            std::mem::size_of::<Rules>(),
            0,
        )
    };
    ensure!(fd >= 0, "Cannot create the engine's Landlock network rules");
    // SAFETY: the syscall returned a new descriptor that nothing else owns.
    let ruleset = unsafe { File::from_raw_fd(fd as i32) };
    let bind = NetPort {
        allowed_access: LANDLOCK_NET_BIND_TCP,
        port: u64::from(port),
    };
    // SAFETY: the kernel reads the live rule for this ruleset descriptor.
    let added = unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            ruleset.as_raw_fd(),
            LANDLOCK_RULE_NET_PORT,
            &bind,
            0,
        )
    };
    ensure!(added == 0, "Cannot allow the engine to bind port {port}");
    Ok(ruleset)
}

/// The workload policy, and no internet socket but TCP. Landlock limits TCP to binding the
/// engine's own port; nothing else would stop UDP, raw, SCTP or MPTCP traffic from an engine
/// that a crafted model took over.
fn engine_filter() -> Vec<libc::sock_filter> {
    let statement = |code, k| libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    };
    let jump = |k, jt, jf| libc::sock_filter {
        code: 0x15,
        jt,
        jf,
        k,
    };
    let mut program = filter();
    let allow = program.pop();
    program.extend([
        jump(libc::SYS_socket as u32, 0, 10),
        statement(0x20, 16),
        jump(libc::AF_INET as u32, 1, 0),
        jump(libc::AF_INET6 as u32, 0, 7),
        statement(0x20, 24),
        statement(0x54, 0xf),
        jump(libc::SOCK_STREAM as u32, 0, 3),
        statement(0x20, 32),
        jump(0, 2, 0),
        jump(libc::IPPROTO_TCP as u32, 1, 0),
        statement(0x06, 0x0005_0000 | libc::EPERM as u32),
    ]);
    program.extend(allow);
    program
}

fn gpu_devices() -> Vec<PathBuf> {
    let mut devices: Vec<PathBuf> = ["/dev/dri", "/dev/kfd"]
        .iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect();
    if let Ok(entries) = std::fs::read_dir("/dev") {
        let mut nvidia: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("nvidia"))
            .map(|entry| entry.path())
            .collect();
        nvidia.sort();
        devices.extend(nvidia);
    }
    devices
}

/// The engine profile: the system read-only, the runtime and model files read-only where
/// they are, GPU device nodes, a private `/tmp`, TCP limited to binding its own loopback
/// port, and no other internet socket. GGUF parsing is an attack surface, so engines get the
/// workload class.
pub(crate) fn engine_command(
    program: &Path,
    args: &[std::ffi::OsString],
    read_only: &[PathBuf],
    port: u16,
) -> Result<tokio::process::Command> {
    landlock_abi()?;
    let metadata = std::fs::metadata(program)?;
    ensure!(
        metadata.is_file() && metadata.mode() & 0o022 == 0,
        "Engine {} must not be group- or world-writable",
        program.display()
    );
    let seccomp = policy_file(&engine_filter())?;
    let rules = engine_rules(port)?;
    let mut command = tokio::process::Command::new(bubblewrap()?);
    command.args([
        "--unshare-user",
        "--unshare-ipc",
        "--unshare-pid",
        "--unshare-uts",
        "--unshare-cgroup",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--disable-userns",
    ]);
    engine_mounts(&mut command, program, read_only)?;
    command
        .args(["--setenv", "HOME", "/tmp", "--setenv", "TMPDIR", "/tmp"])
        .arg("--seccomp")
        .arg(seccomp.as_raw_fd().to_string())
        .arg("--")
        .arg(program)
        .args(args);
    restrict_engine(&mut command, seccomp, rules);
    Ok(command)
}

/// The system read-only, the engine's files read-only where they are, GPU devices, and a
/// private `/tmp`.
fn engine_mounts(
    command: &mut tokio::process::Command,
    program: &Path,
    read_only: &[PathBuf],
) -> Result<()> {
    let system = ["/usr", "/bin", "/lib", "/lib64"]
        .iter()
        .chain(ENGINE_CONFIG.iter())
        .chain(ENGINE_SYSFS.iter())
        .map(Path::new)
        .filter(|path| path.exists());
    for path in system {
        command.arg("--ro-bind").arg(path).arg(path);
    }
    for path in read_only.iter().map(PathBuf::as_path).chain([program]) {
        ensure!(
            path.is_absolute(),
            "Engine path {} must be absolute",
            path.display()
        );
        command.arg("--ro-bind").arg(path).arg(path);
    }
    command.args(["--proc", "/proc", "--dev", "/dev"]);
    for device in gpu_devices() {
        command.arg("--dev-bind").arg(&device).arg(&device);
    }
    command.args(["--tmpfs", "/tmp", "--remount-ro", "/"]);
    Ok(())
}

/// Bubblewrap starts without privileges, inside the network rules, and dies with the agent;
/// it receives the seccomp policy by descriptor.
fn restrict_engine(command: &mut tokio::process::Command, seccomp: File, rules: File) {
    let seccomp_fd = seccomp.as_raw_fd();
    let rules_fd = rules.as_raw_fd();
    // The closure owns both files, so they stay open until the child has started.
    let held = (seccomp, rules);
    // SAFETY: only async-signal-safe calls run between fork and exec.
    unsafe {
        command.pre_exec(move || {
            let _ = &held;
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
                || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                || libc::syscall(libc::SYS_landlock_restrict_self, rules_fd, 0) != 0
                || libc::fcntl(seccomp_fd, libc::F_SETFD, 0) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mount_escape_and_cpu_quota_parsing_are_exact() {
        assert_eq!(
            mount_field("/srv/flow\\040like").unwrap(),
            PathBuf::from("/srv/flow like")
        );
        assert!(mount_field("/srv/flow\\999").is_err());
        assert!(mount_field("/srv/flow\\777").is_err());
        assert_eq!(cpu("150000 100000").unwrap(), 1500);
        assert!(cpu("max 100000").is_err());
        assert!(cpu("1 0").is_err());
    }

    /// The call data the kernel hands a seccomp program: number, architecture, arguments.
    fn call_data(call: libc::c_long, args: [u64; 3]) -> [u8; 64] {
        #[cfg(target_arch = "x86_64")]
        let architecture: u32 = 0xc000_003e;
        #[cfg(target_arch = "aarch64")]
        let architecture: u32 = 0xc000_00b7;
        let mut data = [0_u8; 64];
        data[..4].copy_from_slice(&(call as u32).to_ne_bytes());
        data[4..8].copy_from_slice(&architecture.to_ne_bytes());
        for (index, arg) in args.iter().enumerate() {
            data[16 + index * 8..24 + index * 8].copy_from_slice(&arg.to_ne_bytes());
        }
        data
    }

    /// Runs a seccomp program on one call, for the instructions the policies use: loads of the
    /// call data, AND, equality and bit tests, and returns.
    fn verdict(program: &[libc::sock_filter], call: libc::c_long, args: [u64; 3]) -> u32 {
        let data = call_data(call, args);
        let (mut next, mut accumulator) = (0, 0_u32);
        loop {
            let instruction = program[next];
            next += 1;
            let at = instruction.k as usize;
            let skip = |taken: bool| {
                usize::from(if taken {
                    instruction.jt
                } else {
                    instruction.jf
                })
            };
            match instruction.code {
                0x20 => {
                    accumulator =
                        u32::from_ne_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
                }
                0x54 => accumulator &= instruction.k,
                0x15 => next += skip(accumulator == instruction.k),
                0x45 => next += skip(accumulator & instruction.k != 0),
                0x06 => return instruction.k,
                code => panic!("unexpected seccomp instruction {code:#x}"),
            }
        }
    }

    #[test]
    fn engines_open_no_internet_socket_but_tcp() {
        let (engine, workload) = (engine_filter(), filter());
        let allowed = 0x7fff_0000;
        let refused = 0x0005_0000 | libc::EPERM as u32;
        let socket = |program: &[libc::sock_filter], domain: i32, kind: i32, protocol: i32| {
            let args = [domain as u64, kind as u64, protocol as u64];
            verdict(program, libc::SYS_socket, args)
        };
        let stream = libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK;
        let others = [
            (libc::SOCK_DGRAM, 0),
            (libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::IPPROTO_ICMP),
            (libc::SOCK_RAW, libc::IPPROTO_RAW),
            (libc::SOCK_SEQPACKET, 0),
            (libc::SOCK_STREAM, libc::IPPROTO_SCTP),
            (libc::SOCK_STREAM, 262),
        ];
        for domain in [libc::AF_INET, libc::AF_INET6] {
            assert_eq!(socket(&engine[..], domain, stream, 0), allowed);
            let tcp = socket(&engine[..], domain, libc::SOCK_STREAM, libc::IPPROTO_TCP);
            assert_eq!(tcp, allowed);
            for (kind, protocol) in others {
                let answer = socket(&engine[..], domain, kind, protocol);
                assert_eq!(answer, refused, "{domain} {kind} {protocol}");
            }
            assert_eq!(socket(&workload[..], domain, libc::SOCK_DGRAM, 0), allowed);
        }
        assert_eq!(
            socket(&engine[..], libc::AF_UNIX, libc::SOCK_DGRAM, 0),
            allowed
        );
        assert_eq!(
            socket(&engine[..], libc::AF_NETLINK, libc::SOCK_RAW, 0),
            allowed
        );
        assert_eq!(verdict(&engine, libc::SYS_ptrace, [0; 3]), refused);
        assert_eq!(verdict(&engine, libc::SYS_read, [0; 3]), allowed);
    }

    #[test]
    fn sandbox_refuses_the_managed_project_root_but_accepts_a_revision() {
        let state = Path::new("/srv/agent");
        for shared in ["/srv/agent/projects", "/srv/agent/projects/project"] {
            assert!(ensure_single_project_tree(Path::new(shared), state).is_err());
        }
        for single in [
            "/srv/agent/projects/project/revisions/digest",
            "/srv/agent/projects/project/online-cache",
            "/srv/projects/project",
        ] {
            assert!(ensure_single_project_tree(Path::new(single), state).is_ok());
        }
    }

    #[test]
    fn secrets_mask_exists_before_spawn_and_read_only_projects_need_none() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir()?;
        let project = root.path().canonicalize()?;
        let secrets = secrets_directory(&project, ".secrets")?.context("Missing secrets mask")?;
        assert_eq!(std::fs::metadata(&secrets)?.mode() & 0o777, 0o700);
        assert_eq!(
            secrets_directory(&secrets, "placement")?,
            Some(secrets.join("placement"))
        );
        assert_eq!(secrets_directory(&project, ".secrets")?, Some(secrets));
        let locked = project.join("locked");
        std::fs::create_dir(&locked)?;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500))?;
        let result = secrets_directory(&locked, ".secrets");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700))?;
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(result?, None);
        }
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "Requires the explicit Linux quota/cgroup acceptance harness"]
    async fn linux_boundary_acceptance() -> Result<()> {
        use std::os::linux::net::SocketAddrExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let root = PathBuf::from(std::env::var_os("FLOW_LIKE_ISOLATION_TEST_ROOT").context(
            "Run scripts/test-standalone-isolation-linux.sh with a Linux lib test executable",
        )?)
        .canonicalize()?;
        let state = root.join("state");
        let data = state.join("placement-data/acceptance/current/store");
        let project = root.join("project");
        crate::outbox::private_directory(&project)?;
        std::fs::write(project.join("public"), b"project")?;
        for (placement, contents) in [
            ("acceptance", b"own setting".as_slice()),
            ("other-placement", b"other setting".as_slice()),
        ] {
            crate::outbox::private_directory(&project.join(".secrets").join(placement))?;
            std::fs::write(
                project.join(".secrets").join(placement).join("token"),
                contents,
            )?;
        }
        std::fs::write(state.join("agent-secret"), b"never exposed")?;
        std::fs::write(state.join("sibling-secret"), b"never exposed")?;
        std::fs::write(data.parent().unwrap().join("binding.json"), b"{}")?;
        let lock_path = state.join("test-worker.lock");
        std::fs::write(&lock_path, b"")?;
        let lock = File::open(lock_path)?;
        let secret = File::open(state.join("agent-secret"))?;
        let secret_fd = unsafe { libc::fcntl(secret.as_raw_fd(), libc::F_DUPFD, 64) };
        ensure!(
            secret_fd >= 64,
            "Cannot create intentional ambient descriptor fixture"
        );
        let secret = unsafe { File::from_raw_fd(secret_fd) };
        let abstract_name = format!("flow-like-isolation-{}", std::process::id());
        let abstract_address = std::os::unix::net::SocketAddr::from_abstract_name(&abstract_name)?;
        let _outside_socket = std::os::unix::net::UnixListener::bind_addr(&abstract_address)?;
        let config: crate::config::PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"acceptance", "project_id":"test", "deployment_id":"test", "revision":"test",
            "source":"offline", "project_path":project, "events":[],
            "hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":1,"request_timeout_secs":5,"auth_secret":"test"},
            "resources":{"profile":"linux_sandbox","cpu_millis":100,"memory_bytes":128*1024*1024,
                "max_processes":32,"disk_bytes":16*1024*1024}
        }))?;
        let python = Path::new("/usr/bin/python3").canonicalize()?;
        for mode in ["boundary", "disk", "pids", "cpu", "memory"] {
            let (mut broker, child_broker) = crate::ipc::socket_pair()?;
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            let listener_address = listener.local_addr()?;
            let (mut child_command, sandbox) = command(&config, &state, &data, &python, 0)?;
            child_command
                .env_clear()
                .env("FLOW_LIKE_TEST_MODE", mode)
                .env("FLOW_LIKE_TEST_DATA", &data)
                .env("FLOW_LIKE_TEST_PROJECT", &project)
                .env("FLOW_LIKE_TEST_SECRET", state.join("agent-secret"))
                .env("FLOW_LIKE_TEST_SIBLING", state.join("sibling-secret"))
                .env("FLOW_LIKE_TEST_PARENT", std::process::id().to_string())
                .env("FLOW_LIKE_TEST_ABSTRACT", &abstract_name)
                .env("FLOW_LIKE_TEST_AMBIENT_FD", secret.as_raw_fd().to_string())
                .args([
                    "-c",
                    include_str!("../../tests/fixtures/linux_isolation_probe.py"),
                ])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let descriptors =
                crate::ipc::attach_child_descriptors(&mut child_command, &child_broker, &lock)?;
            let listener_descriptor = crate::ipc::attach_listener(&mut child_command, &listener)?;
            sandbox.attach(&mut child_command)?;
            let child = child_command.spawn()?;
            drop((descriptors, listener_descriptor, child_broker));
            if mode == "boundary" {
                let mut client = tokio::net::TcpStream::connect(listener_address).await?;
                client.write_all(b"hosted").await?;
                let mut response = [0_u8; 6];
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    client.read_exact(&mut response),
                )
                .await??;
                assert_eq!(&response, b"served");
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    broker.read_exact(&mut response),
                )
                .await??;
                assert_eq!(&response, b"broker");
                let mut oversized = config.resources.clone().unwrap();
                oversized.cpu_millis = Some(1_024_000);
                assert!(Cgroup::new(&oversized, &state, 314160, "over-budget", 1).is_err());
            }
            let output =
                tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
                    .await??;
            if mode == "memory" {
                assert!(
                    !output.status.success(),
                    "memory probe unexpectedly survived"
                );
                let events = read(&sandbox.cgroup.root.join("memory.events"))?;
                assert!(
                    events.lines().any(|line| line
                        .strip_prefix("oom_kill ")
                        .and_then(|value| value.parse::<u64>().ok())
                        .is_some_and(|value| value > 0)),
                    "{events}"
                );
            } else {
                assert!(
                    output.status.success(),
                    "{mode}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    String::from_utf8_lossy(&output.stdout).contains(&format!("{mode}:passed"))
                );
                if mode == "cpu" {
                    let stats = read(&sandbox.cgroup.root.join("cpu.stat"))?;
                    assert!(
                        stats.lines().any(|line| line
                            .strip_prefix("nr_throttled ")
                            .and_then(|value| value.parse::<u64>().ok())
                            .is_some_and(|value| value > 0)),
                        "{stats}"
                    );
                }
            }
            drop(sandbox);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(())
    }
}
