use super::*;
use crate::computer::accessibility::AccessibilityBounds;
use ::windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    },
    UI::{Accessibility::*, WindowsAndMessaging::*},
};
use ::windows::core::BSTR;
struct Com;
impl Com {
    fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
fn hwnd(window: &xcap::Window) -> Result<HWND> {
    Ok(HWND(window.id()? as usize as *mut std::ffi::c_void))
}
fn automation() -> Result<IUIAutomation> {
    Ok(unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? })
}
fn node(e: &IUIAutomationElement) -> AccessibilityNode {
    unsafe {
        let bounds = e
            .CurrentBoundingRectangle()
            .ok()
            .map(|r| AccessibilityBounds {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
            });
        let mut states = vec![];
        if e.CurrentIsEnabled().is_ok_and(|v| v.as_bool()) {
            states.push("enabled".into());
        }
        if e.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool()) {
            states.push("focused".into());
        }
        let mut actions = vec!["focus".into()];
        if e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            .is_ok()
        {
            actions.push("invoke".into());
        }
        if e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .is_ok()
        {
            actions.push("set_value".into());
        }
        if e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
            .is_ok()
        {
            actions.push("select".into());
        }
        if e.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
            .is_ok()
        {
            actions.extend(["expand".into(), "collapse".into()]);
        }
        AccessibilityNode {
            native_id: None,
            role: e
                .CurrentControlType()
                .map(|kind| control_type_name(kind.0))
                .unwrap_or_default(),
            name: e.CurrentName().ok().map(|s| s.to_string()),
            value: e
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()
                .and_then(|p| p.CurrentValue().ok())
                .map(|s| s.to_string()),
            description: e.CurrentHelpText().ok().map(|s| s.to_string()),
            bounds,
            states,
            actions,
            children: vec![],
        }
    }
}
fn children(
    e: &IUIAutomationElement,
    walker: &IUIAutomationTreeWalker,
) -> Vec<IUIAutomationElement> {
    let mut result = vec![];
    let mut current = unsafe { walker.GetFirstChildElement(e) }.ok();
    while let Some(child) = current {
        current = unsafe { walker.GetNextSiblingElement(&child) }.ok();
        result.push(child);
        if result.len() >= 2000 {
            break;
        }
    }
    result
}
fn walk(
    e: &IUIAutomationElement,
    walker: &IUIAutomationTreeWalker,
    id: &str,
    path: Vec<usize>,
    depth: usize,
    budget: &mut usize,
) -> AccessibilityNode {
    *budget = budget.saturating_sub(1);
    let mut result = node(e);
    identify(&mut result, id, path.clone());
    if depth > 0 {
        for (i, child) in children(e, walker).iter().enumerate() {
            if *budget == 0 {
                break;
            }
            let mut next = path.clone();
            next.push(i);
            result
                .children
                .push(walk(child, walker, id, next, depth - 1, budget));
        }
    }
    result
}
pub async fn tree(id: &str, depth: usize) -> Result<AccessibilityNode> {
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        let _com = Com::new()?;
        let automation = automation()?;
        let w = select_window(&id, "", "", true)?.ok_or_else(|| anyhow!("Window not found"))?;
        let element = unsafe { automation.ElementFromHandle(hwnd(&w)?)? };
        let walker = unsafe { automation.ControlViewWalker()? };
        Ok(walk(
            &element,
            &walker,
            &w.id()?.to_string(),
            vec![],
            depth,
            &mut 2000,
        ))
    })
    .await?
}
pub async fn action(
    locator: &ElementLocator,
    action: &str,
    value: &str,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let locator = locator.clone();
    let action = action.to_owned();
    let value = value.to_owned();
    tokio::task::spawn_blocking(move || {
        let _com = Com::new()?;
        let automation = automation()?;
        let w = select_window(&locator.window_id, "", "", true)?
            .ok_or_else(|| anyhow!("Window not found"))?;
        let mut element = unsafe { automation.ElementFromHandle(hwnd(&w)?)? };
        let walker = unsafe { automation.ControlViewWalker()? };
        for i in &locator.path {
            element = children(&element, &walker)
                .into_iter()
                .nth(*i)
                .ok_or_else(|| anyhow!("Element no longer exists"))?;
        }
        verify(&node(&element), &locator)?;
        if cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            return Err(anyhow!("Automation cancelled"));
        }
        unsafe {
            match action.as_str() {
                "focus" => element.SetFocus()?,
                "invoke" => element
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)?
                    .Invoke()?,
                "select" => element
                    .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                        UIA_SelectionItemPatternId,
                    )?
                    .Select()?,
                "expand" => element
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )?
                    .Expand()?,
                "collapse" => element
                    .GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                        UIA_ExpandCollapsePatternId,
                    )?
                    .Collapse()?,
                "set_value" => {
                    let text = BSTR::from(value.as_str());
                    element
                        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)?
                        .SetValue(&text)?;
                }
                _ => return Err(anyhow!("Unsupported accessibility action")),
            }
        }
        Ok(())
    })
    .await?
}
pub fn operate_window(
    w: &xcap::Window,
    operation: &str,
    bounds: Option<(i32, i32, i32, i32)>,
) -> Result<()> {
    let hwnd = hwnd(w)?;
    unsafe {
        match operation {
            "focus" => {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                if !SetForegroundWindow(hwnd).as_bool() {
                    return Err(anyhow!("Windows denied foreground activation"));
                }
            }
            "minimize" => {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            "maximize" => {
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            }
            "restore" => {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            "close" => PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?,
            "move_resize" => {
                let (x, y, width, height) =
                    bounds.ok_or_else(|| anyhow!("Window bounds required"))?;
                SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
            }
            _ => return Err(anyhow!("Unsupported window operation")),
        }
    }
    Ok(())
}

fn control_type_name(control_type: i32) -> String {
    match control_type {
        50000 => "Button".to_string(),
        50001 => "Calendar".to_string(),
        50002 => "CheckBox".to_string(),
        50003 => "ComboBox".to_string(),
        50004 => "Edit".to_string(),
        50005 => "Hyperlink".to_string(),
        50006 => "Image".to_string(),
        50007 => "ListItem".to_string(),
        50008 => "List".to_string(),
        50009 => "Menu".to_string(),
        50010 => "MenuBar".to_string(),
        50011 => "MenuItem".to_string(),
        50012 => "ProgressBar".to_string(),
        50013 => "RadioButton".to_string(),
        50014 => "ScrollBar".to_string(),
        50015 => "Slider".to_string(),
        50016 => "Spinner".to_string(),
        50017 => "StatusBar".to_string(),
        50018 => "Tab".to_string(),
        50019 => "TabItem".to_string(),
        50020 => "Text".to_string(),
        50021 => "ToolBar".to_string(),
        50022 => "ToolTip".to_string(),
        50023 => "Tree".to_string(),
        50024 => "TreeItem".to_string(),
        50025 => "Custom".to_string(),
        50026 => "Group".to_string(),
        50027 => "Thumb".to_string(),
        50028 => "DataGrid".to_string(),
        50029 => "DataItem".to_string(),
        50030 => "Document".to_string(),
        50031 => "SplitButton".to_string(),
        50032 => "Window".to_string(),
        50033 => "Pane".to_string(),
        50034 => "Header".to_string(),
        50035 => "HeaderItem".to_string(),
        50036 => "Table".to_string(),
        50037 => "TitleBar".to_string(),
        50038 => "Separator".to_string(),
        _ => format!("Unknown({})", control_type),
    }
}
