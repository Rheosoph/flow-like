use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use dashmap::DashMap;
use flow_like_types::{Result, anyhow};
use flow_like_bot_tools::computer::{
    ComputerAction, ComputerBackend, PointerButton, Screen, ScrollDirection,
};
use flow_like_catalog_automation::bot_driver::{BotComputer, BotPointerButton, BotScrollDirection};

use super::view::{self, FrameKind};

static COMPUTERS: LazyLock<DashMap<String, Arc<BotComputer>>> = LazyLock::new(DashMap::new);

pub(super) fn session(bot_id: &str) -> Arc<DesktopComputer> {
    let computer = COMPUTERS
        .entry(bot_id.to_string())
        .or_insert_with(|| Arc::new(BotComputer::new()))
        .clone();
    Arc::new(DesktopComputer {
        bot_id: bot_id.to_string(),
        computer,
    })
}

pub(super) fn stop(bot_id: &str) {
    if let Some(computer) = COMPUTERS.get(bot_id) {
        computer.stop();
    }
}

pub(crate) struct DesktopComputer {
    bot_id: String,
    computer: Arc<BotComputer>,
}

fn button(button: PointerButton) -> BotPointerButton {
    match button {
        PointerButton::Left => BotPointerButton::Left,
        PointerButton::Right => BotPointerButton::Right,
        PointerButton::Middle => BotPointerButton::Middle,
    }
}

fn direction(direction: ScrollDirection) -> BotScrollDirection {
    match direction {
        ScrollDirection::Up => BotScrollDirection::Up,
        ScrollDirection::Down => BotScrollDirection::Down,
        ScrollDirection::Left => BotScrollDirection::Left,
        ScrollDirection::Right => BotScrollDirection::Right,
    }
}

#[async_trait]
impl ComputerBackend for DesktopComputer {
    async fn screenshot(&self) -> Result<Screen> {
        let shot = self.computer.screenshot().await?;
        view::publish(
            &self.bot_id,
            FrameKind::Computer,
            "Screen",
            shot.png.clone(),
        );
        Ok(Screen {
            png: shot.png,
            width: shot.width,
            height: shot.height,
        })
    }

    async fn act(&self, action: &ComputerAction) -> Result<String> {
        let done = match action {
            ComputerAction::Click {
                x,
                y,
                button: pointer,
                count,
            } => self.computer.click(*x, *y, button(*pointer), *count).await,
            ComputerAction::Move { x, y } => self.computer.move_to(*x, *y).await,
            ComputerAction::Type { text } => self.computer.type_text(text).await,
            ComputerAction::Key { chord } => self.computer.press(chord).await,
            ComputerAction::Scroll {
                x,
                y,
                direction: toward,
                ticks,
            } => {
                self.computer
                    .scroll(*x, *y, direction(*toward), *ticks)
                    .await
            }
        };
        done.map_err(|error| anyhow!("{error:#}"))
    }
}
