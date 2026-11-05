//! The browser and the computer of a bot on a device (`browser_*`, `computer_*`): backends for
//! `flow-like-bot-tools`, one worker process per bot with an optional virtual display, the
//! owner's live frames and takeover.

mod desk_lock;
mod display;
mod frames;
mod guard;
mod host;
mod input;
mod local;
mod playground;
mod rpc;
mod takeover;
mod wire;
mod worker;

pub use desk_lock::{DESK_LEASE, DesktopLock, WAIT_FOR_DESK};
pub use display::{DisplayKind, DisplayPolicy, Facts, VirtualDisplay, resolve_display};
pub use frames::{FRAME_MAX_WIDTH, Frame, FrameKind, FrameStream, Shot};
pub use guard::{BotChannel, GuardedBrowser, GuardedComputer, NoDisplay};
pub use host::{Machine, MachineHost, Mode};
pub use input::InputEvent;
pub use local::{LocalPlayground, bot_dir};
pub use playground::Playground;
pub use rpc::{Connection, RemotePlayground, Spawner, Transport, serve};
pub use takeover::Takeover;
pub use worker::{ProcessSpawner, WORKER_SUBCOMMAND, WorkerArgs, run_worker, worker_command};
