#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::layout::Rect;
use super::windows::BrowserKind;
use flow_like_catalog_automation::computer::native::StackedWindow;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TabInfo {
    pub url: Option<String>,
    pub title: Option<String>,
}

#[cfg(target_os = "macos")]
const BOUNDS_TOLERANCE: f64 = 6.0;

const SEPARATORS: [&str; 3] = [" - ", " \u{2014} ", " \u{2013} "];
const BRANDS: [&str; 10] = [
    "Google Chrome",
    "Chromium",
    "Microsoft Edge",
    "Brave",
    "Arc",
    "Vivaldi",
    "Opera",
    "Mozilla Firefox",
    "Firefox",
    "Safari",
];

/// Removes the " - Google Chrome" style suffix, including a trailing profile name after it.
pub(crate) fn strip_browser_suffix(title: &str) -> String {
    let clean: String = title
        .chars()
        .filter(|c| !matches!(c, '\u{200b}' | '\u{200e}' | '\u{200f}'))
        .collect();
    let cut = SEPARATORS
        .iter()
        .flat_map(|separator| {
            BRANDS
                .iter()
                .map(move |brand| format!("{separator}{brand}"))
        })
        .filter_map(|marker| {
            let position = clean.rfind(&marker)?;
            let tail = &clean[position + marker.len()..];
            (tail.is_empty() || SEPARATORS.iter().any(|s| tail.starts_with(s))).then_some(position)
        })
        .max();
    match cut {
        Some(position) => clean[..position].trim_end().to_owned(),
        None => clean.trim().to_owned(),
    }
}

/// Whether the tab a perch was bound to is still the browser window's active tab. Unknown
/// readings count as active so a failed read never hides a bot.
pub(crate) fn tab_matches(bound: &TabInfo, current: &TabInfo) -> bool {
    match (&bound.url, &current.url) {
        (Some(bound_url), Some(current_url)) => bound_url == current_url,
        _ => match (&bound.title, &current.title) {
            (Some(bound_title), Some(current_title)) => bound_title == current_title,
            _ => true,
        },
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JxaBounds {
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    Edges([f64; 4]),
}

#[derive(Debug, Deserialize)]
pub(crate) struct JxaWindow {
    bounds: Option<JxaBounds>,
    url: Option<String>,
    title: Option<String>,
}

impl JxaWindow {
    fn rect(&self) -> Option<Rect> {
        match self.bounds.as_ref()? {
            JxaBounds::Rect {
                x,
                y,
                width,
                height,
            } => Some(Rect {
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            }),
            JxaBounds::Edges([left, top, right, bottom]) => Some(Rect {
                x: *left,
                y: *top,
                width: right - left,
                height: bottom - top,
            }),
        }
    }

    fn tab(&self) -> TabInfo {
        TabInfo {
            url: self.url.clone().filter(|url| !url.is_empty()),
            title: self.title.clone().filter(|title| !title.is_empty()),
        }
    }
}

pub(crate) fn parse_jxa_windows(output: &str) -> Result<Vec<JxaWindow>, String> {
    let (Some(start), Some(end)) = (output.find('['), output.rfind(']')) else {
        return Err(format!(
            "Browser scripting returned no window list: {:?}",
            output.chars().take(120).collect::<String>()
        ));
    };
    serde_json::from_str(&output[start..=end])
        .map_err(|error| format!("Browser scripting returned an unreadable window list: {error}"))
}

/// The scripted window whose bounds equal `target`; a lone scripted window is taken as the one.
pub(crate) fn match_window(
    windows: &[JxaWindow],
    target: Rect,
    tolerance: f64,
) -> Option<&JxaWindow> {
    let distance = |rect: Rect| {
        (rect.x - target.x)
            .abs()
            .max((rect.y - target.y).abs())
            .max((rect.width - target.width).abs())
            .max((rect.height - target.height).abs())
    };
    windows
        .iter()
        .filter_map(|window| Some((window, distance(window.rect()?))))
        .filter(|(_, distance)| *distance <= tolerance)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(window, _)| window)
        .or_else(|| (windows.len() == 1).then(|| &windows[0]))
}

fn application_name(kind: BrowserKind) -> Option<&'static str> {
    Some(match kind {
        BrowserKind::Chrome => "Google Chrome",
        BrowserKind::Chromium => "Chromium",
        BrowserKind::Edge => "Microsoft Edge",
        BrowserKind::Brave => "Brave Browser",
        BrowserKind::Arc => "Arc",
        BrowserKind::Vivaldi => "Vivaldi",
        BrowserKind::Opera => "Opera",
        BrowserKind::Safari => "Safari",
        BrowserKind::Firefox => return None,
    })
}

const JXA_TEMPLATE: &str = r#"(function () {
  function safe(read) { try { return read(); } catch (error) { return null; } }
  const app = Application("__APP__");
  if (!app.running()) { return "[]"; }
  return JSON.stringify(app.windows().map(function (window) {
    const tab = safe(function () { return window.__TAB__(); });
    return {
      bounds: safe(function () { return window.bounds(); }),
      url: tab ? safe(function () { return tab.url(); }) : null,
      title: tab ? safe(function () { return tab.__TITLE__(); }) : null
    };
  }));
})()"#;

/// The script is assembled only from constants chosen by the browser kind.
pub(crate) fn jxa_script(kind: BrowserKind) -> Option<String> {
    let app = application_name(kind)?;
    let (tab, title) = if kind == BrowserKind::Safari {
        ("currentTab", "name")
    } else {
        ("activeTab", "title")
    };
    Some(
        JXA_TEMPLATE
            .replace("__APP__", app)
            .replace("__TAB__", tab)
            .replace("__TITLE__", title),
    )
}

#[cfg(target_os = "macos")]
async fn run_jxa(script: &str) -> Result<String, String> {
    use std::time::Duration;
    use tokio::process::Command;

    let child = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", script])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(8), child)
        .await
        .map_err(|_| "Browser scripting timed out after 8 s".to_owned())?
        .map_err(|error| format!("Could not run osascript: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Browser scripting failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if stdout.trim().is_empty() {
        return Ok(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(stdout)
}

#[cfg(target_os = "macos")]
async fn scripted_tab(
    kind: BrowserKind,
    window: &StackedWindow,
) -> Result<Option<TabInfo>, String> {
    let Some(script) = jxa_script(kind) else {
        return Ok(None);
    };
    let output = run_jxa(&script).await?;
    let windows = parse_jxa_windows(&output)?;
    Ok(
        match_window(&windows, super::windows::raw_rect(window), BOUNDS_TOLERANCE)
            .map(JxaWindow::tab),
    )
}

#[cfg(not(target_os = "macos"))]
async fn scripted_tab(_: BrowserKind, _: &StackedWindow) -> Result<Option<TabInfo>, String> {
    Ok(None)
}

fn title_only(window: &StackedWindow) -> Option<TabInfo> {
    let title = flow_like_catalog_automation::computer::native::window_title(window.id)
        .or_else(|| window.title.clone())?;
    let title = strip_browser_suffix(&title);
    Some(TabInfo {
        url: None,
        title: (!title.is_empty()).then_some(title),
    })
}

/// The active tab of a browser window; `None` for a window that is not a browser.
pub(crate) async fn active_tab(window: &StackedWindow) -> Option<TabInfo> {
    let kind = BrowserKind::detect(&window.app_name)?;
    match scripted_tab(kind, window).await {
        Ok(Some(tab)) if tab != TabInfo::default() => return Some(tab),
        Ok(_) => {}
        Err(error) => {
            tracing::debug!(%error, app = %window.app_name, "Bot notch: scripted tab read failed")
        }
    }
    let window = window.clone();
    tauri::async_runtime::spawn_blocking(move || title_only(&window))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_browser_suffixes() {
        let cases = [
            ("Inbox - Google Chrome", "Inbox"),
            ("Inbox - Google Chrome - Felix", "Inbox"),
            ("Docs \u{2014} Mozilla Firefox", "Docs"),
            (
                "Page and 3 more pages - Personal - Microsoft\u{200b} Edge",
                "Page and 3 more pages - Personal",
            ),
            ("A - B - Brave", "A - B"),
            ("Plain title", "Plain title"),
            ("Brave new world", "Brave new world"),
            ("Why Arc - a story", "Why Arc - a story"),
        ];
        for (input, expected) in cases {
            assert_eq!(strip_browser_suffix(input), expected, "{input}");
        }
    }

    #[test]
    fn parses_object_and_edge_bounds() {
        let output = r#"[{"bounds":{"x":10,"y":20,"width":800,"height":600},"url":"https://a.test/","title":"A"},
            {"bounds":[100,50,500,450],"url":null,"title":null}]"#;
        let windows = parse_jxa_windows(output).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(
            windows[1].rect(),
            Some(Rect {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 400.0
            })
        );
        assert_eq!(windows[0].tab().url.as_deref(), Some("https://a.test/"));
        assert_eq!(windows[1].tab(), TabInfo::default());
    }

    #[test]
    fn parsing_rejects_noise_and_tolerates_trailing_newline() {
        assert!(parse_jxa_windows("execution error: not allowed").is_err());
        assert_eq!(parse_jxa_windows("[]\n").unwrap().len(), 0);
    }

    fn jxa(x: f64, url: &str) -> JxaWindow {
        JxaWindow {
            bounds: Some(JxaBounds::Rect {
                x,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            }),
            url: Some(url.into()),
            title: None,
        }
    }

    #[test]
    fn matches_windows_by_bounds_with_tolerance() {
        let windows = vec![jxa(0.0, "a"), jxa(400.0, "b")];
        let target = Rect {
            x: 402.0,
            y: 1.0,
            width: 800.0,
            height: 600.0,
        };
        assert_eq!(
            match_window(&windows, target, 6.0).unwrap().url.as_deref(),
            Some("b")
        );
        let far = Rect { x: 900.0, ..target };
        assert!(match_window(&windows, far, 6.0).is_none());
        assert_eq!(
            match_window(&windows[..1], far, 6.0)
                .unwrap()
                .url
                .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn tab_comparison_prefers_url_then_title_and_assumes_active_when_unknown() {
        let bound = TabInfo {
            url: Some("https://a.test".into()),
            title: Some("A".into()),
        };
        let other_url = TabInfo {
            url: Some("https://b.test".into()),
            title: Some("A".into()),
        };
        assert!(!tab_matches(&bound, &other_url));
        let title_only = TabInfo {
            url: None,
            title: Some("B".into()),
        };
        assert!(!tab_matches(&bound, &title_only));
        assert!(tab_matches(&bound, &TabInfo::default()));
    }

    #[test]
    fn scripts_name_only_constant_applications() {
        let chrome = jxa_script(BrowserKind::Chrome).unwrap();
        assert!(chrome.contains("Application(\"Google Chrome\")"));
        assert!(chrome.contains("window.activeTab()"));
        let safari = jxa_script(BrowserKind::Safari).unwrap();
        assert!(safari.contains("window.currentTab()") && safari.contains("tab.name()"));
        assert!(!chrome.contains("__"));
        assert!(jxa_script(BrowserKind::Firefox).is_none());
    }
}
