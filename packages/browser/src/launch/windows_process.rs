// Derived from agent-browser cli/src/native/cdp/windows_process.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::collections::{BTreeMap, btree_map::Entry};
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_MORE_DATA, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS,
    HDESK,
};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows_sys::core::BOOL;

const DESKTOP_PREFIX: &str = "flow-like-browser-";
const TERMINATED_EXIT_CODE: u32 = 1;
const TEARDOWN_WAIT_MS: u32 = 5_000;
const MEMBER_LIST_CAPACITY: usize = 256;

pub(crate) struct WindowsChild {
    process: OwnedHandle,
    job: OwnedHandle,
    members: Mutex<BTreeMap<u32, OwnedHandle>>,
    _desktop: Option<Desktop>,
    pid: u32,
}

/// JOBOBJECT_BASIC_PROCESS_ID_LIST with room for more than one process id.
#[repr(C)]
struct ProcessIdList {
    _assigned: u32,
    listed: u32,
    ids: [usize; MEMBER_LIST_CAPACITY],
}

const _: () = assert!(
    std::mem::offset_of!(ProcessIdList, ids)
        == std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList)
);

impl WindowsChild {
    /// HANDLE_LIST restricts only this child: while CreateProcessW runs, a process
    /// that another thread spawns with bInheritHandles (every std and tokio spawn)
    /// can inherit the inheritable stderr writer. The returned reader then reaches
    /// EOF only once that process exits as well, so callers detect browser exit with
    /// try_wait and never wait for EOF. The inheritable NUL and stderr duplicates are
    /// closed as soon as CreateProcessW returns to keep that window short.
    pub(crate) fn spawn(
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
        private_desktop: bool,
    ) -> io::Result<(WindowsChild, File)> {
        let (application, mut command_line) = application_and_command_line(program, args)?;
        let environment = environment_block(env)?;
        let job = create_job()?;
        let mut desktop = private_desktop.then(Desktop::create).transpose()?;
        let (inherited, stderr_reader) = ChildStdio::new()?;

        let mut attributes = AttributeList::for_child(
            Box::new([raw(&inherited.null), raw(&inherited.stderr)]),
            Box::new([raw(&job)]),
        )?;
        let startup = startup_info(&inherited, desktop.as_mut(), &mut attributes);

        let created = create_process(
            &application,
            &mut command_line,
            environment.as_deref(),
            &startup,
        );
        drop(inherited);
        let info = created?;
        // SAFETY: a successful CreateProcessW hands us two new handles; the job
        // assignment is already done, so the thread handle is not needed.
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
        let child = WindowsChild {
            process,
            job,
            members: Mutex::new(BTreeMap::new()),
            _desktop: desktop,
            pid: info.dwProcessId,
        };
        Ok((child, File::from(OwnedHandle::from(stderr_reader))))
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn terminate_job(&self) {
        self.hold_members();
        // SAFETY: the job holds only the browser tree launched by spawn.
        if unsafe { TerminateJobObject(raw(&self.job), TERMINATED_EXIT_CODE) } == 0 {
            let error = io::Error::last_os_error();
            tracing::warn!(
                pid = self.pid,
                %error,
                "TerminateJobObject failed for the browser process tree"
            );
        }
    }

    pub(crate) fn try_wait(&self) -> io::Result<Option<u32>> {
        self.wait_for(0)
    }

    /// A terminated process leaves the job list before it closes its handles, so the members
    /// are opened before TerminateJobObject; each handle signals once its files are closed.
    fn hold_members(&self) {
        let ids = match self.member_ids() {
            Ok(ids) => ids,
            Err(error) => {
                tracing::debug!(pid = self.pid, %error, "could not list the browser job processes");
                return;
            }
        };
        let mut members = self.members.lock().unwrap_or_else(PoisonError::into_inner);
        for id in ids {
            if let Entry::Vacant(slot) = members.entry(id)
                && let Some(member) = self.open_member(id)
            {
                slot.insert(member);
            }
        }
    }

    fn member_ids(&self) -> io::Result<Vec<u32>> {
        let mut list = ProcessIdList {
            _assigned: 0,
            listed: 0,
            ids: [0; MEMBER_LIST_CAPACITY],
        };
        // SAFETY: list starts with the JOBOBJECT_BASIC_PROCESS_ID_LIST layout (checked at
        // compile time) and its full size is passed.
        let queried = unsafe {
            QueryInformationJobObject(
                raw(&self.job),
                JobObjectBasicProcessIdList,
                std::ptr::from_mut(&mut list).cast(),
                size_of::<ProcessIdList>() as u32,
                null_mut(),
            )
        };
        if queried == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
                return Err(context(
                    error,
                    "QueryInformationJobObject(JobObjectBasicProcessIdList)",
                ));
            }
        }
        let listed = (list.listed as usize).min(MEMBER_LIST_CAPACITY);
        Ok(list.ids[..listed]
            .iter()
            .filter_map(|&id| u32::try_from(id).ok())
            .collect())
    }

    fn open_member(&self, id: u32) -> Option<OwnedHandle> {
        // SAFETY: opens a new handle that `owned` takes; an exited process yields null.
        let process = owned(
            unsafe {
                OpenProcess(
                    PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                    0,
                    id,
                )
            },
            "OpenProcess",
        )
        .ok()?;
        let mut member = 0;
        // SAFETY: both handles are live; the check rejects a process id reused outside the job.
        let checked = unsafe { IsProcessInJob(raw(&process), raw(&self.job), &mut member) };
        (checked != 0 && member != 0).then_some(process)
    }

    fn wait_members(&mut self, deadline: Instant) -> bool {
        let members = std::mem::take(
            self.members
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner),
        );
        members.values().all(|member| {
            // SAFETY: the member handle stays owned for the whole wait.
            let outcome = unsafe { WaitForSingleObject(raw(member), remaining_ms(deadline)) };
            outcome == WAIT_OBJECT_0
        })
    }

    fn wait_for(&self, timeout_ms: u32) -> io::Result<Option<u32>> {
        // SAFETY: the process handle stays owned for both calls. Waiting first keeps
        // an exit code of STILL_ACTIVE (259) from reading as a running process.
        match unsafe { WaitForSingleObject(raw(&self.process), timeout_ms) } {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                check(
                    unsafe { GetExitCodeProcess(raw(&self.process), &mut code) },
                    format_args!("GetExitCodeProcess for browser process {}", self.pid),
                )?;
                Ok(Some(code))
            }
            WAIT_TIMEOUT => Ok(None),
            other => Err(os_error(format_args!(
                "WaitForSingleObject for browser process {} returned {other:#x}",
                self.pid
            ))),
        }
    }
}

impl Drop for WindowsChild {
    fn drop(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(u64::from(TEARDOWN_WAIT_MS));
        self.terminate_job();
        match self.wait_for(remaining_ms(deadline)) {
            Ok(Some(_)) => {}
            Ok(None) => tracing::warn!(
                pid = self.pid,
                "browser process did not exit within {TEARDOWN_WAIT_MS} ms of TerminateJobObject"
            ),
            Err(error) => {
                tracing::warn!(pid = self.pid, %error, "waiting for the terminated browser process failed")
            }
        }
        if !self.wait_members(deadline) {
            tracing::warn!(
                pid = self.pid,
                "a process of the browser job did not exit within {TEARDOWN_WAIT_MS} ms of TerminateJobObject"
            );
        }
    }
}

fn remaining_ms(deadline: Instant) -> u32 {
    let remaining = deadline.saturating_duration_since(Instant::now());
    u32::try_from(remaining.as_millis()).unwrap_or(TEARDOWN_WAIT_MS)
}

struct Desktop {
    handle: HDESK,
    name: Vec<u16>,
}

// SAFETY: the handle is never selected on a thread; it is only closed in Drop.
unsafe impl Send for Desktop {}
unsafe impl Sync for Desktop {}

impl Desktop {
    // Some Chromium versions (150 among them) let DWM paint hidden headless windows
    // on the interactive desktop (agent-browser #1498); a private desktop contains them.
    fn create() -> io::Result<Desktop> {
        let name = wide(OsStr::new(&format!(
            "{DESKTOP_PREFIX}{}",
            uuid::Uuid::new_v4()
        )))?;
        // SAFETY: the name is NUL-terminated; the default DACL applies and no
        // switch-desktop access is requested.
        let handle = unsafe {
            CreateDesktopW(
                name.as_ptr(),
                null(),
                null(),
                0,
                DESKTOP_CREATEWINDOW | DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS,
                null(),
            )
        };
        if handle.is_null() {
            return Err(os_error("CreateDesktopW for the private browser desktop"));
        }
        Ok(Desktop { handle, name })
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // SAFETY: a desktop handle created by Desktop::create and closed only here.
        unsafe { CloseDesktop(self.handle) };
    }
}

struct ChildStdio {
    null: OwnedHandle,
    stderr: OwnedHandle,
}

impl ChildStdio {
    fn new() -> io::Result<(ChildStdio, io::PipeReader)> {
        let null_device = OpenOptions::new()
            .read(true)
            .write(true)
            .open("NUL")
            .map_err(|error| context(error, "open NUL for the browser stdio"))?;
        let (stderr_reader, stderr_writer) =
            io::pipe().map_err(|error| context(error, "create the browser stderr pipe"))?;
        let stdio = ChildStdio {
            null: inheritable(&null_device, "NUL")?,
            stderr: inheritable(&stderr_writer, "the stderr pipe")?,
        };
        Ok((stdio, stderr_reader))
    }
}

struct AttributeList {
    storage: Vec<usize>,
    values: [Box<[HANDLE]>; 2],
}

impl AttributeList {
    fn for_child(handles: Box<[HANDLE]>, jobs: Box<[HANDLE]>) -> io::Result<AttributeList> {
        let mut list = AttributeList {
            storage: initialized_attribute_storage(2)?,
            values: [handles, jobs],
        };
        let attributes = [
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST,
        ];
        for (attribute, values) in attributes.into_iter().zip(&list.values) {
            // SAFETY: the list owns the boxed arrays, whose heap storage never moves,
            // so the recorded pointers stay valid for every use of the list.
            let updated = unsafe {
                UpdateProcThreadAttribute(
                    list.storage.as_mut_ptr().cast(),
                    0,
                    attribute as usize,
                    values.as_ptr().cast(),
                    size_of_val(&**values),
                    null_mut(),
                    null(),
                )
            };
            check(
                updated,
                format_args!("UpdateProcThreadAttribute({attribute:#x})"),
            )?;
        }
        Ok(list)
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}

fn initialized_attribute_storage(count: u32) -> io::Result<Vec<usize>> {
    let mut bytes = 0;
    // SAFETY: a null list asks only for the required size.
    unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut bytes) };
    if bytes == 0 {
        return Err(os_error("InitializeProcThreadAttributeList size query"));
    }
    let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
    // SAFETY: storage is usize-aligned and at least the queried size.
    check(
        unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), count, 0, &mut bytes)
        },
        "InitializeProcThreadAttributeList",
    )?;
    Ok(storage)
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: only initialised lists are constructed and the storage is still live.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

fn startup_info(
    stdio: &ChildStdio,
    desktop: Option<&mut Desktop>,
    attributes: &mut AttributeList,
) -> STARTUPINFOEXW {
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = raw(&stdio.null);
    startup.StartupInfo.hStdOutput = raw(&stdio.null);
    startup.StartupInfo.hStdError = raw(&stdio.stderr);
    startup.StartupInfo.lpDesktop = desktop.map_or(null_mut(), |desktop| desktop.name.as_mut_ptr());
    startup.lpAttributeList = attributes.as_ptr();
    startup
}

fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes create an unnamed job whose handle is not inheritable.
    let job = owned(
        unsafe { CreateJobObjectW(null(), null()) },
        "CreateJobObjectW",
    )?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: limits has the layout and size of JobObjectExtendedLimitInformation.
    check(
        unsafe {
            SetInformationJobObject(
                raw(&job),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of_val(&limits) as u32,
            )
        },
        "SetInformationJobObject(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)",
    )?;
    Ok(job)
}

fn create_process(
    application: &[u16],
    command_line: &mut [u16],
    environment: Option<&[u16]>,
    startup: &STARTUPINFOEXW,
) -> io::Result<PROCESS_INFORMATION> {
    let mut flags = EXTENDED_STARTUPINFO_PRESENT;
    if environment.is_some() {
        flags |= CREATE_UNICODE_ENVIRONMENT;
    }
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: every string, attribute value and listed handle outlives the call.
    // The job assignment is atomic with process creation.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            flags,
            environment.map_or(null(), |block| block.as_ptr().cast()),
            null(),
            std::ptr::from_ref(startup).cast(),
            &mut info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn application_and_command_line(
    program: &Path,
    args: &[OsString],
) -> io::Result<(Vec<u16>, Vec<u16>)> {
    // canonicalize would yield a \\?\ path, which Chrome then sees as its own module path.
    let program = std::path::absolute(program).map_err(|error| {
        context(
            error,
            format_args!("resolve browser executable {}", program.display()),
        )
    })?;
    Ok((wide(program.as_os_str())?, command_line(&program, args)?))
}

fn command_line(program: &Path, args: &[OsString]) -> io::Result<Vec<u16>> {
    let mut line = quoted(program.as_os_str())?;
    for arg in args {
        line.push(u16::from(b' '));
        line.extend(quoted(arg)?);
    }
    line.push(0);
    Ok(line)
}

fn environment_block(overrides: &[(OsString, OsString)]) -> io::Result<Option<Vec<u16>>> {
    if overrides.is_empty() {
        return Ok(None);
    }
    let mut variables = BTreeMap::new();
    for (name, value) in std::env::vars_os() {
        variables.insert(sort_key(&name), (name, value));
    }
    for (name, value) in overrides {
        validate_variable(name, value)?;
        variables.insert(sort_key(name), (name.clone(), value.clone()));
    }
    let mut block = Vec::new();
    for (name, value) in variables.values() {
        block.extend(name.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(Some(block))
}

fn validate_variable(name: &OsStr, value: &OsStr) -> io::Result<()> {
    let name_units = units(name)?;
    units(value)?;
    let equals = u16::from(b'=');
    if name_units.is_empty() || name_units.iter().skip(1).any(|&unit| unit == equals) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "environment variable name {} is empty or contains '='",
                name.display()
            ),
        ));
    }
    Ok(())
}

fn sort_key(name: &OsStr) -> Vec<u16> {
    name.encode_wide()
        .map(|unit| u8::try_from(unit).map_or(unit, |byte| u16::from(byte.to_ascii_uppercase())))
        .collect()
}

fn check(result: BOOL, operation: impl Display) -> io::Result<()> {
    if result == 0 {
        Err(os_error(operation))
    } else {
        Ok(())
    }
}

fn os_error(operation: impl Display) -> io::Error {
    context(
        io::Error::last_os_error(),
        format_args!("{operation} failed"),
    )
}

fn context(error: io::Error, operation: impl Display) -> io::Error {
    io::Error::new(error.kind(), format!("{operation}: {error}"))
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle()
}

fn owned(handle: HANDLE, operation: &str) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        Err(os_error(operation))
    } else {
        // SAFETY: called only with a newly created, non-null handle that nothing else owns.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}

fn inheritable(source: &impl AsRawHandle, what: &str) -> io::Result<OwnedHandle> {
    let mut duplicate = null_mut();
    // SAFETY: duplicates a live handle within this process; `owned` takes the copy.
    check(
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                source.as_raw_handle(),
                GetCurrentProcess(),
                &mut duplicate,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            )
        },
        format_args!("DuplicateHandle for {what}"),
    )?;
    owned(duplicate, "DuplicateHandle")
}

fn units(value: &OsStr) -> io::Result<Vec<u16>> {
    let units: Vec<u16> = value.encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} contains a NUL character", value.display()),
        ));
    }
    Ok(units)
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut units = units(value)?;
    units.push(0);
    Ok(units)
}

/// Quotes one argument for CommandLineToArgvW, which Chrome uses. Backslashes
/// are doubled only before a quote or the closing quote, so paths and JSON survive.
fn quoted(value: &OsStr) -> io::Result<Vec<u16>> {
    let backslash = u16::from(b'\\');
    let quote = u16::from(b'"');
    let mut output = vec![quote];
    let mut slashes = 0;
    for unit in units(value)? {
        if unit == backslash {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(backslash, slashes));
        if unit == quote {
            output.extend(std::iter::repeat_n(backslash, slashes + 1));
        }
        output.push(unit);
        slashes = 0;
    }
    output.extend(std::iter::repeat_n(backslash, slashes * 2));
    output.push(quote);
    Ok(output)
}

#[cfg(all(test, windows))]
pub(crate) mod tests {
    use super::*;
    use std::io::Read;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use windows_sys::Win32::Foundation::CompareObjectHandles;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::StationsAndDesktops::{
        GetThreadDesktop, GetUserObjectInformationW, UOI_NAME,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetCurrentThreadId, INFINITE, OpenEventW, SYNCHRONIZATION_SYNCHRONIZE,
    };

    const TEST_DIR: &str = "FLOW_LIKE_BROWSER_TEST_WINDOWS_PROCESS_DIR";
    const PROFILE: &str = "profile";
    const PROBE_HANDLE: &str = "FLOW_LIKE_BROWSER_TEST_PROBE_HANDLE";
    const PROBE_NAME: &str = "FLOW_LIKE_BROWSER_TEST_PROBE_NAME";
    const TREE_READY: &str = "tree ready";
    const HELPER_LIFETIME: Duration = Duration::from_secs(30);
    const FILE_TIMEOUT: Duration = Duration::from_secs(15);
    const EXIT_TIMEOUT_MS: u32 = 5_000;

    fn helper_args(name: &str) -> Vec<OsString> {
        vec![
            "--exact".into(),
            format!("launch::windows_process::tests::{name}").into(),
            "--ignored".into(),
            "--nocapture".into(),
        ]
    }

    fn helper_dir() -> Option<PathBuf> {
        std::env::var_os(TEST_DIR).map(PathBuf::from)
    }

    fn spawn_helper(
        name: &str,
        env: &[(OsString, OsString)],
        private_desktop: bool,
    ) -> (WindowsChild, File) {
        WindowsChild::spawn(
            &std::env::current_exe().unwrap(),
            &helper_args(name),
            env,
            private_desktop,
        )
        .unwrap()
    }

    fn dir_env(dir: &Path) -> Vec<(OsString, OsString)> {
        vec![(TEST_DIR.into(), dir.as_os_str().to_owned())]
    }

    fn write_atomically(path: &Path, text: &str) {
        let staging = path.with_extension("partial");
        std::fs::write(&staging, text).unwrap();
        std::fs::rename(&staging, path).unwrap();
    }

    fn wait_for_file(path: &Path) -> String {
        let deadline = Instant::now() + FILE_TIMEOUT;
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn read_pid(path: &Path) -> u32 {
        wait_for_file(path).parse().unwrap()
    }

    fn read_until(reader: &mut File, needle: &str) -> String {
        let mut output = Vec::new();
        let mut buffer = [0u8; 256];
        while !String::from_utf8_lossy(&output).contains(needle) {
            let read = reader.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8_lossy(&output).into_owned()
    }

    fn current_desktop_name() -> String {
        let mut name = [0u16; 256];
        let mut needed = 0;
        // SAFETY: the thread desktop handle is borrowed, not closed.
        check(
            unsafe {
                GetUserObjectInformationW(
                    GetThreadDesktop(GetCurrentThreadId()),
                    UOI_NAME,
                    name.as_mut_ptr().cast(),
                    size_of_val(&name) as u32,
                    &mut needed,
                )
            },
            "GetUserObjectInformationW(UOI_NAME)",
        )
        .unwrap();
        let end = name.iter().position(|&unit| unit == 0).unwrap();
        String::from_utf16_lossy(&name[..end])
    }

    fn open_process(pid: u32) -> OwnedHandle {
        // SAFETY: opens a new handle that `owned` takes.
        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        owned(handle, "OpenProcess").unwrap()
    }

    fn exits_within(process: &OwnedHandle, timeout_ms: u32) -> bool {
        // SAFETY: the handle is owned by the caller for the whole wait.
        let outcome = unsafe { WaitForSingleObject(raw(process), timeout_ms) };
        outcome == WAIT_OBJECT_0
    }

    fn inheritable_event(name: &str) -> OwnedHandle {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        let name = wide(OsStr::new(name)).unwrap();
        // SAFETY: attributes and name outlive the call; `owned` takes the handle.
        owned(
            unsafe { CreateEventW(&attributes, 1, 0, name.as_ptr()) },
            "CreateEventW",
        )
        .unwrap()
    }

    fn hold_profile_file(dir: &Path) -> Option<File> {
        use std::os::windows::fs::OpenOptionsExt;
        let profile = dir.join(PROFILE);
        profile.is_dir().then(|| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(0)
                .open(profile.join("held"))
                .unwrap()
        })
    }

    /// Starts a tree whose root has exited while its leaf, still in the job, keeps a file of
    /// the returned profile folder open without FILE_SHARE_DELETE.
    pub(crate) fn spawn_leaf_holding_a_profile_file(
        dir: &Path,
    ) -> (WindowsChild, OwnedHandle, PathBuf) {
        let profile = dir.join(PROFILE);
        std::fs::create_dir(&profile).unwrap();
        std::fs::write(dir.join("exit-parent"), "1").unwrap();
        let (child, _stderr) = spawn_helper("tree_helper", &dir_env(dir), false);
        let leaf = open_process(read_pid(&dir.join("leaf.pid")));
        assert_eq!(child.wait_for(EXIT_TIMEOUT_MS * 3).unwrap(), Some(0));
        assert!(!exits_within(&leaf, 0));
        (child, leaf, profile)
    }

    #[test]
    #[ignore = "subprocess helper"]
    fn leaf_helper() {
        let Some(dir) = helper_dir() else { return };
        let _held = hold_profile_file(&dir);
        write_atomically(&dir.join("leaf.pid"), &std::process::id().to_string());
        std::thread::sleep(HELPER_LIFETIME);
    }

    #[test]
    #[ignore = "subprocess helper"]
    fn tree_helper() {
        let Some(dir) = helper_dir() else { return };
        write_atomically(&dir.join("desktop.txt"), &current_desktop_name());
        #[allow(
            clippy::zombie_processes,
            reason = "with exit-parent the leaf must outlive this root inside the job"
        )]
        let mut leaf = Command::new(std::env::current_exe().unwrap())
            .args(helper_args("leaf_helper"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for_file(&dir.join("leaf.pid"));
        eprintln!("{TREE_READY}");
        if dir.join("exit-parent").exists() {
            return;
        }
        leaf.wait().unwrap();
    }

    #[test]
    #[ignore = "subprocess helper"]
    fn owner_helper() {
        let Some(dir) = helper_dir() else { return };
        let (child, _stderr) = spawn_helper("tree_helper", &dir_env(&dir), true);
        write_atomically(&dir.join("root.pid"), &child.pid().to_string());
        child.wait_for(INFINITE).unwrap();
    }

    #[test]
    #[ignore = "subprocess helper"]
    fn probe_helper() {
        let Some(dir) = helper_dir() else { return };
        let value: usize = std::env::var(PROBE_HANDLE).unwrap().parse().unwrap();
        let name = wide(&std::env::var_os(PROBE_NAME).unwrap()).unwrap();
        // SAFETY: opens the named event of the parent test; `owned` takes the handle.
        let named = owned(
            unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) },
            "OpenEventW",
        )
        .unwrap();
        // An inherited handle keeps its value, so it cannot equal the handle opened here.
        // SAFETY: only compares the two values; one that is not a valid handle compares unequal.
        let inherited = value != raw(&named).addr()
            && unsafe {
                CompareObjectHandles(std::ptr::without_provenance_mut(value), raw(&named))
            } != 0;
        let verdict = if inherited { "inherited" } else { "absent" };
        write_atomically(&dir.join("probe.txt"), verdict);
    }

    #[test]
    fn private_desktop_and_tree_cleanup() {
        for private_desktop in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (child, mut stderr) =
                spawn_helper("tree_helper", &dir_env(dir.path()), private_desktop);
            let desktop = wait_for_file(&dir.path().join("desktop.txt"));
            if private_desktop {
                assert!(desktop.starts_with(DESKTOP_PREFIX), "{desktop}");
                assert_ne!(desktop, current_desktop_name());
            } else {
                assert_eq!(desktop, current_desktop_name());
            }
            let leaf = open_process(read_pid(&dir.path().join("leaf.pid")));
            let root = open_process(child.pid());
            assert!(read_until(&mut stderr, TREE_READY).contains(TREE_READY));
            assert_eq!(child.try_wait().unwrap(), None);
            drop(child);
            assert!(exits_within(&root, EXIT_TIMEOUT_MS));
            assert!(exits_within(&leaf, EXIT_TIMEOUT_MS));
            let mut rest = Vec::new();
            stderr.read_to_end(&mut rest).unwrap();
        }
    }

    #[test]
    fn terminate_job_kills_the_running_tree() {
        let dir = tempfile::tempdir().unwrap();
        let (child, _stderr) = spawn_helper("tree_helper", &dir_env(dir.path()), false);
        let leaf = open_process(read_pid(&dir.path().join("leaf.pid")));
        child.terminate_job();
        assert_eq!(
            child.wait_for(EXIT_TIMEOUT_MS).unwrap(),
            Some(TERMINATED_EXIT_CODE)
        );
        assert!(exits_within(&leaf, EXIT_TIMEOUT_MS));
    }

    #[test]
    fn cleanup_includes_descendants_after_browser_exits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exit-parent"), "1").unwrap();
        let (child, _stderr) = spawn_helper("tree_helper", &dir_env(dir.path()), false);
        let leaf = open_process(read_pid(&dir.path().join("leaf.pid")));
        assert_eq!(child.wait_for(EXIT_TIMEOUT_MS * 3).unwrap(), Some(0));
        assert!(!exits_within(&leaf, 0));
        child.terminate_job();
        assert!(exits_within(&leaf, EXIT_TIMEOUT_MS));
        assert_eq!(child.try_wait().unwrap(), Some(0));
    }

    #[test]
    fn drop_waits_until_every_process_of_the_job_has_closed_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let (child, leaf, profile) = spawn_leaf_holding_a_profile_file(dir.path());
        child.terminate_job();
        drop(child);
        assert!(
            exits_within(&leaf, 0),
            "a process of the job outlived the drop"
        );
        std::fs::remove_dir_all(&profile).unwrap();
    }

    #[test]
    fn killing_owner_reaps_tree_without_touching_unrelated_process() {
        let dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let mut other = Command::new(std::env::current_exe().unwrap())
            .args(helper_args("leaf_helper"))
            .env(TEST_DIR, other_dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut owner = Command::new(std::env::current_exe().unwrap())
            .args(helper_args("owner_helper"))
            .env(TEST_DIR, dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let root = open_process(read_pid(&dir.path().join("root.pid")));
        let leaf = open_process(read_pid(&dir.path().join("leaf.pid")));
        owner.kill().unwrap();
        owner.wait().unwrap();
        assert!(exits_within(&root, EXIT_TIMEOUT_MS));
        assert!(exits_within(&leaf, EXIT_TIMEOUT_MS));
        assert!(other.try_wait().unwrap().is_none());
        other.kill().unwrap();
        other.wait().unwrap();
    }

    #[test]
    fn handle_allow_list_keeps_other_inheritable_handles_out() {
        let name = format!("{DESKTOP_PREFIX}probe-{}", uuid::Uuid::new_v4());
        let event = inheritable_event(&name);
        let probe_env = |dir: &Path| -> Vec<(OsString, OsString)> {
            let mut env = dir_env(dir);
            env.push((PROBE_HANDLE.into(), raw(&event).addr().to_string().into()));
            env.push((PROBE_NAME.into(), name.clone().into()));
            env
        };

        let control = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(helper_args("probe_helper"))
            .envs(probe_env(control.path()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            wait_for_file(&control.path().join("probe.txt")),
            "inherited"
        );

        let dir = tempfile::tempdir().unwrap();
        let (child, _stderr) = spawn_helper("probe_helper", &probe_env(dir.path()), false);
        assert_eq!(child.wait_for(EXIT_TIMEOUT_MS * 3).unwrap(), Some(0));
        assert_eq!(wait_for_file(&dir.path().join("probe.txt")), "absent");
    }

    #[test]
    fn environment_block_overrides_case_insensitively_and_stays_sorted() {
        assert!(environment_block(&[]).unwrap().is_none());
        let block = environment_block(&[
            ("path".into(), r"C:\override".into()),
            ("FLOW_LIKE_BROWSER_NEW".into(), "1".into()),
        ])
        .unwrap()
        .unwrap();
        assert!(block.ends_with(&[0, 0]));
        let text = String::from_utf16_lossy(&block[..block.len() - 2]);
        let entries: Vec<(&str, &str)> = text
            .split('\0')
            .map(|entry| {
                let split = 1 + entry[1..].find('=').unwrap();
                (&entry[..split], &entry[split + 1..])
            })
            .collect();
        let paths: Vec<_> = entries
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("path"))
            .collect();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].1, r"C:\override");
        assert!(entries.contains(&("FLOW_LIKE_BROWSER_NEW", "1")));
        let keys: Vec<Vec<u16>> = entries
            .iter()
            .map(|(name, _)| sort_key(OsStr::new(name)))
            .collect();
        assert!(keys.is_sorted());
        assert!(environment_block(&[("A=B".into(), "1".into())]).is_err());
        assert!(environment_block(&[("".into(), "1".into())]).is_err());
        assert!(environment_block(&[("A".into(), "1\0".into())]).is_err());
    }

    #[test]
    fn quotes_preserve_paths_quotes_and_empty_arguments() {
        for (input, expected) in [
            ("", "\"\""),
            ("plain", "\"plain\""),
            (
                r"C:\profile with spaces\",
                "\"C:\\profile with spaces\\\\\"",
            ),
            ("a\"b", "\"a\\\"b\""),
            ("a\\\"b", "\"a\\\\\\\"b\""),
            ("你好", "\"你好\""),
        ] {
            assert_eq!(
                String::from_utf16(&quoted(OsStr::new(input)).unwrap()).unwrap(),
                expected
            );
        }
        assert!(quoted(OsStr::new("a\0b")).is_err());
    }

    #[test]
    fn missing_executable_fails_without_starting_process() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.exe");
        assert!(WindowsChild::spawn(&missing, &[], &[], true).is_err());
    }

    #[test]
    fn child_is_send_and_sync_for_the_watchdog_thread() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WindowsChild>();
    }
}
