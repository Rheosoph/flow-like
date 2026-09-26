use enigo::{Axis, Button, Coordinate, Direction, Enigo, InputResult, Key, Keyboard, Mouse};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub(super) trait NativeBackend: Keyboard + Mouse {
    fn live(&mut self) -> bool {
        true
    }
}
impl NativeBackend for Enigo {}
type Job = Box<dyn FnOnce(&mut dyn NativeBackend) + Send>;
fn backend(settings: &enigo::Settings) -> flow_like_types::Result<Box<dyn NativeBackend>> {
    #[cfg(target_os = "linux")]
    if wayland() {
        return Ok(Box::new(super::portal::PortalInput::new()?));
    }
    Ok(Box::new(Enigo::new(settings)?))
}
struct Worker {
    sender: mpsc::Sender<Job>,
    alive: Arc<AtomicBool>,
}
static INPUT: OnceLock<Mutex<Option<Arc<Worker>>>> = OnceLock::new();
static START: Mutex<()> = Mutex::new(());
static ACTION: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
fn slot() -> &'static Mutex<Option<Arc<Worker>>> {
    INPUT.get_or_init(|| Mutex::new(None))
}
pub(crate) fn wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
            || std::env::var_os("WAYLAND_DISPLAY").is_some())
}
pub fn input_capability_granted() -> bool {
    slot()
        .lock()
        .ok()
        .and_then(|v| v.clone())
        .is_some_and(|w| w.alive.load(Ordering::Acquire))
}
fn start() -> flow_like_types::Result<Arc<Worker>> {
    let _start = START
        .lock()
        .map_err(|_| flow_like_types::anyhow!("Input connection lock unavailable"))?;
    if let Some(worker) = slot()
        .lock()
        .map_err(|_| flow_like_types::anyhow!("Input state unavailable"))?
        .clone()
        .filter(|w| w.alive.load(Ordering::Acquire))
    {
        return Ok(worker);
    }
    let (sender, receiver) = mpsc::channel::<Job>();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let alive = Arc::new(AtomicBool::new(true));
    let live = alive.clone();
    std::thread::Builder::new()
        .name("automation-input".into())
        .spawn(move || {
            let settings = enigo::Settings {
                open_prompt_to_get_permissions: false,
                ..Default::default()
            };
            match backend(&settings) {
                Ok(mut input) => {
                    if ready_tx.send(Ok(())).is_ok() {
                        loop {
                            if !live.load(Ordering::Acquire) || !input.live() {
                                break;
                            }
                            match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
                                Ok(job) => job(input.as_mut()),
                                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error.to_string()));
                }
            }
            live.store(false, Ordering::Release);
        })?;
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(120))
        .map_err(|_| flow_like_types::anyhow!("Input authorization timed out"))?
        .map_err(|e| flow_like_types::anyhow!("Input control unavailable: {}", e))?;
    let worker = Arc::new(Worker { sender, alive });
    *slot()
        .lock()
        .map_err(|_| flow_like_types::anyhow!("Input state unavailable"))? = Some(worker.clone());
    Ok(worker)
}
/// Establishes one input connection. On Wayland this opens the native portal grant dialog.
pub async fn request_input_capability() -> flow_like_types::Result<()> {
    tokio::task::spawn_blocking(|| start().map(|_| ())).await?
}
pub fn release_input_capability() {
    if let Ok(mut slot) = slot().lock() {
        if let Some(worker) = slot.take() {
            worker.alive.store(false, Ordering::Release);
        }
    }
}

/// Input commands stay on the thread that owns the native/portal connection.
pub struct DesktopInput {
    worker: Arc<Worker>,
    session_active: Arc<AtomicBool>,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
    buttons: Vec<Button>,
    keys: Vec<Key>,
    _operation: tokio::sync::OwnedMutexGuard<()>,
}
impl DesktopInput {
    pub async fn new(
        session_active: Arc<AtomicBool>,
        cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
    ) -> flow_like_types::Result<Self> {
        let worker = if wayland() {
            slot()
                .lock()
                .map_err(|_| flow_like_types::anyhow!("Input state unavailable"))?
                .clone()
                .filter(|w| w.alive.load(Ordering::Acquire))
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Grant Wayland input control before running automation"
                    )
                })?
        } else {
            tokio::task::spawn_blocking(start).await??
        };
        let operation = ACTION
            .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
            .lock_owned()
            .await;
        Ok(Self {
            worker,
            session_active,
            cancellation,
            buttons: vec![],
            keys: vec![],
            _operation: operation,
        })
    }
    fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut dyn NativeBackend) -> InputResult<T> + Send + 'static,
    ) -> InputResult<T> {
        if !self.worker.alive.load(Ordering::Acquire) {
            return Err(enigo::InputError::Simulate("Native input grant ended"));
        }
        if !self.session_active.load(Ordering::Acquire) {
            return Err(enigo::InputError::Simulate("Automation session is closed"));
        }
        let cancellation = self.cancellation.clone();
        let active = self.session_active.clone();
        self.call_cleanup(move |input| {
            if cancellation
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
            {
                return Err(enigo::InputError::Simulate("Automation cancelled"));
            }
            if !active.load(Ordering::Acquire) {
                return Err(enigo::InputError::Simulate("Automation session is closed"));
            }
            operation(input)
        })
    }
    fn call_cleanup<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut dyn NativeBackend) -> InputResult<T> + Send + 'static,
    ) -> InputResult<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.worker
            .sender
            .send(Box::new(move |input| {
                let _ = tx.send(operation(input));
            }))
            .map_err(|_| enigo::InputError::Simulate("Native input connection ended"))?;
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| {
                self.worker.alive.store(false, Ordering::Release);
                enigo::InputError::Simulate("Native input operation timed out")
            })?;
        result
    }
    pub fn modifiers(&mut self, text: &str) -> flow_like_types::Result<()> {
        let mut keys = Vec::new();
        for value in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let key = match value.to_lowercase().as_str() {
                "ctrl" | "control" => Key::Control,
                "shift" => Key::Shift,
                "alt" | "option" => Key::Alt,
                "meta" | "cmd" | "command" | "win" => Key::Meta,
                _ => return Err(flow_like_types::anyhow!("Unknown modifier: {}", value)),
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        for key in keys {
            self.key(key, Direction::Press)?;
        }
        Ok(())
    }
}
impl Keyboard for DesktopInput {
    fn fast_text(&mut self, text: &str) -> InputResult<Option<()>> {
        let mut chars = text.chars().peekable();
        while chars.peek().is_some() {
            let chunk: String = chars.by_ref().take(32).collect();
            self.call(move |input| input.text(&chunk))?;
        }
        Ok(Some(()))
    }
    fn key(&mut self, key: Key, direction: Direction) -> InputResult<()> {
        self.call(move |i| i.key(key, direction))?;
        if direction == Direction::Press && !self.keys.contains(&key) {
            self.keys.push(key);
        }
        if direction == Direction::Release {
            self.keys.retain(|k| *k != key);
        }
        Ok(())
    }
    fn raw(&mut self, keycode: u16, direction: Direction) -> InputResult<()> {
        self.call(move |i| i.raw(keycode, direction))
    }
}
impl Mouse for DesktopInput {
    fn button(&mut self, button: Button, direction: Direction) -> InputResult<()> {
        self.call(move |i| i.button(button, direction))?;
        if direction == Direction::Press && !self.buttons.contains(&button) {
            self.buttons.push(button);
        }
        if direction == Direction::Release {
            self.buttons.retain(|b| *b != button);
        }
        Ok(())
    }
    fn move_mouse(&mut self, x: i32, y: i32, coordinate: Coordinate) -> InputResult<()> {
        #[cfg(target_os = "windows")]
        return self.call(move |input| {
            let (x, y) = if coordinate == Coordinate::Rel {
                let (cx, cy) = input.location()?;
                (cx.saturating_add(x), cy.saturating_add(y))
            } else {
                (x, y)
            };
            unsafe { ::windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y) }
                .map_err(|_| enigo::InputError::Simulate("Windows denied cursor movement"))
        });
        #[cfg(not(target_os = "windows"))]
        self.call(move |i| i.move_mouse(x, y, coordinate))
    }
    fn scroll(&mut self, length: i32, axis: Axis) -> InputResult<()> {
        self.call(move |i| i.scroll(length, axis))
    }
    fn main_display(&self) -> InputResult<(i32, i32)> {
        self.call(|i| i.main_display())
    }
    fn location(&self) -> InputResult<(i32, i32)> {
        self.call(|i| i.location())
    }
}
impl Drop for DesktopInput {
    fn drop(&mut self) {
        for button in std::mem::take(&mut self.buttons) {
            let _ = self.call_cleanup(move |i| i.button(button, Direction::Release));
        }
        for key in std::mem::take(&mut self.keys).into_iter().rev() {
            let _ = self.call_cleanup(move |i| i.key(key, Direction::Release));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn isolated_input(
        active: bool,
        cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
    ) -> (DesktopInput, mpsc::Receiver<Job>) {
        let (sender, receiver) = mpsc::channel();
        (
            DesktopInput {
                worker: Arc::new(Worker {
                    sender,
                    alive: Arc::new(AtomicBool::new(true)),
                }),
                session_active: Arc::new(AtomicBool::new(active)),
                cancellation,
                buttons: vec![],
                keys: vec![],
                _operation: Arc::new(tokio::sync::Mutex::new(())).lock_owned().await,
            },
            receiver,
        )
    }

    #[tokio::test]
    async fn closed_session_never_queues_input() {
        let (mut input, receiver) = isolated_input(false, None).await;
        assert!(input.button(Button::Left, Direction::Click).is_err());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(input.worker.alive.load(Ordering::Acquire));
    }

    struct Counter(usize);
    impl Keyboard for Counter {
        fn fast_text(&mut self, _: &str) -> InputResult<Option<()>> {
            self.0 += 1;
            Ok(Some(()))
        }
        fn key(&mut self, _: Key, _: Direction) -> InputResult<()> {
            self.0 += 1;
            Ok(())
        }
        fn raw(&mut self, _: u16, _: Direction) -> InputResult<()> {
            self.0 += 1;
            Ok(())
        }
    }
    impl Mouse for Counter {
        fn button(&mut self, _: Button, _: Direction) -> InputResult<()> {
            self.0 += 1;
            Ok(())
        }
        fn move_mouse(&mut self, _: i32, _: i32, _: Coordinate) -> InputResult<()> {
            self.0 += 1;
            Ok(())
        }
        fn scroll(&mut self, _: i32, _: Axis) -> InputResult<()> {
            self.0 += 1;
            Ok(())
        }
        fn main_display(&self) -> InputResult<(i32, i32)> {
            Ok((0, 0))
        }
        fn location(&self) -> InputResult<(i32, i32)> {
            Ok((0, 0))
        }
    }
    impl NativeBackend for Counter {}

    #[tokio::test]
    async fn cancellation_suppresses_queued_commands_but_releases_held_input() {
        let token = flow_like_types::tokio_util::sync::CancellationToken::new();
        let (mut input, receiver) = isolated_input(true, Some(token.clone())).await;
        input.buttons.push(Button::Left);
        token.cancel();
        let worker = std::thread::spawn(move || {
            let mut counter = Counter(0);
            while let Ok(job) = receiver.recv() {
                job(&mut counter);
            }
            counter.0
        });
        assert!(input.button(Button::Left, Direction::Click).is_err());
        assert!(input.worker.alive.load(Ordering::Acquire));
        drop(input);
        assert_eq!(worker.join().unwrap(), 1);
    }
}
