use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CifxDriverMode {
    /// Windows installed cifX driver, initialized by the operating system.
    SystemDriver,
    /// Hilscher LinuxCIFXDrv 3.x, using its installed firmware/configuration directory.
    LinuxV3 {
        base_directory: String,
        card_number: u16,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CifxConfig {
    /// Absolute path to the installed vendor DLL or shared library.
    pub library_path: String,
    pub driver_mode: CifxDriverMode,
    pub board: String,
    pub channel: u32,
    pub timeout_ms: u32,
    /// Optional exact prefix check against xDriverGetInformation's driver version.
    #[serde(default)]
    pub expected_version_prefix: Option<String>,
    #[serde(default)]
    pub start_bus: bool,
}
impl CifxConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            std::path::Path::new(&self.library_path).is_absolute()
                && self.library_path.len() <= 4096,
            "cifX requires an absolute installed library path",
        )?;
        require(
            !self.board.is_empty() && self.board.len() <= 15 && !self.board.contains('\0'),
            "cifX board name requires 1..15 characters without NUL",
        )?;
        require(self.channel < 6, "cifX channel must be in 0..5")?;
        require(
            (1..=30_000).contains(&self.timeout_ms),
            "cifX timeout must be in 1..30000 ms",
        )?;
        if let Some(prefix) = &self.expected_version_prefix {
            require(
                !prefix.is_empty() && prefix.len() <= 32,
                "cifX expected version prefix requires 1..32 characters",
            )?;
        }
        if let CifxDriverMode::LinuxV3 { base_directory, .. } = &self.driver_mode {
            require(
                std::path::Path::new(base_directory).is_absolute() && base_directory.len() <= 4096,
                "cifX firmware directory must be an absolute path",
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CifxInfo {
    pub driver_version: String,
    pub linux_driver_version: Option<String>,
    pub board_count: u32,
    pub firmware_name: String,
    pub firmware_version: String,
    pub channel_error: u32,
    pub input_areas: u32,
    pub output_areas: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CifxReadRequest {
    pub area: u32,
    pub offset: u32,
    pub length: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CifxWriteRequest {
    pub area: u32,
    pub offset: u32,
    pub data: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CifxState {
    pub host_ready: bool,
    pub bus_on: bool,
}

#[cfg(feature = "execute")]
pub use execution::CifxClient;

#[cfg(feature = "execute")]
mod execution {
    use super::*;
    use crate::Error;
    use libloading::Library;
    use std::{
        collections::HashSet,
        ffi::c_void,
        path::PathBuf,
        ptr,
        sync::{
            LazyLock, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    use std::{
        ffi::{CString, c_char, c_int, c_ulong},
        path::Path,
    };
    use tokio::sync::{mpsc, oneshot};
    use tokio_util::sync::CancellationToken;

    type Handle = *mut c_void;
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    type DriverOpen = unsafe extern "system" fn(*mut Handle) -> i32;
    type Close = unsafe extern "system" fn(Handle) -> i32;
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    type GetInfo = unsafe extern "system" fn(Handle, u32, *mut c_void) -> i32;
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    type ChannelOpen = unsafe extern "system" fn(Handle, *mut c_char, u32, *mut Handle) -> i32;
    type Io = unsafe extern "system" fn(Handle, u32, u32, u32, *mut c_void, u32) -> i32;
    type IoInfo = unsafe extern "system" fn(Handle, u32, u32, u32, *mut c_void) -> i32;
    type State = unsafe extern "system" fn(Handle, u32, *mut u32, u32) -> i32;
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    type LinuxVersion = unsafe extern "C" fn(u32, *mut c_char) -> i32;
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    type LinuxInit = unsafe extern "C" fn(*const LinuxInitOptions) -> i32;
    type LinuxDeinit = unsafe extern "C" fn();

    // Hilscher nxdrvlinux 3.x cifxlinux.h: native C layout, including unsigned long on LP64.
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    #[repr(C)]
    struct LinuxInitOptions {
        init_options: c_int,
        base_dir: *const c_char,
        poll_interval: c_ulong,
        poll_priority: c_int,
        trace_level: c_ulong,
        user_card_cnt: c_int,
        user_cards: *mut c_void,
        card_number: c_int,
        enable_card_locking: c_int,
        poll_stack_size: c_int,
        poll_schedpolicy: c_int,
        logfd: *mut c_void,
    }
    static ACTIVE_LIBRARIES: LazyLock<Mutex<HashSet<PathBuf>>> =
        LazyLock::new(|| Mutex::new(HashSet::new()));
    struct LibraryLease(PathBuf);
    impl LibraryLease {
        #[cfg(any(target_os = "linux", target_os = "windows", test))]
        fn acquire(path: &Path) -> Result<Self> {
            let mut active = ACTIVE_LIBRARIES
                .lock()
                .map_err(|_| Error::Invalid("cifX session registry is unavailable".into()))?;
            require(
                active.insert(path.to_path_buf()),
                "A cifX session already owns this driver library; close it before opening another channel",
            )?;
            Ok(Self(path.to_path_buf()))
        }
    }
    impl Drop for LibraryLease {
        fn drop(&mut self) {
            if let Ok(mut active) = ACTIVE_LIBRARIES.lock() {
                active.remove(&self.0);
            }
        }
    }
    struct Api {
        #[cfg(any(target_os = "linux", target_os = "windows", test))]
        driver_open: DriverOpen,
        driver_close: Close,
        #[cfg(any(target_os = "linux", target_os = "windows", test))]
        driver_info: GetInfo,
        #[cfg(any(target_os = "linux", target_os = "windows", test))]
        channel_open: ChannelOpen,
        channel_close: Close,
        #[cfg(any(target_os = "linux", target_os = "windows", test))]
        channel_info: GetInfo,
        read: Io,
        write: Io,
        io_info: IoInfo,
        host_state: State,
        bus_state: State,
    }
    impl Api {
        #[cfg(any(target_os = "linux", target_os = "windows", test))]
        unsafe fn load(library: &Library) -> Result<Self> {
            // Function signatures follow cifXUser.h; the library remains owned by NativeChannel.
            unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T> {
                unsafe { library.get::<T>(name) }
                    .map(|s| *s)
                    .map_err(|e| Error::Invalid(format!("cifX API symbol is unavailable: {e}")))
            }
            unsafe {
                Ok(Self {
                    driver_open: symbol(library, b"xDriverOpen\0")?,
                    driver_close: symbol(library, b"xDriverClose\0")?,
                    driver_info: symbol(library, b"xDriverGetInformation\0")?,
                    channel_open: symbol(library, b"xChannelOpen\0")?,
                    channel_close: symbol(library, b"xChannelClose\0")?,
                    channel_info: symbol(library, b"xChannelInfo\0")?,
                    read: symbol(library, b"xChannelIORead\0")?,
                    write: symbol(library, b"xChannelIOWrite\0")?,
                    io_info: symbol(library, b"xChannelIOInfo\0")?,
                    host_state: symbol(library, b"xChannelHostState\0")?,
                    bus_state: symbol(library, b"xChannelBusState\0")?,
                })
            }
        }
    }
    struct NativeChannel {
        _library: Library,
        _lease: LibraryLease,
        api: Api,
        driver: Handle,
        channel: Handle,
        deinit: Option<LinuxDeinit>,
        timeout_ms: u32,
        restore_host: Option<u32>,
        restore_bus: Option<u32>,
    }
    impl NativeChannel {
        fn open(config: &CifxConfig, stop: &CancellationToken) -> Result<(Self, CifxInfo)> {
            config.validate()?;
            require(!stop.is_cancelled(), "cifX connection cancelled")?;
            #[cfg(not(any(target_os = "linux", target_os = "windows", test)))]
            return Err(Error::Invalid(
                "The installed cifX driver adapter requires Linux or Windows".into(),
            ));
            #[cfg(any(target_os = "linux", target_os = "windows", test))]
            {
                // A native call already in progress must finish, but cancellation prevents
                // starting another initialization phase or activating the controller afterward.
                let check_startup = || require(!stop.is_cancelled(), "cifX connection cancelled");
                #[cfg(all(not(test), target_os = "linux"))]
                require(
                    matches!(config.driver_mode, CifxDriverMode::LinuxV3 { .. }),
                    "Linux requires cifX LinuxV3 initialization",
                )?;
                #[cfg(all(not(test), target_os = "windows"))]
                require(
                    matches!(config.driver_mode, CifxDriverMode::SystemDriver),
                    "Windows requires the cifX system driver mode",
                )?;
                let path = Path::new(&config.library_path).canonicalize()?;
                require(path.is_file(), "cifX library path must point to a file")?;
                let lease = LibraryLease::acquire(&path)?;
                // Loading a native library executes its initializer. The catalog permits local execution only.
                let library = unsafe { Library::new(&path) }.map_err(|e| {
                    Error::Invalid(format!("Cannot load the installed cifX library: {e}"))
                })?;
                check_startup()?;
                let api = unsafe { Api::load(&library) }?;
                let mut native = Self {
                    _library: library,
                    _lease: lease,
                    api,
                    driver: ptr::null_mut(),
                    channel: ptr::null_mut(),
                    deinit: None,
                    timeout_ms: config.timeout_ms,
                    restore_host: None,
                    restore_bus: None,
                };
                let linux_driver_version = if let CifxDriverMode::LinuxV3 {
                    base_directory,
                    card_number,
                } = &config.driver_mode
                {
                    let version: LinuxVersion = unsafe {
                        native
                            ._library
                            .get::<LinuxVersion>(b"cifXGetDriverVersion\0")
                    }
                    .map(|symbol| *symbol)
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                    let mut bytes = [0u8; 128];
                    check(
                        unsafe { version(bytes.len() as u32, bytes.as_mut_ptr().cast()) },
                        "cifXGetDriverVersion",
                    )?;
                    let version = text(&bytes);
                    require(
                        version.starts_with("LinuxCIFXDrv 3."),
                        "cifX Linux initialization supports the verified LinuxCIFXDrv 3.x ABI",
                    )?;
                    let initialize: LinuxInit =
                        unsafe { native._library.get::<LinuxInit>(b"cifXDriverInit\0") }
                            .map(|symbol| *symbol)
                            .map_err(|e| Error::Invalid(e.to_string()))?;
                    let deinitialize: LinuxDeinit =
                        unsafe { native._library.get::<LinuxDeinit>(b"cifXDriverDeinit\0") }
                            .map(|symbol| *symbol)
                            .map_err(|e| Error::Invalid(e.to_string()))?;
                    let directory = Path::new(base_directory).canonicalize()?;
                    require(directory.is_dir(), "cifX firmware path must be a directory")?;
                    let directory = CString::new(directory.to_string_lossy().as_bytes())
                        .map_err(|_| Error::Invalid("cifX firmware path contains NUL".into()))?;
                    let options = LinuxInitOptions {
                        init_options: 2,
                        base_dir: directory.as_ptr(),
                        poll_interval: 0,
                        poll_priority: 0,
                        trace_level: 0,
                        user_card_cnt: 0,
                        user_cards: ptr::null_mut(),
                        card_number: i32::from(*card_number),
                        enable_card_locking: 1,
                        poll_stack_size: 0,
                        poll_schedpolicy: 0,
                        logfd: ptr::null_mut(),
                    };
                    check_startup()?;
                    check(unsafe { initialize(&options) }, "cifXDriverInit")?;
                    native.deinit = Some(deinitialize);
                    Some(version)
                } else {
                    None
                };
                check_startup()?;
                check(
                    unsafe { (native.api.driver_open)(&mut native.driver) },
                    "xDriverOpen",
                )?;
                require(
                    !native.driver.is_null(),
                    "cifX returned a null driver handle",
                )?;
                let mut driver_info = [0u8; 36];
                check_startup()?;
                check(
                    unsafe {
                        (native.api.driver_info)(
                            native.driver,
                            driver_info.len() as u32,
                            driver_info.as_mut_ptr().cast(),
                        )
                    },
                    "xDriverGetInformation",
                )?;
                let driver_version = text(&driver_info[..32]);
                require(
                    !driver_version.is_empty(),
                    "cifX driver returned an empty API version",
                )?;
                if let Some(prefix) = &config.expected_version_prefix {
                    require(
                        driver_version.starts_with(prefix),
                        "Installed cifX driver version differs from the expected prefix",
                    )?;
                }
                let mut board = CString::new(config.board.as_str())
                    .map_err(|_| Error::Invalid("Invalid cifX board name".into()))?
                    .into_bytes_with_nul();
                check_startup()?;
                check(
                    unsafe {
                        (native.api.channel_open)(
                            native.driver,
                            board.as_mut_ptr().cast(),
                            config.channel,
                            &mut native.channel,
                        )
                    },
                    "xChannelOpen",
                )?;
                require(
                    !native.channel.is_null(),
                    "cifX returned a null channel handle",
                )?;
                // CHANNEL_INFORMATION is packed in cifXUser.h. Decode by offsets, never unaligned references.
                let mut channel_info = [0u8; 164];
                check_startup()?;
                check(
                    unsafe {
                        (native.api.channel_info)(
                            native.channel,
                            channel_info.len() as u32,
                            channel_info.as_mut_ptr().cast(),
                        )
                    },
                    "xChannelInfo",
                )?;
                let name_length = usize::from(channel_info[48]).min(63);
                let firmware_version = (0..4)
                    .map(|i| {
                        u16::from_ne_bytes(channel_info[40 + i * 2..42 + i * 2].try_into().unwrap())
                            .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(".");
                let info = CifxInfo {
                    driver_version,
                    linux_driver_version,
                    board_count: number(&driver_info, 32),
                    firmware_name: text(&channel_info[49..49 + name_length]),
                    firmware_version,
                    channel_error: number(&channel_info, 116),
                    input_areas: number(&channel_info, 136),
                    output_areas: number(&channel_info, 140),
                };
                check_startup()?;
                let original = native.state()?;
                check_startup()?;
                native.restore_host = Some(u32::from(original.host_ready));
                native.host_state(1)?;
                if config.start_bus {
                    check_startup()?;
                    native.restore_bus = Some(u32::from(original.bus_on));
                    native.bus_state(1)?;
                }
                check_startup()?;
                Ok((native, info))
            }
        }
        fn host_state(&self, command: u32) -> Result<u32> {
            let mut value = 0;
            check(
                unsafe {
                    (self.api.host_state)(self.channel, command, &mut value, self.timeout_ms)
                },
                "xChannelHostState",
            )?;
            Ok(value)
        }
        fn bus_state(&self, command: u32) -> Result<u32> {
            let mut value = 0;
            check(
                unsafe { (self.api.bus_state)(self.channel, command, &mut value, self.timeout_ms) },
                "xChannelBusState",
            )?;
            Ok(value)
        }
        fn state(&self) -> Result<CifxState> {
            Ok(CifxState {
                host_ready: self.host_state(2)? != 0,
                bus_on: self.bus_state(2)? != 0,
            })
        }
        fn set_bus(&mut self, on: bool) -> Result<CifxState> {
            if self.restore_bus.is_none() {
                self.restore_bus = Some(self.bus_state(2)?);
            }
            self.bus_state(u32::from(on))?;
            self.state()
        }
        fn check_range(&self, direction: u32, area: u32, offset: u32, length: u32) -> Result<()> {
            require(
                (1..=1_048_576).contains(&length) && area <= 255,
                "cifX transfer exceeds the area or 1 MiB limit",
            )?;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| Error::Invalid("cifX transfer offset overflows".into()))?;
            let mut information = [0u8; 12];
            check(
                unsafe {
                    (self.api.io_info)(
                        self.channel,
                        direction,
                        area,
                        information.len() as u32,
                        information.as_mut_ptr().cast(),
                    )
                },
                "xChannelIOInfo",
            )?;
            require(
                end <= number(&information, 0),
                "cifX transfer extends beyond the configured process image",
            )
        }
        fn read(&self, request: CifxReadRequest) -> Result<Vec<u8>> {
            self.check_range(1, request.area, request.offset, request.length)?;
            let mut data = vec![0u8; request.length as usize];
            check(
                unsafe {
                    (self.api.read)(
                        self.channel,
                        request.area,
                        request.offset,
                        request.length,
                        data.as_mut_ptr().cast(),
                        self.timeout_ms,
                    )
                },
                "xChannelIORead",
            )?;
            Ok(data)
        }
        fn write(&self, mut request: CifxWriteRequest) -> Result<()> {
            require(request.data.len() <= 1_048_576, "cifX write exceeds 1 MiB")?;
            self.check_range(2, request.area, request.offset, request.data.len() as u32)?;
            check(
                unsafe {
                    (self.api.write)(
                        self.channel,
                        request.area,
                        request.offset,
                        request.data.len() as u32,
                        request.data.as_mut_ptr().cast(),
                        self.timeout_ms,
                    )
                },
                "xChannelIOWrite",
            )
        }
        fn close(&mut self) -> Result<()> {
            let mut failure = None;
            let mut record = |result: Result<()>| {
                if let Err(error) = result {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                }
            };
            if !self.channel.is_null() {
                if let Some(state) = self.restore_bus.take() {
                    record(self.bus_state(state).map(|_| ()));
                }
                if let Some(state) = self.restore_host.take() {
                    record(self.host_state(state).map(|_| ()));
                }
                record(check(
                    unsafe { (self.api.channel_close)(self.channel) },
                    "xChannelClose",
                ));
                self.channel = ptr::null_mut();
            }
            if !self.driver.is_null() {
                record(check(
                    unsafe { (self.api.driver_close)(self.driver) },
                    "xDriverClose",
                ));
                self.driver = ptr::null_mut();
            }
            if let Some(deinitialize) = self.deinit.take() {
                unsafe { deinitialize() };
            }
            failure.map_or(Ok(()), Err)
        }
    }
    impl Drop for NativeChannel {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(&bytes[..bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len())])
            .into_owned()
    }
    fn number(bytes: &[u8], offset: usize) -> u32 {
        u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }
    fn check(code: i32, operation: &str) -> Result<()> {
        require(
            code == 0,
            &format!("cifX {operation} failed with status 0x{:08X}", code as u32),
        )
    }
    enum Command {
        Read(CifxReadRequest, oneshot::Sender<Result<Vec<u8>>>),
        Write(CifxWriteRequest, oneshot::Sender<Result<()>>),
        State(oneshot::Sender<Result<CifxState>>),
        Bus(bool, oneshot::Sender<Result<CifxState>>),
        Close(oneshot::Sender<Result<()>>),
    }
    pub struct CifxClient {
        commands: mpsc::Sender<Command>,
        info: CifxInfo,
        timeout: Duration,
        closed: AtomicBool,
    }
    impl CifxClient {
        pub async fn connect(config: CifxConfig) -> Result<Self> {
            config.validate()?;
            let timeout = Duration::from_millis(u64::from(config.timeout_ms) * 4 + 5000);
            let (commands, mut receiver) = mpsc::channel(32);
            let (ready, result) = oneshot::channel();
            let startup_stop = CancellationToken::new();
            let _cancel_startup = startup_stop.clone().drop_guard();
            std::thread::Builder::new()
                .name("flow-cifx".into())
                .spawn(move || {
                    let (mut native, info) = match NativeChannel::open(&config, &startup_stop) {
                        Ok(result) => result,
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };
                    if ready.send(Ok(info)).is_err() {
                        return;
                    }
                    while let Some(command) = receiver.blocking_recv() {
                        match command {
                            Command::Read(request, reply) if !reply.is_closed() => {
                                let _ = reply.send(native.read(request));
                            }
                            Command::Write(request, reply) if !reply.is_closed() => {
                                let _ = reply.send(native.write(request));
                            }
                            Command::State(reply) if !reply.is_closed() => {
                                let _ = reply.send(native.state());
                            }
                            Command::Bus(on, reply) if !reply.is_closed() => {
                                let _ = reply.send(native.set_bus(on));
                            }
                            Command::Close(reply) => {
                                let result = native.close();
                                drop(native);
                                let _ = reply.send(result);
                                return;
                            }
                            _ => {}
                        }
                    }
                })?;
            let info = tokio::time::timeout(Duration::from_secs(120), result)
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(|_| Error::Invalid("cifX driver worker failed to initialize".into()))??;
            Ok(Self {
                commands,
                info,
                timeout,
                closed: AtomicBool::new(false),
            })
        }
        pub fn info(&self) -> CifxInfo {
            self.info.clone()
        }
        async fn request<T>(
            &self,
            build: impl FnOnce(oneshot::Sender<Result<T>>) -> Command,
        ) -> Result<T> {
            require(
                !self.closed.load(Ordering::Acquire),
                "cifX session is closed",
            )?;
            let (sender, receiver) = oneshot::channel();
            self.commands.try_send(build(sender)).map_err(|_| {
                Error::Invalid("cifX command queue is full or the driver worker stopped".into())
            })?;
            tokio::time::timeout(self.timeout, receiver)
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(|_| Error::Invalid("cifX driver worker stopped".into()))?
        }
        pub async fn read(&self, request: CifxReadRequest) -> Result<Vec<u8>> {
            self.request(|reply| Command::Read(request, reply)).await
        }
        pub async fn write(&self, request: CifxWriteRequest) -> Result<()> {
            self.request(|reply| Command::Write(request, reply)).await
        }
        pub async fn state(&self) -> Result<CifxState> {
            self.request(Command::State).await
        }
        pub async fn set_bus(&self, on: bool) -> Result<CifxState> {
            self.request(|reply| Command::Bus(on, reply)).await
        }
        pub async fn disconnect(&self) -> Result<()> {
            if self.closed.swap(true, Ordering::AcqRel) {
                return Ok(());
            }
            let (reply, result) = oneshot::channel();
            tokio::time::timeout(self.timeout, async {
                self.commands
                    .send(Command::Close(reply))
                    .await
                    .map_err(|_| Error::Invalid("cifX worker already stopped".into()))?;
                result
                    .await
                    .map_err(|_| Error::Invalid("cifX worker stopped while closing".into()))?
            })
            .await
            .map_err(|_| Error::Timeout)?
        }
    }

    #[cfg(all(test, unix))]
    mod tests {
        use super::*;
        use std::{
            mem::{offset_of, size_of},
            process::Command as ProcessCommand,
        };

        struct Fixture {
            directory: PathBuf,
            library: Library,
            path: PathBuf,
        }
        impl Fixture {
            fn build() -> Self {
                static NEXT_FIXTURE: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                let directory = std::env::temp_dir().join(format!(
                    "flow-cifx-abi-{}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir(&directory).unwrap();
                let source = directory.join("driver.c");
                let path = directory.join(if cfg!(target_os = "macos") {
                    "driver.dylib"
                } else {
                    "driver.so"
                });
                std::fs::write(&source, MOCK_DRIVER).unwrap();
                let output = ProcessCommand::new("cc")
                    .arg(if cfg!(target_os = "macos") {
                        "-dynamiclib"
                    } else {
                        "-shared"
                    })
                    .args(["-fPIC", "-std=c11", "-Wall", "-Werror"])
                    .arg(&source)
                    .arg("-o")
                    .arg(&path)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "C fixture compilation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let library = unsafe { Library::new(&path) }.unwrap();
                Self {
                    directory,
                    library,
                    path,
                }
            }
            fn count(&self, field: i32) -> i32 {
                unsafe {
                    self.library
                        .get::<unsafe extern "C" fn(i32) -> i32>(b"mock_count\0")
                        .unwrap()(field)
                }
            }
            fn block(&self, stage: i32) {
                unsafe {
                    self.library
                        .get::<unsafe extern "C" fn(i32)>(b"mock_block\0")
                        .unwrap()(stage);
                }
            }
            fn release(&self) {
                unsafe {
                    self.library
                        .get::<unsafe extern "C" fn()>(b"mock_release\0")
                        .unwrap()();
                }
            }
            fn config(&self) -> CifxConfig {
                CifxConfig {
                    library_path: self.path.to_string_lossy().into_owned(),
                    driver_mode: CifxDriverMode::LinuxV3 {
                        base_directory: self.directory.to_string_lossy().into_owned(),
                        card_number: 7,
                    },
                    board: "cifX0".into(),
                    channel: 1,
                    timeout_ms: 1234,
                    expected_version_prefix: Some("test-api-1".into()),
                    start_bus: true,
                }
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }

        #[tokio::test]
        async fn native_abi_process_image_and_cleanup_match_compiled_c_driver() {
            let fixture = Fixture::build();
            assert_eq!(fixture.count(20) as usize, size_of::<LinuxInitOptions>());
            assert_eq!(
                fixture.count(21) as usize,
                offset_of!(LinuxInitOptions, trace_level)
            );
            assert_eq!(
                fixture.count(22) as usize,
                offset_of!(LinuxInitOptions, card_number)
            );
            assert_eq!(
                fixture.count(23) as usize,
                offset_of!(LinuxInitOptions, logfd)
            );
            let client = CifxClient::connect(fixture.config()).await.unwrap();
            assert_eq!(client.info().driver_version, "test-api-1");
            assert_eq!(client.info().firmware_name, "PN Controller");
            assert_eq!(client.info().firmware_version, "1.2.3.4");
            assert_eq!(client.info().input_areas, 1);
            assert!(client.state().await.unwrap().bus_on);
            assert!(
                CifxClient::connect(fixture.config()).await.is_err(),
                "Concurrent initialization of the process-global driver is forbidden"
            );
            client
                .write(CifxWriteRequest {
                    area: 0,
                    offset: 2,
                    data: vec![9, 8, 7],
                })
                .await
                .unwrap();
            assert_eq!(
                client
                    .read(CifxReadRequest {
                        area: 0,
                        offset: 2,
                        length: 3
                    })
                    .await
                    .unwrap(),
                vec![9, 8, 7]
            );
            assert!(
                client
                    .read(CifxReadRequest {
                        area: 0,
                        offset: 63,
                        length: 2
                    })
                    .await
                    .is_err()
            );
            assert_eq!(
                fixture.count(6),
                1,
                "Out-of-range reads must not reach native IORead"
            );
            assert!(!client.set_bus(false).await.unwrap().bus_on);
            client.disconnect().await.unwrap();
            assert!(client.state().await.is_err());
            assert_eq!(fixture.count(0), 1, "Linux driver initialization");
            assert_eq!(fixture.count(3), 1, "Channel cleanup");
            assert_eq!(fixture.count(4), 1, "Driver cleanup");
            assert_eq!(fixture.count(5), 1, "Linux driver deinitialization");
            assert_eq!(fixture.count(8), 0, "Original host state restored");
            assert_eq!(fixture.count(9), 0, "Original bus state restored");
            assert_eq!(
                fixture.count(10),
                0,
                "C ABI, handle, timeout and cleanup-order checks"
            );
            // A completed close releases the driver lease before its acknowledgment.
            let second = CifxClient::connect(fixture.config()).await.unwrap();
            second.disconnect().await.unwrap();
            assert_eq!(fixture.count(5), 2);
        }

        #[tokio::test]
        async fn partial_open_and_version_failures_release_driver_resources() {
            let fixture = Fixture::build();
            let mut config = fixture.config();
            config.board = "fail".into();
            assert!(CifxClient::connect(config).await.is_err());
            assert_eq!(fixture.count(3), 0);
            assert_eq!(fixture.count(4), 1);
            assert_eq!(fixture.count(5), 1);
            let mut config = fixture.config();
            config.expected_version_prefix = Some("different-api".into());
            assert!(CifxClient::connect(config).await.is_err());
            assert_eq!(
                fixture.count(2),
                1,
                "Version rejection happens before opening the channel"
            );
            assert_eq!(fixture.count(4), 2);
            assert_eq!(fixture.count(5), 2);
            let client = CifxClient::connect(fixture.config()).await.unwrap();
            client.disconnect().await.unwrap();
            assert_eq!(fixture.count(10), 0);
        }

        #[tokio::test]
        async fn cancelled_initialization_never_starts_later_activation_phases() {
            for stage in [1, 2] {
                let fixture = Fixture::build();
                fixture.block(stage);
                let connecting = tokio::spawn(CifxClient::connect(fixture.config()));
                let entered = tokio::time::timeout(Duration::from_secs(5), async {
                    while fixture.count(13) != stage {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await;
                connecting.abort();
                let _ = connecting.await;
                fixture.release();
                entered.expect("Native initialization did not reach the blocked phase");
                tokio::time::timeout(Duration::from_secs(5), async {
                    while fixture.count(5) != 1 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .expect("Cancelled initialization did not release the driver");
                assert_eq!(
                    fixture.count(11),
                    i32::from(stage == 2),
                    "Host activation after cancellation"
                );
                assert_eq!(fixture.count(12), 0, "Bus activation after cancellation");
                assert_eq!(fixture.count(8), 0, "Original host state restored");
                assert_eq!(fixture.count(9), 0, "Original bus state restored");
                assert_eq!(fixture.count(3), 1, "Channel closed");
                assert_eq!(fixture.count(4), 1, "Driver closed");
                assert_eq!(fixture.count(10), 0, "Native cleanup ordering");
            }
        }

        const MOCK_DRIVER: &str = r#"
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>
#include <stdatomic.h>
#include <sched.h>
typedef struct {
  int init_options; const char* base_dir; unsigned long poll_interval;
  int poll_priority; unsigned long trace_level; int user_card_cnt; void* user_cards;
  int iCardNumber; int fEnableCardLocking; int poll_StackSize; int poll_schedpolicy; FILE* logfd;
} CIFX_LINUX_INIT;
static _Atomic int counts[24], blocked_stage, released;
static int driver_open, channel_open;
static _Atomic uint32_t host, bus;
static unsigned char image[64];
void mock_block(int stage) { blocked_stage=stage; released=0; counts[13]=0; }
void mock_release(void) { released=1; }
static void wait_at(int stage) {
  if (blocked_stage != stage) return;
  counts[13]=stage;
  while (!released) sched_yield();
}
static int32_t verify(int condition) { if (!condition) { counts[10]++; return -1; } return 0; }
static void put32(unsigned char* data, size_t offset, uint32_t value) { memcpy(data+offset,&value,4); }
int mock_count(int field) {
  if (field==8) return host;
  if (field==9) return bus;
  if (field==20) return sizeof(CIFX_LINUX_INIT);
  if (field==21) return offsetof(CIFX_LINUX_INIT,trace_level);
  if (field==22) return offsetof(CIFX_LINUX_INIT,iCardNumber);
  if (field==23) return offsetof(CIFX_LINUX_INIT,logfd);
  return counts[field];
}
int32_t cifXGetDriverVersion(uint32_t length, char* version) {
  if (verify(length >= 22)) return -1;
  strcpy(version,"LinuxCIFXDrv 3.1.0"); return 0;
}
int32_t cifXDriverInit(const CIFX_LINUX_INIT* options) {
  if (verify(options && options->init_options==2 && options->base_dir && options->base_dir[0]=='/' &&
    options->poll_interval==0 && options->poll_priority==0 && options->trace_level==0 &&
    options->user_card_cnt==0 && !options->user_cards && options->iCardNumber==7 &&
    options->fEnableCardLocking==1 && options->poll_StackSize==0 && options->poll_schedpolicy==0 && !options->logfd)) return -1;
  counts[0]++; return 0;
}
void cifXDriverDeinit(void) { verify(!driver_open && !channel_open); counts[5]++; }
int32_t xDriverOpen(void** handle) {
  if (verify(!driver_open && handle)) return -1;
  driver_open=1; *handle=(void*)0x11; counts[1]++; return 0;
}
int32_t xDriverClose(void* handle) {
  if (verify(handle==(void*)0x11 && driver_open && !channel_open)) return -1;
  driver_open=0; counts[4]++; return 0;
}
int32_t xDriverGetInformation(void* handle,uint32_t size,void* info) {
  if (verify(handle==(void*)0x11 && size==36 && info)) return -1;
  memset(info,0,size); strcpy(info,"test-api-1"); put32(info,32,1); return 0;
}
int32_t xChannelOpen(void* driver,char* board,uint32_t channel,void** handle) {
  counts[2]++;
  if (verify(driver==(void*)0x11 && driver_open && board && channel==1 && handle)) return -1;
  if (!strcmp(board,"fail")) return -2;
  if (verify(!strcmp(board,"cifX0") && !channel_open)) return -1;
  channel_open=1; *handle=(void*)0x22; return 0;
}
int32_t xChannelClose(void* handle) {
  if (verify(handle==(void*)0x22 && channel_open && !host && !bus)) return -1;
  channel_open=0; counts[3]++; return 0;
}
int32_t xChannelInfo(void* handle,uint32_t size,void* info) {
  if (verify(handle==(void*)0x22 && channel_open && size==164 && info)) return -1;
  wait_at(1);
  unsigned char* bytes=info; uint16_t version[4]={1,2,3,4};
  memset(info,0,size); memcpy(bytes+40,version,8); bytes[48]=13;
  memcpy(bytes+49,"PN Controller",13); put32(bytes,136,1); put32(bytes,140,1); return 0;
}
int32_t xChannelHostState(void* handle,uint32_t command,uint32_t* value,uint32_t timeout) {
  if (verify(handle==(void*)0x22 && channel_open && command<=2 && value && timeout==1234)) return -1;
  if (command==1) { counts[11]++; wait_at(2); }
  if (command<2) host=command;
  *value=host; return 0;
}
int32_t xChannelBusState(void* handle,uint32_t command,uint32_t* value,uint32_t timeout) {
  if (verify(handle==(void*)0x22 && channel_open && command<=2 && value && timeout==1234)) return -1;
  if (command==1) counts[12]++;
  if (command<2) bus=command;
  *value=bus; return 0;
}
int32_t xChannelIOInfo(void* handle,uint32_t direction,uint32_t area,uint32_t size,void* info) {
  if (verify(handle==(void*)0x22 && channel_open && (direction==1||direction==2) && area==0 && size==12 && info)) return -1;
  memset(info,0,size); put32(info,0,64); return 0;
}
int32_t xChannelIORead(void* handle,uint32_t area,uint32_t offset,uint32_t length,void* data,uint32_t timeout) {
  if (verify(handle==(void*)0x22 && channel_open && area==0 && offset+length<=64 && data && timeout==1234)) return -1;
  memcpy(data,image+offset,length); counts[6]++; return 0;
}
int32_t xChannelIOWrite(void* handle,uint32_t area,uint32_t offset,uint32_t length,void* data,uint32_t timeout) {
  if (verify(handle==(void*)0x22 && channel_open && area==0 && offset+length<=64 && data && timeout==1234)) return -1;
  memcpy(image+offset,data,length); counts[7]++; return 0;
}
"#;
    }
}
