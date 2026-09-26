use super::input::NativeBackend;
use ashpd::desktop::{
    PersistMode, Session,
    remote_desktop::{Axis as PortalAxis, DeviceType, KeyState, RemoteDesktop},
    screencast::{CursorMode, Screencast, SourceType, Stream},
};
use enigo::{Axis, Button, Coordinate, Direction, InputError, InputResult, Key, Keyboard, Mouse};
use futures::{FutureExt, StreamExt, executor::block_on};

pub struct PortalInput {
    desktop: RemoteDesktop<'static>,
    session: Session<'static, RemoteDesktop<'static>>,
    streams: Vec<Stream>,
    closed: std::pin::Pin<Box<dyn futures::Stream<Item = ()> + Send>>,
}
impl PortalInput {
    pub fn new() -> flow_like_types::Result<Self> {
        block_on(async {
            let desktop = RemoteDesktop::new().await?;
            let session = desktop.create_session().await?;
            let setup: flow_like_types::Result<_> = async {
                desktop
                    .select_devices(
                        &session,
                        DeviceType::Keyboard | DeviceType::Pointer,
                        None,
                        PersistMode::DoNot,
                    )
                    .await?
                    .response()?;
                Screencast::new()
                    .await?
                    .select_sources(
                        &session,
                        CursorMode::Hidden,
                        SourceType::Monitor.into(),
                        true,
                        None,
                        PersistMode::DoNot,
                    )
                    .await?
                    .response()?;
                let response = desktop.start(&session, None).await?.response()?;
                if !response
                    .devices()
                    .contains(DeviceType::Keyboard | DeviceType::Pointer)
                {
                    return Err(flow_like_types::anyhow!(
                        "The portal did not grant keyboard and pointer access"
                    ));
                }
                let streams = response.streams().unwrap_or_default().to_vec();
                let closed = Box::pin(session.receive_closed().await?);
                Ok((streams, closed))
            }
            .await;
            let (streams, closed) = match setup {
                Ok(result) => result,
                Err(error) => {
                    let _ = session.close().await;
                    return Err(error);
                }
            };
            Ok(Self {
                desktop,
                session,
                streams,
                closed,
            })
        })
    }
    fn result<T>(result: Result<T, ashpd::Error>) -> InputResult<T> {
        result.map_err(|error| {
            tracing::warn!(%error,"Portal input failed");
            InputError::Simulate("The desktop portal rejected input; grant access again")
        })
    }
    fn keysym(&self, symbol: i32, direction: Direction) -> InputResult<()> {
        if direction != Direction::Release {
            Self::result(block_on(self.desktop.notify_keyboard_keysym(
                &self.session,
                symbol,
                KeyState::Pressed,
            )))?;
        }
        if direction != Direction::Press {
            Self::result(block_on(self.desktop.notify_keyboard_keysym(
                &self.session,
                symbol,
                KeyState::Released,
            )))?;
        }
        Ok(())
    }
}
impl NativeBackend for PortalInput {
    fn live(&mut self) -> bool {
        self.closed.next().now_or_never().is_none()
    }
}
impl Keyboard for PortalInput {
    fn fast_text(&mut self, text: &str) -> InputResult<Option<()>> {
        if text.contains('\0') {
            return Err(InputError::InvalidInput("Text contains a null character"));
        }
        for c in text.chars() {
            self.key(Key::Unicode(c), Direction::Click)?;
        }
        Ok(Some(()))
    }
    fn key(&mut self, key: Key, direction: Direction) -> InputResult<()> {
        let symbol = match key {
            Key::Unicode(c) => match c {
                '\n' | '\r' => 0xff0d,
                '\t' => 0xff09,
                c if (c as u32) <= 0xff => c as u32,
                c => 0x01000000 | c as u32,
            },
            Key::Return => 0xff0d,
            Key::Tab => 0xff09,
            Key::Escape => 0xff1b,
            Key::Backspace => 0xff08,
            Key::Delete => 0xffff,
            Key::Space => 0x20,
            Key::UpArrow => 0xff52,
            Key::DownArrow => 0xff54,
            Key::LeftArrow => 0xff51,
            Key::RightArrow => 0xff53,
            Key::Home => 0xff50,
            Key::End => 0xff57,
            Key::PageUp => 0xff55,
            Key::PageDown => 0xff56,
            Key::Shift => 0xffe1,
            Key::Control => 0xffe3,
            Key::Alt => 0xffe9,
            Key::Meta => 0xffeb,
            Key::CapsLock => 0xffe5,
            Key::F1 => 0xffbe,
            Key::F2 => 0xffbf,
            Key::F3 => 0xffc0,
            Key::F4 => 0xffc1,
            Key::F5 => 0xffc2,
            Key::F6 => 0xffc3,
            Key::F7 => 0xffc4,
            Key::F8 => 0xffc5,
            Key::F9 => 0xffc6,
            Key::F10 => 0xffc7,
            Key::F11 => 0xffc8,
            Key::F12 => 0xffc9,
            Key::Other(code) => code,
            _ => {
                return Err(InputError::InvalidInput(
                    "This key is unavailable through the desktop portal",
                ));
            }
        };
        self.keysym(symbol as i32, direction)
    }
    fn raw(&mut self, keycode: u16, direction: Direction) -> InputResult<()> {
        if direction != Direction::Release {
            Self::result(block_on(self.desktop.notify_keyboard_keycode(
                &self.session,
                keycode.into(),
                KeyState::Pressed,
            )))?;
        }
        if direction != Direction::Press {
            Self::result(block_on(self.desktop.notify_keyboard_keycode(
                &self.session,
                keycode.into(),
                KeyState::Released,
            )))?;
        }
        Ok(())
    }
}
impl Mouse for PortalInput {
    fn button(&mut self, button: Button, direction: Direction) -> InputResult<()> {
        let code = match button {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
            _ => {
                return Err(InputError::InvalidInput(
                    "This mouse button is unavailable through the desktop portal",
                ));
            }
        };
        if direction != Direction::Release {
            Self::result(block_on(self.desktop.notify_pointer_button(
                &self.session,
                code,
                KeyState::Pressed,
            )))?;
        }
        if direction != Direction::Press {
            Self::result(block_on(self.desktop.notify_pointer_button(
                &self.session,
                code,
                KeyState::Released,
            )))?;
        }
        Ok(())
    }
    fn move_mouse(&mut self, x: i32, y: i32, coordinate: Coordinate) -> InputResult<()> {
        if coordinate == Coordinate::Rel {
            return Self::result(block_on(self.desktop.notify_pointer_motion(
                &self.session,
                x.into(),
                y.into(),
            )));
        }
        let mut candidates = self.streams.iter().filter_map(|s| {
            let (ox, oy) = s.position()?;
            let (w, h) = s.size()?;
            let (dx, dy) = (i64::from(x) - i64::from(ox), i64::from(y) - i64::from(oy));
            (dx >= 0 && dy >= 0 && dx < i64::from(w) && dy < i64::from(h)).then_some((s, dx, dy))
        });
        let (stream, dx, dy) = candidates.next().ok_or(InputError::Simulate(
            "No granted portal display has verified bounds for this coordinate",
        ))?;
        if candidates.next().is_some() {
            return Err(InputError::Simulate(
                "Granted portal display bounds overlap; absolute target is ambiguous",
            ));
        }
        Self::result(block_on(self.desktop.notify_pointer_motion_absolute(
            &self.session,
            stream.pipe_wire_node_id(),
            dx as f64,
            dy as f64,
        )))
    }
    fn scroll(&mut self, length: i32, axis: Axis) -> InputResult<()> {
        let axis = match axis {
            Axis::Horizontal => PortalAxis::Horizontal,
            Axis::Vertical => PortalAxis::Vertical,
        };
        Self::result(block_on(self.desktop.notify_pointer_axis_discrete(
            &self.session,
            axis,
            length,
        )))
    }
    fn main_display(&self) -> InputResult<(i32, i32)> {
        self.streams
            .first()
            .and_then(Stream::size)
            .ok_or(InputError::Simulate(
                "The portal did not provide display dimensions",
            ))
    }
    fn location(&self) -> InputResult<(i32, i32)> {
        Err(InputError::Simulate(
            "Wayland does not expose the current global cursor position",
        ))
    }
}
impl Drop for PortalInput {
    fn drop(&mut self) {
        let _ = block_on(self.session.close());
    }
}
