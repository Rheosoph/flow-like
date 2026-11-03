//! Mouse and keyboard gestures on an input connection, without a flow run around them. The
//! click, scroll, type and key nodes, the computer-use agent and the bot tools all run these.
use crate::computer::keyboard::{ChordModifier, KeyChord, parse_key};
use crate::computer::mouse::{check_cancellation, interruptible_sleep};
use crate::computer::native::input::DesktopInput;
use enigo::{Axis, Coordinate, Direction, Keyboard, Mouse};
use flow_like_types::tokio_util::sync::CancellationToken;

pub(crate) const SCROLL_TICK_MS: u64 = 15;
pub(crate) const KEY_REPEAT_MS: u64 = 40;
const CLICK_GAP_MS: u64 = 70;

type Cancel<'a> = Option<&'a CancellationToken>;

/// Moves to `point` (desktop input coordinates), waits `click_delay_ms` and clicks `clicks`
/// times while `modifiers` (comma-separated) are held.
pub(crate) fn click(
    input: &mut DesktopInput,
    point: (i32, i32),
    button: enigo::Button,
    clicks: u32,
    modifiers: &str,
    click_delay_ms: u64,
    cancellation: Cancel<'_>,
) -> flow_like_types::Result<()> {
    check_cancellation(cancellation)?;
    input.modifiers(modifiers)?;
    input
        .move_mouse(point.0, point.1, Coordinate::Abs)
        .map_err(|e| flow_like_types::anyhow!("Failed to move mouse to {:?}: {}", point, e))?;
    interruptible_sleep(click_delay_ms, cancellation)?;
    for click in 0..clicks {
        if click > 0 {
            interruptible_sleep(CLICK_GAP_MS, cancellation)?;
        }
        check_cancellation(cancellation)?;
        input.button(button, Direction::Click).map_err(|e| {
            flow_like_types::anyhow!(
                "Failed click {} of {} at {:?}: {}",
                click + 1,
                clicks,
                point,
                e
            )
        })?;
    }
    Ok(())
}

pub(crate) fn move_to(input: &mut DesktopInput, point: (i32, i32)) -> flow_like_types::Result<()> {
    input
        .move_mouse(point.0, point.1, Coordinate::Abs)
        .map_err(|e| flow_like_types::anyhow!("Failed to move the mouse to {:?}: {}", point, e))
}

pub(crate) fn scroll(
    input: &mut DesktopInput,
    point: (i32, i32),
    axis: Axis,
    step: i32,
    ticks: u32,
    cancellation: Cancel<'_>,
) -> flow_like_types::Result<()> {
    move_to(input, point)?;
    interruptible_sleep(50, cancellation)?;
    for _ in 0..ticks {
        check_cancellation(cancellation)?;
        input
            .scroll(step, axis)
            .map_err(|e| flow_like_types::anyhow!("Failed to scroll: {}", e))?;
        interruptible_sleep(SCROLL_TICK_MS, cancellation)?;
    }
    Ok(())
}

pub(crate) fn type_text(
    input: &mut DesktopInput,
    text: &str,
    press_enter: bool,
    cancellation: Cancel<'_>,
) -> flow_like_types::Result<()> {
    check_cancellation(cancellation)?;
    input
        .text(text)
        .map_err(|e| flow_like_types::anyhow!("Failed to type text: {}", e))?;
    if press_enter {
        input
            .key(enigo::Key::Return, Direction::Click)
            .map_err(|e| flow_like_types::anyhow!("Failed to press Enter: {}", e))?;
    }
    Ok(())
}

pub(crate) fn chord_keys(
    chord: &KeyChord,
) -> flow_like_types::Result<(Vec<enigo::Key>, enigo::Key)> {
    let modifiers = chord
        .modifiers
        .iter()
        .map(|modifier| match modifier {
            ChordModifier::Control => enigo::Key::Control,
            ChordModifier::Shift => enigo::Key::Shift,
            ChordModifier::Alt => enigo::Key::Alt,
            ChordModifier::Meta => enigo::Key::Meta,
        })
        .collect();
    Ok((modifiers, parse_key(&chord.key)?))
}

/// Presses `modifiers` in order, runs `body`, then releases them in reverse, even on failure.
pub(crate) fn with_modifiers(
    input: &mut DesktopInput,
    modifiers: &[enigo::Key],
    body: impl FnOnce(&mut DesktopInput) -> flow_like_types::Result<()>,
) -> flow_like_types::Result<()> {
    let mut pressed = Vec::new();
    let mut result = Ok(());
    for modifier in modifiers {
        if let Err(error) = input.key(*modifier, Direction::Press) {
            result = Err(flow_like_types::anyhow!(
                "Failed to press modifier {:?}: {}",
                modifier,
                error
            ));
            break;
        }
        pressed.push(*modifier);
    }
    if result.is_ok() {
        result = body(input);
    }
    for modifier in pressed.into_iter().rev() {
        if let Err(error) = input.key(modifier, Direction::Release)
            && result.is_ok()
        {
            result = Err(flow_like_types::anyhow!(
                "Failed to release modifier {:?}: {}",
                modifier,
                error
            ));
        }
    }
    result
}

pub(crate) fn press(
    input: &mut DesktopInput,
    modifiers: &[enigo::Key],
    key: enigo::Key,
    repeat: u32,
    cancellation: Cancel<'_>,
) -> flow_like_types::Result<()> {
    for index in 0..repeat {
        check_cancellation(cancellation)?;
        if index > 0 {
            interruptible_sleep(KEY_REPEAT_MS, cancellation)?;
        }
        with_modifiers(input, modifiers, |input| {
            input
                .key(key, Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to press {:?}: {}", key, e))
        })?;
    }
    Ok(())
}
