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
    fn ready(&self) -> InputResult<()> {
        if !self.worker.alive.load(Ordering::Acquire) {
            return Err(enigo::InputError::Simulate("Native input grant ended"));
        }
        if !self.session_active.load(Ordering::Acquire) {
            return Err(enigo::InputError::Simulate("Automation session is closed"));
        }
        Ok(())
    }
    fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut dyn NativeBackend) -> InputResult<T> + Send + 'static,
    ) -> InputResult<T> {
        self.ready()?;
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
        run_job(&self.worker, OPERATION_TIMEOUT, operation)
    }
    /// Releases every button and key this handle still holds. When the input connection hung or
    /// ended, the releases go through a replacement connection so modifiers are never left down.
    fn release_held(
        &mut self,
        timeout: std::time::Duration,
        replacement: impl FnOnce() -> flow_like_types::Result<Arc<Worker>>,
    ) {
        let buttons = std::mem::take(&mut self.buttons);
        let keys = std::mem::take(&mut self.keys);
        if buttons.is_empty() && keys.is_empty() {
            return;
        }
        if self.worker.alive.load(Ordering::Acquire) {
            let result = run_job(
                &self.worker,
                timeout,
                release_job(buttons.clone(), keys.clone()),
            );
            if self.worker.alive.load(Ordering::Acquire) {
                if let Err(error) = result {
                    tracing::warn!("Releasing held input reported an error: {}", error);
                }
                return;
            }
        }
        let released = replacement().and_then(|worker| {
            run_job(&worker, timeout, release_job(buttons, keys))
                .map_err(|error| flow_like_types::anyhow!("{}", error))
        });
        if let Err(error) = released {
            tracing::error!(
                "Held keys or mouse buttons could not be released after the input connection failed: {}",
                error
            );
        }
    }
    /// Keeps `button` pressed after this handle drops (Mouse Down). It is released by Mouse Up,
    /// or automatically when the session closes or the run is cancelled.
    pub fn latch_button(&mut self, button: Button) {
        self.buttons.retain(|held| *held != button);
        if let Ok(mut latched) = LATCHED.lock()
            && !latched.contains(&button)
        {
            latched.push(button);
        }
        let active = self.session_active.clone();
        let cancellation = self.cancellation.clone();
        let spawned = std::thread::Builder::new()
            .name("automation-latched-button".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    if !is_latched(button) {
                        return;
                    }
                    if !active.load(Ordering::Acquire)
                        || cancellation
                            .as_ref()
                            .is_some_and(|token| token.is_cancelled())
                    {
                        release_latched(button);
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("Latched mouse button has no automatic release: {}", error);
        }
    }
    /// Forgets a latch after the button was released explicitly.
    pub fn unlatch_button(&mut self, button: Button) {
        if let Ok(mut latched) = LATCHED.lock() {
            latched.retain(|held| *held != button);
        }
    }
    pub fn modifiers(&mut self, text: &str) -> flow_like_types::Result<()> {
        let mut keys = Vec::new();
        for value in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let key = match value.to_lowercase().as_str() {
                "ctrl" | "control" => Key::Control,
                "shift" => Key::Shift,
                "alt" | "option" => Key::Alt,
                "meta" | "cmd" | "command" | "super" | "win" | "windows" => Key::Meta,
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
        self.ready()?;
        let tracked = track_press(&mut self.keys, key, direction);
        let result = self.call(move |i| i.key(key, direction));
        untrack_release(&mut self.keys, key, direction, tracked, result.is_ok());
        result
    }
    fn raw(&mut self, keycode: u16, direction: Direction) -> InputResult<()> {
        self.call(move |i| i.raw(keycode, direction))
    }
}
impl Mouse for DesktopInput {
    fn button(&mut self, button: Button, direction: Direction) -> InputResult<()> {
        self.ready()?;
        let tracked = track_press(&mut self.buttons, button, direction);
        let result = self.call(move |i| i.button(button, direction));
        untrack_release(
            &mut self.buttons,
            button,
            direction,
            tracked,
            result.is_ok(),
        );
        result
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
        self.release_held(RELEASE_TIMEOUT, || {
            if wayland() {
                return Err(flow_like_types::anyhow!(
                    "the Wayland input grant ended; the compositor releases held input when its session closes"
                ));
            }
            start()
        });
    }
}

/// Tracks a press before it is sent, so a press whose outcome is unknown (timeout, dropped
/// connection) is still released. Returns whether this call started tracking it.
fn track_press<T: PartialEq + Copy>(held: &mut Vec<T>, input: T, direction: Direction) -> bool {
    let starts = direction != Direction::Release && !held.contains(&input);
    if starts {
        held.push(input);
    }
    starts
}

fn untrack_release<T: PartialEq + Copy>(
    held: &mut Vec<T>,
    input: T,
    direction: Direction,
    tracked: bool,
    succeeded: bool,
) {
    let completed = match direction {
        Direction::Release => succeeded,
        Direction::Click => succeeded && tracked,
        Direction::Press => false,
    };
    if completed {
        held.retain(|held| *held != input);
    }
}

static LATCHED: Mutex<Vec<Button>> = Mutex::new(Vec::new());

fn is_latched(button: Button) -> bool {
    LATCHED
        .lock()
        .map(|latched| latched.contains(&button))
        .unwrap_or(false)
}

fn release_latched(button: Button) {
    if let Ok(mut latched) = LATCHED.lock() {
        latched.retain(|held| *held != button);
    }
    let worker = if wayland() {
        slot()
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .filter(|worker| worker.alive.load(Ordering::Acquire))
            .ok_or_else(|| flow_like_types::anyhow!("the Wayland input grant ended"))
    } else {
        start()
    };
    let released = worker.and_then(|worker| {
        run_job(&worker, RELEASE_TIMEOUT, move |input| {
            input.button(button, Direction::Release)
        })
        .map_err(|error| flow_like_types::anyhow!("{}", error))
    });
    if let Err(error) = released {
        tracing::error!(
            "Mouse button {:?} held by Mouse Down could not be released: {}",
            button,
            error
        );
    }
}

const OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const RELEASE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

fn run_job<T: Send + 'static>(
    worker: &Worker,
    timeout: std::time::Duration,
    operation: impl FnOnce(&mut dyn NativeBackend) -> InputResult<T> + Send + 'static,
) -> InputResult<T> {
    let (tx, rx) = mpsc::sync_channel(1);
    if worker
        .sender
        .send(Box::new(move |input| {
            let _ = tx.send(operation(input));
        }))
        .is_err()
    {
        worker.alive.store(false, Ordering::Release);
        return Err(enigo::InputError::Simulate("Native input connection ended"));
    }
    rx.recv_timeout(timeout).map_err(|_| {
        worker.alive.store(false, Ordering::Release);
        enigo::InputError::Simulate("Native input operation timed out")
    })?
}

fn release_job(
    buttons: Vec<Button>,
    keys: Vec<Key>,
) -> impl FnOnce(&mut dyn NativeBackend) -> InputResult<()> + Send + 'static {
    move |input| {
        let mut result = Ok(());
        for button in buttons {
            if let Err(error) = input.button(button, Direction::Release) {
                result = Err(error);
            }
        }
        for key in keys.into_iter().rev() {
            if let Err(error) = input.key(key, Direction::Release) {
                result = Err(error);
            }
        }
        result
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

    #[test]
    fn presses_with_unknown_outcome_stay_tracked_for_release() {
        let mut held = vec![];
        assert!(track_press(&mut held, Key::Shift, Direction::Press));
        untrack_release(&mut held, Key::Shift, Direction::Press, true, false);
        assert_eq!(held, vec![Key::Shift]);
        let tracked = track_press(&mut held, Key::Tab, Direction::Click);
        untrack_release(&mut held, Key::Tab, Direction::Click, tracked, false);
        assert_eq!(held, vec![Key::Shift, Key::Tab]);
        let tracked = track_press(&mut held, Key::Shift, Direction::Click);
        untrack_release(&mut held, Key::Shift, Direction::Click, tracked, true);
        assert!(held.contains(&Key::Shift));
        untrack_release(&mut held, Key::Tab, Direction::Release, false, true);
        assert_eq!(held, vec![Key::Shift]);
    }

    fn counting_worker() -> (Arc<Worker>, std::thread::JoinHandle<usize>) {
        let (sender, receiver) = mpsc::channel::<Job>();
        let thread = std::thread::spawn(move || {
            let mut counter = Counter(0);
            while let Ok(job) = receiver.recv() {
                job(&mut counter);
            }
            counter.0
        });
        (
            Arc::new(Worker {
                sender,
                alive: Arc::new(AtomicBool::new(true)),
            }),
            thread,
        )
    }

    #[tokio::test]
    async fn dead_worker_releases_held_input_through_a_replacement() {
        let (mut input, receiver) = isolated_input(true, None).await;
        input.buttons.push(Button::Left);
        input.keys.extend([Key::Shift, Key::Control]);
        input.worker.alive.store(false, Ordering::Release);
        let (replacement, thread) = counting_worker();
        input.release_held(std::time::Duration::from_secs(2), move || Ok(replacement));
        assert_eq!(thread.join().unwrap(), 3);
        assert!(receiver.try_recv().is_err());
        assert!(input.buttons.is_empty() && input.keys.is_empty());
    }

    #[tokio::test]
    async fn hung_worker_times_out_and_releases_through_a_replacement() {
        let (mut input, _receiver) = isolated_input(true, None).await;
        input.keys.push(Key::Meta);
        let (replacement, thread) = counting_worker();
        input.release_held(std::time::Duration::from_millis(50), move || {
            Ok(replacement)
        });
        assert!(!input.worker.alive.load(Ordering::Acquire));
        assert_eq!(thread.join().unwrap(), 1);
    }

    #[tokio::test]
    async fn responsive_worker_releases_without_a_replacement() {
        let (mut input, receiver) = isolated_input(true, None).await;
        input.keys.push(Key::Alt);
        let thread = std::thread::spawn(move || {
            let mut counter = Counter(0);
            while let Ok(job) = receiver.recv() {
                job(&mut counter);
            }
            counter.0
        });
        input.release_held(std::time::Duration::from_secs(2), || {
            Err(flow_like_types::anyhow!("replacement must not be used"))
        });
        drop(input);
        assert_eq!(thread.join().unwrap(), 1);
    }
}
