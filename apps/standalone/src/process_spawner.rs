//! Starts supervised children from a thread that lives as long as the agent.

use std::{
    io,
    sync::{Mutex, mpsc},
};
use tokio::process::{Child, Command};

enum Answer {
    Async(tokio::sync::oneshot::Sender<io::Result<Child>>),
    Blocking(mpsc::SyncSender<io::Result<Child>>),
}

impl Answer {
    fn send(self, child: io::Result<Child>) {
        match self {
            Self::Async(answer) => {
                let _ = answer.send(child);
            }
            Self::Blocking(answer) => {
                let _ = answer.send(child);
            }
        }
    }
}

struct Spawn {
    command: Command,
    runtime: tokio::runtime::Handle,
    answer: Answer,
}

static SPAWNER: Mutex<Option<mpsc::Sender<Spawn>>> = Mutex::new(None);

fn stopped() -> io::Error {
    io::Error::other("The process spawner thread stopped")
}

/// Linux's parent-death signal follows the thread that forked the child. Tokio retires
/// worker threads while the agent runs, so every supervised child uses this spawner.
pub(crate) async fn spawn(command: Command) -> io::Result<Child> {
    let (answer, answered) = tokio::sync::oneshot::channel();
    submit(command, Answer::Async(answer))?;
    answered.await.map_err(|_| stopped())?
}

/// Keeps the caller's inherited descriptors alive until the child has forked and execed.
pub(crate) fn spawn_blocking(command: Command) -> io::Result<Child> {
    let (answer, answered) = mpsc::sync_channel(1);
    submit(command, Answer::Blocking(answer))?;
    answered.recv().map_err(|_| stopped())?
}

fn submit(command: Command, answer: Answer) -> io::Result<()> {
    let runtime = tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
    spawner()?
        .send(Spawn {
            command,
            runtime,
            answer,
        })
        .map_err(|_| stopped())
}

fn spawner() -> io::Result<mpsc::Sender<Spawn>> {
    let mut spawner = SPAWNER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(sender) = spawner.as_ref() {
        return Ok(sender.clone());
    }
    let (sender, spawns) = mpsc::channel();
    std::thread::Builder::new()
        .name("agent-process-spawner".into())
        .spawn(move || serve_spawns(spawns))?;
    *spawner = Some(sender.clone());
    Ok(sender)
}

fn serve_spawns(spawns: mpsc::Receiver<Spawn>) {
    for Spawn {
        mut command,
        runtime,
        answer,
    } in spawns
    {
        let _context = runtime.enter();
        let child = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| command.spawn()))
            .unwrap_or_else(|_| Err(io::Error::other("Starting the child process panicked")));
        answer.send(child);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn children_outlive_the_thread_that_asked_for_them() -> anyhow::Result<()> {
        for blocking in [false, true] {
            let runtime = tokio::runtime::Handle::current();
            let asking = std::thread::spawn(move || -> io::Result<Child> {
                let _context = runtime.enter();
                let mut command = Command::new("sleep");
                command.arg("30").kill_on_drop(true);
                #[cfg(target_os = "linux")]
                // SAFETY: prctl is async-signal-safe and only sets this child's death signal.
                unsafe {
                    command.pre_exec(|| {
                        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                if blocking {
                    spawn_blocking(command)
                } else {
                    runtime.block_on(spawn(command))
                }
            });
            let mut child = asking
                .join()
                .map_err(|_| anyhow::anyhow!("the asking thread panicked"))??;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let ended = child.try_wait()?;
            child.start_kill()?;
            child.wait().await?;
            assert!(
                ended.is_none(),
                "the child died with its requesting thread (blocking={blocking})"
            );
        }
        Ok(())
    }
}
