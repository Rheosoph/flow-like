use super::*;
use crate::computer::accessibility::AccessibilityBounds;
use atspi::{AccessibilityConnection, ObjectRef, zbus};

async fn proxy<'a>(
    connection: &'a zbus::Connection,
    bus: &'a str,
    path: &'a str,
    interface: &'a str,
) -> Result<zbus::Proxy<'a>> {
    Ok(zbus::Proxy::new(connection, bus, path, interface).await?)
}
fn walk<'a>(
    connection: &'a zbus::Connection,
    bus: String,
    path: String,
    depth: usize,
    budget: &'a mut usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<AccessibilityNode>> + Send + 'a>> {
    Box::pin(async move {
        *budget = budget.saturating_sub(1);
        let accessible = proxy(connection, &bus, &path, "org.a11y.atspi.Accessible").await?;
        let role: String = accessible.call("GetRoleName", &()).await?;
        let name: Option<String> = accessible.get_property("Name").await.ok();
        let description: Option<String> = accessible.get_property("Description").await.ok();
        let component = proxy(connection, &bus, &path, "org.a11y.atspi.Component").await?;
        let bounds: Option<(i32, i32, i32, i32)> =
            component.call("GetExtents", &(0u32,)).await.ok();
        let action = proxy(connection, &bus, &path, "org.a11y.atspi.Action").await?;
        let actions: Vec<(String, String, String)> =
            action.call("GetActions", &()).await.unwrap_or_default();
        let states: Vec<u32> = accessible.call("GetState", &()).await.unwrap_or_default();
        let mut node = AccessibilityNode {
            native_id: None,
            role,
            name,
            value: None,
            description,
            bounds: bounds.map(|(x, y, width, height)| AccessibilityBounds {
                x,
                y,
                width,
                height,
            }),
            states: states.iter().map(|s| format!("bits:{s:08x}")).collect(),
            actions: actions.into_iter().map(|a| a.0).collect(),
            children: vec![],
        };
        identify(&mut node, &format!("atspi:{bus}|{path}"), vec![]);
        if depth > 0 {
            let children: Vec<ObjectRef> = accessible
                .call("GetChildren", &())
                .await
                .unwrap_or_default();
            for child in children {
                if *budget == 0 {
                    node.states.push("truncated".into());
                    break;
                }
                if child.path.as_str().ends_with("/null") {
                    continue;
                }
                if let Ok(child) = walk(
                    connection,
                    child.name.to_string(),
                    child.path.to_string(),
                    depth - 1,
                    budget,
                )
                .await
                {
                    node.children.push(child);
                }
            }
        }
        Ok(node)
    })
}
fn find_titles(node: AccessibilityNode, title: &str, matches: &mut Vec<AccessibilityNode>) {
    if node.name.as_deref() == Some(title) {
        matches.push(node);
        return;
    }
    for child in node.children {
        find_titles(child, title, matches);
    }
}
pub async fn tree(id: &str, depth: usize) -> Result<AccessibilityNode> {
    let connection = AccessibilityConnection::new().await?;
    if let Some((bus, path)) = id.strip_prefix("atspi:").and_then(|s| s.split_once('|')) {
        return walk(
            connection.connection(),
            bus.into(),
            path.into(),
            depth,
            &mut 2000,
        )
        .await;
    }
    let root = walk(
        connection.connection(),
        "org.a11y.atspi.Registry".into(),
        "/org/a11y/atspi/accessible/root".into(),
        depth.saturating_add(2).min(32),
        &mut 2000,
    )
    .await?;
    if id.is_empty() {
        return Ok(root);
    }
    let window = select_window(id, "", "", true)?.ok_or_else(|| anyhow!("Window not found"))?;
    let mut matches = Vec::new();
    find_titles(root, &window.title()?, &mut matches);
    let dbus = proxy(
        connection.connection(),
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?;
    let mut verified = Vec::new();
    for candidate in matches {
        let Some(locator) = candidate
            .native_id
            .as_deref()
            .and_then(|id| flow_like_types::json::from_str::<ElementLocator>(id).ok())
        else {
            continue;
        };
        let Some((bus, _)) = locator
            .window_id
            .strip_prefix("atspi:")
            .and_then(|s| s.split_once('|'))
        else {
            continue;
        };
        let pid: u32 = dbus.call("GetConnectionUnixProcessID", &(bus,)).await?;
        if pid == window.pid()? {
            verified.push(candidate);
        }
    }
    if verified.len() != 1 {
        return Err(anyhow!(
            "Window matches {} AT-SPI elements; a unique process and title are required",
            verified.len()
        ));
    }
    Ok(verified.remove(0))
}
pub async fn action(
    locator: &ElementLocator,
    action: &str,
    value: &str,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let (bus, path) = locator
        .window_id
        .strip_prefix("atspi:")
        .and_then(|s| s.split_once('|'))
        .ok_or_else(|| anyhow!("Invalid AT-SPI element identity"))?;
    let connection = AccessibilityConnection::new().await?;
    let node = walk(connection.connection(), bus.into(), path.into(), 0, &mut 1).await?;
    verify(&node, locator)?;
    if cancellation
        .as_ref()
        .is_some_and(|token| token.is_cancelled())
    {
        return Err(anyhow!("Automation cancelled"));
    }
    let result: bool = match action {
        "focus" => {
            proxy(
                connection.connection(),
                bus,
                path,
                "org.a11y.atspi.Component",
            )
            .await?
            .call("GrabFocus", &())
            .await?
        }
        "set_value" => {
            proxy(
                connection.connection(),
                bus,
                path,
                "org.a11y.atspi.EditableText",
            )
            .await?
            .call("SetTextContents", &(value,))
            .await?
        }
        _ => {
            let actions =
                proxy(connection.connection(), bus, path, "org.a11y.atspi.Action").await?;
            let available: Vec<(String, String, String)> = actions.call("GetActions", &()).await?;
            let index = available
                .iter()
                .position(|a| {
                    a.0.eq_ignore_ascii_case(action)
                        || (action == "invoke"
                            && ["click", "press", "activate"].contains(&a.0.as_str()))
                })
                .ok_or_else(|| anyhow!("Element does not offer action {}", action))?;
            actions.call("DoAction", &(index as i32,)).await?
        }
    };
    if !result {
        return Err(anyhow!("AT-SPI application rejected {}", action));
    }
    Ok(())
}
pub fn operate_window(
    w: &xcap::Window,
    operation: &str,
    bounds: Option<(i32, i32, i32, i32)>,
) -> Result<()> {
    if std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
    {
        return Err(anyhow!(
            "Window management requires an X11 window manager; native Wayland window control is unavailable"
        ));
    }
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask},
    };
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let id = w.id()?;
    let atom = |name: &str| -> Result<u32> {
        Ok(connection
            .intern_atom(false, name.as_bytes())?
            .reply()?
            .atom)
    };
    let (message, data) = match operation {
        "focus" => {
            connection.map_window(id)?.check()?;
            ("_NET_ACTIVE_WINDOW", [2, 0, 0, 0, 0])
        }
        "close" => ("_NET_CLOSE_WINDOW", [0, 2, 0, 0, 0]),
        "minimize" => ("WM_CHANGE_STATE", [3, 0, 0, 0, 0]),
        "maximize" | "restore" => {
            if operation == "restore" {
                connection.map_window(id)?.check()?;
            }
            (
                "_NET_WM_STATE",
                [
                    u32::from(operation == "maximize"),
                    atom("_NET_WM_STATE_MAXIMIZED_VERT")?,
                    atom("_NET_WM_STATE_MAXIMIZED_HORZ")?,
                    2,
                    0,
                ],
            )
        }
        "move_resize" => {
            let (x, y, width, height) = bounds.ok_or_else(|| anyhow!("Window bounds required"))?;
            (
                "_NET_MOVERESIZE_WINDOW",
                [
                    10 | (0xf << 8) | (2 << 12),
                    x as u32,
                    y as u32,
                    u32::try_from(width)?,
                    u32::try_from(height)?,
                ],
            )
        }
        _ => return Err(anyhow!("Unsupported window operation")),
    };
    let message_type = atom(message)?;
    if message != "WM_CHANGE_STATE" {
        let supported = connection
            .get_property(
                false,
                root,
                atom("_NET_SUPPORTED")?,
                AtomEnum::ATOM,
                0,
                u32::MAX,
            )?
            .reply()?;
        if !supported
            .value32()
            .is_some_and(|mut values| values.any(|value| value == message_type))
        {
            return Err(anyhow!(
                "The X11 window manager does not support {}",
                message
            ));
        }
    }
    let event = ClientMessageEvent::new(32, id, message_type, data);
    connection
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?
        .check()?;
    connection.flush()?;
    if operation == "focus" {
        return Ok(());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let confirmed = if operation == "close" {
            !xcap::Window::all()?
                .iter()
                .any(|window| window.id().is_ok_and(|candidate| candidate == id))
        } else if operation == "move_resize" {
            let (x, y, width, height) = bounds.ok_or_else(|| anyhow!("Window bounds required"))?;
            w.x()? == x && w.y()? == y && w.width()? == width as u32 && w.height()? == height as u32
        } else {
            let state = connection
                .get_property(
                    false,
                    id,
                    atom("_NET_WM_STATE")?,
                    AtomEnum::ATOM,
                    0,
                    u32::MAX,
                )?
                .reply()?;
            let values: Vec<u32> = state.value32().map(Iterator::collect).unwrap_or_default();
            let hidden = values.contains(&atom("_NET_WM_STATE_HIDDEN")?);
            let maximized = values.contains(&atom("_NET_WM_STATE_MAXIMIZED_VERT")?)
                && values.contains(&atom("_NET_WM_STATE_MAXIMIZED_HORZ")?);
            match operation {
                "minimize" => hidden,
                "maximize" => maximized,
                "restore" => !hidden && !maximized,
                _ => false,
            }
        };
        if confirmed {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "The X11 window manager did not confirm {} for window {}",
                operation,
                id
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
