use super::state::RecordedFingerprint;

/// Roles that accessibility APIs report for password fields.
pub fn is_password_role(role: &str) -> bool {
    role.eq_ignore_ascii_case("password text") || role.eq_ignore_ascii_case("AXSecureTextField")
}

#[cfg(target_os = "macos")]
mod ax {
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        pub fn AXUIElementCopyElementAtPosition(
            application: *const c_void,
            x: f32,
            y: f32,
            element: *mut *const c_void,
        ) -> i32;
        pub fn AXUIElementCreateSystemWide() -> *const c_void;
        pub fn AXUIElementSetMessagingTimeout(element: *const c_void, timeout: f32) -> i32;
        pub fn AXUIElementCopyAttributeValue(
            element: *const c_void,
            attribute: *const c_void,
            value: *mut *const c_void,
        ) -> i32;
    }

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *const c_void;
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut i8,
            buffer_size: isize,
            encoding: u32,
        ) -> bool;
        fn CFGetTypeID(cf: *const c_void) -> u64;
        fn CFStringGetTypeID() -> u64;
        pub fn CFRelease(cf: *const c_void);
    }

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        pub fn IsSecureEventInputEnabled() -> u8;
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

    // SAFETY: Calls CoreFoundation FFI. The CString is valid for the duration of the call,
    // and CFStringCreateWithCString returns a retained CF object that must be CFRelease'd.
    pub unsafe fn create_cf_string(s: &str) -> *const c_void {
        let c_str = match std::ffi::CString::new(s) {
            Ok(c) => c,
            Err(_) => return ptr::null(),
        };
        unsafe { CFStringCreateWithCString(ptr::null(), c_str.as_ptr(), K_CF_STRING_ENCODING_UTF8) }
    }

    // SAFETY: Reads CF string contents into a buffer. Validates the CF type before reading.
    // Buffer is stack-allocated with fixed size, null-terminated by CFStringGetCString.
    pub unsafe fn cf_string_to_string(cf: *const c_void) -> Option<String> {
        unsafe {
            if cf.is_null() || CFGetTypeID(cf) != CFStringGetTypeID() {
                return None;
            }
            let mut buffer = [0i8; 256];
            if CFStringGetCString(cf, buffer.as_mut_ptr(), 256, K_CF_STRING_ENCODING_UTF8) {
                std::ffi::CStr::from_ptr(buffer.as_ptr())
                    .to_str()
                    .ok()
                    .map(|s| s.to_string())
            } else {
                None
            }
        }
    }

    // SAFETY: `element` must be a live AXUIElementRef. The returned value is retained and the
    // caller must CFRelease it.
    pub unsafe fn copy_attribute(element: *const c_void, name: &str) -> Option<*const c_void> {
        unsafe {
            let attribute = create_cf_string(name);
            if attribute.is_null() {
                return None;
            }
            let mut value: *const c_void = ptr::null();
            let result = AXUIElementCopyAttributeValue(element, attribute, &mut value);
            CFRelease(attribute);
            if result != 0 || value.is_null() {
                return None;
            }
            Some(value)
        }
    }

    // SAFETY: `element` must be a live AXUIElementRef; the copied value is released here.
    pub unsafe fn string_attribute(element: *const c_void, name: &str) -> Option<String> {
        unsafe {
            let value = copy_attribute(element, name)?;
            let text = cf_string_to_string(value);
            CFRelease(value);
            text
        }
    }

    // SAFETY: `element` must be a live AXUIElementRef.
    pub unsafe fn is_secure_element(element: *const c_void) -> bool {
        unsafe {
            string_attribute(element, "AXSubrole")
                .is_some_and(|role| super::is_password_role(&role))
                || string_attribute(element, "AXRole")
                    .is_some_and(|role| super::is_password_role(&role))
        }
    }
}

/// Whether keyboard focus is in a password field. `None` when the platform cannot tell.
#[cfg(target_os = "macos")]
pub fn focused_element_is_secure() -> Option<bool> {
    use ax::*;

    // SAFETY: IsSecureEventInputEnabled only reads global input state. Every CF object copied
    // here is released on all paths, and null checks guard each dereference.
    unsafe {
        if IsSecureEventInputEnabled() != 0 {
            return Some(true);
        }
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(system_wide, 0.1);
        let focused = copy_attribute(system_wide, "AXFocusedUIElement");
        CFRelease(system_wide);
        let focused = focused?;
        AXUIElementSetMessagingTimeout(focused, 0.1);
        let secure = is_secure_element(focused);
        CFRelease(focused);
        Some(secure)
    }
}

#[cfg(target_os = "macos")]
pub fn extract_fingerprint_at(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use ax::*;
    use std::ffi::c_void;
    use std::ptr;

    // SAFETY: macOS Accessibility API requires FFI calls. All CF objects obtained via
    // Copy* functions are properly CFRelease'd on all code paths. Null checks prevent
    // dereferencing invalid pointers. The AXUIElement APIs are thread-safe.
    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(system_wide, 0.1);

        let mut element: *const c_void = ptr::null();
        let result =
            AXUIElementCopyElementAtPosition(system_wide, x as f32, y as f32, &mut element);

        if result != 0 || element.is_null() {
            CFRelease(system_wide);
            return None;
        }

        let mut fp = RecordedFingerprint {
            id: flow_like_types::create_id(),
            role: None,
            name: None,
            text: None,
            bounding_box: None,
        };

        let role_attr = create_cf_string("AXRole");
        if !role_attr.is_null() {
            let mut role_value: *const c_void = ptr::null();
            if AXUIElementCopyAttributeValue(element, role_attr, &mut role_value) == 0
                && !role_value.is_null()
            {
                fp.role = cf_string_to_string(role_value);
                CFRelease(role_value);
            }
            CFRelease(role_attr);
        }

        let title_attr = create_cf_string("AXTitle");
        if !title_attr.is_null() {
            let mut title_value: *const c_void = ptr::null();
            if AXUIElementCopyAttributeValue(element, title_attr, &mut title_value) == 0
                && !title_value.is_null()
            {
                let title = cf_string_to_string(title_value);
                if title.as_ref().is_some_and(|t| !t.is_empty()) {
                    fp.name = title;
                }
                CFRelease(title_value);
            }
            CFRelease(title_attr);
        }

        if fp.name.is_none() {
            let desc_attr = create_cf_string("AXDescription");
            if !desc_attr.is_null() {
                let mut desc_value: *const c_void = ptr::null();
                if AXUIElementCopyAttributeValue(element, desc_attr, &mut desc_value) == 0
                    && !desc_value.is_null()
                {
                    let desc = cf_string_to_string(desc_value);
                    if desc.as_ref().is_some_and(|d| !d.is_empty()) {
                        fp.name = desc;
                    }
                    CFRelease(desc_value);
                }
                CFRelease(desc_attr);
            }
        }

        let value_attr = if is_secure_element(element) {
            ptr::null()
        } else {
            create_cf_string("AXValue")
        };
        if !value_attr.is_null() {
            let mut value_value: *const c_void = ptr::null();
            if AXUIElementCopyAttributeValue(element, value_attr, &mut value_value) == 0
                && !value_value.is_null()
            {
                let value = cf_string_to_string(value_value);
                if value.as_ref().is_some_and(|v| !v.is_empty()) {
                    fp.text = value;
                }
                CFRelease(value_value);
            }
            CFRelease(value_attr);
        }

        // Extract bounding box from AXPosition + AXSize
        // Both return AXValue objects wrapping CGPoint / CGSize.
        // We use AXValueGetValue to unpack them.
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn AXValueGetValue(
                value: *const c_void,
                value_type: i32,
                value_ptr: *mut c_void,
            ) -> bool;
        }

        // AXValueType: kAXValueCGPointType = 1, kAXValueCGSizeType = 2
        let mut got_pos = false;
        let mut got_size = false;
        let mut pos_x: f64 = 0.0;
        let mut pos_y: f64 = 0.0;
        let mut size_w: f64 = 0.0;
        let mut size_h: f64 = 0.0;

        let pos_attr = create_cf_string("AXPosition");
        if !pos_attr.is_null() {
            let mut pos_value: *const c_void = ptr::null();
            if AXUIElementCopyAttributeValue(element, pos_attr, &mut pos_value) == 0
                && !pos_value.is_null()
            {
                #[repr(C)]
                struct CGPoint {
                    x: f64,
                    y: f64,
                }
                let mut point = CGPoint { x: 0.0, y: 0.0 };
                if AXValueGetValue(
                    pos_value,
                    1, // kAXValueCGPointType
                    &mut point as *mut CGPoint as *mut c_void,
                ) {
                    pos_x = point.x;
                    pos_y = point.y;
                    got_pos = true;
                }
                CFRelease(pos_value);
            }
            CFRelease(pos_attr);
        }

        let size_attr = create_cf_string("AXSize");
        if !size_attr.is_null() {
            let mut size_value: *const c_void = ptr::null();
            if AXUIElementCopyAttributeValue(element, size_attr, &mut size_value) == 0
                && !size_value.is_null()
            {
                #[repr(C)]
                struct CGSize {
                    width: f64,
                    height: f64,
                }
                let mut size = CGSize {
                    width: 0.0,
                    height: 0.0,
                };
                if AXValueGetValue(
                    size_value,
                    2, // kAXValueCGSizeType
                    &mut size as *mut CGSize as *mut c_void,
                ) {
                    size_w = size.width;
                    size_h = size.height;
                    got_size = true;
                }
                CFRelease(size_value);
            }
            CFRelease(size_attr);
        }

        if got_pos && got_size && (size_w > 0.0 || size_h > 0.0) {
            fp.bounding_box = Some((pos_x, pos_y, pos_x + size_w, pos_y + size_h));
        }

        CFRelease(element);
        CFRelease(system_wide);

        Some(fp)
    }
}

#[cfg(target_os = "windows")]
fn with_com<T>(operation: impl FnOnce() -> Option<T>) -> Option<T> {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

    // SAFETY: CoInitializeEx/CoUninitialize are COM initialization functions.
    // We call CoInitializeEx once at start and CoUninitialize on all return paths.
    // This is safe as long as we don't call COM from other threads without initialization.
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let result = operation();
    if initialized {
        unsafe {
            CoUninitialize();
        }
    }

    result
}

#[cfg(target_os = "windows")]
pub fn extract_fingerprint_at(x: i32, y: i32) -> Option<RecordedFingerprint> {
    with_com(|| extract_fingerprint_windows_inner(x, y))
}

/// Whether keyboard focus is in a password field. `None` when the platform cannot tell.
#[cfg(target_os = "windows")]
pub fn focused_element_is_secure() -> Option<bool> {
    with_com(|| {
        use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
        use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

        // SAFETY: COM is initialized by `with_com`; the interfaces are released before it
        // uninitializes.
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let element = unsafe { automation.GetFocusedElement() }.ok()?;
        unsafe { element.CurrentIsPassword() }
            .ok()
            .map(|password| password.as_bool())
    })
}

#[cfg(target_os = "windows")]
fn extract_fingerprint_windows_inner(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    let automation: IUIAutomation =
        match unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) } {
            Ok(automation) => automation,
            Err(e) => {
                tracing::debug!("Failed to create UIAutomation: {:?}", e);
                return None;
            }
        };

    let point = POINT { x, y };

    // SAFETY: ElementFromPoint is safe in the windows crate
    let element = match unsafe { automation.ElementFromPoint(point) } {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!("Failed to get element at point ({}, {}): {:?}", x, y, e);
            return None;
        }
    };

    let mut fp = RecordedFingerprint {
        id: flow_like_types::create_id(),
        role: Some("Unknown".to_string()),
        name: None,
        text: None,
        bounding_box: None,
    };

    if let Ok(control_type) = unsafe { element.CurrentControlType() } {
        fp.role = Some(control_type_to_string(control_type.0));
    }

    if let Ok(name) = unsafe { element.CurrentName() } {
        let name = name.to_string();
        if !name.is_empty() {
            fp.name = Some(name);
        }
    }

    // Extract bounding rectangle via IUIAutomationElement::CurrentBoundingRectangle
    if let Ok(rect) = unsafe { element.CurrentBoundingRectangle() } {
        let x1 = rect.left as f64;
        let y1 = rect.top as f64;
        let x2 = rect.right as f64;
        let y2 = rect.bottom as f64;
        if x2 > x1 || y2 > y1 {
            fp.bounding_box = Some((x1, y1, x2, y2));
        }
    }

    Some(fp)
}

#[cfg(target_os = "windows")]
fn control_type_to_string(control_type: i32) -> String {
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

#[cfg(target_os = "linux")]
fn block_on_atspi<T, F>(operation: impl FnOnce() -> F + Send) -> Option<T>
where
    T: Send,
    F: std::future::Future<Output = Option<T>>,
{
    use std::time::Duration;

    // AT-SPI2 requires an async runtime, and callers already run inside one, so the
    // runtime has to be built on a thread that is not a tokio worker.
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::debug!("Failed to create tokio runtime for AT-SPI: {:?}", e);
                        return None;
                    }
                };

                rt.block_on(async move {
                    tokio::time::timeout(Duration::from_millis(500), operation())
                        .await
                        .ok()
                        .flatten()
                })
            })
            .join()
            .unwrap_or_else(|_| {
                tracing::debug!("AT-SPI thread panicked");
                None
            })
    })
}

#[cfg(target_os = "linux")]
pub fn extract_fingerprint_at(x: i32, y: i32) -> Option<RecordedFingerprint> {
    block_on_atspi(move || extract_fingerprint_atspi(x, y))
}

/// Whether keyboard focus is in a password field. `None` when the platform cannot tell.
#[cfg(target_os = "linux")]
pub fn focused_element_is_secure() -> Option<bool> {
    block_on_atspi(focused_is_password_atspi)
}

/// Finds the focused object inside the active top-level windows, preferring one Collection
/// query per window and walking showing descendants for toolkits without Collection.
#[cfg(target_os = "linux")]
async fn focused_is_password_atspi() -> Option<bool> {
    use atspi::connection::AccessibilityConnection;
    use atspi::proxy::{accessible::AccessibleProxy, collection::CollectionProxy};
    use atspi::{MatchType, ObjectMatchRule, ObjectRef, Role, SortOrder, State};

    let conn = AccessibilityConnection::new().await.ok()?;
    let bus = conn.inner().connection();
    let root = AccessibleProxy::builder(bus)
        .destination("org.a11y.atspi.Registry")
        .ok()?
        .path("/org/a11y/atspi/accessible/root")
        .ok()?
        .build()
        .await
        .ok()?;
    let focused_rule = ObjectMatchRule::builder()
        .states([State::Focused], MatchType::All)
        .build();
    let mut pending: Vec<ObjectRef> = Vec::new();
    for application in root.get_children().await.ok()? {
        let Ok(app) = AccessibleProxy::builder(bus)
            .destination(application.name.clone())
            .ok()?
            .path(application.path.clone())
            .ok()?
            .build()
            .await
        else {
            continue;
        };
        for window in app.get_children().await.unwrap_or_default() {
            let Ok(accessible) = AccessibleProxy::builder(bus)
                .destination(window.name.clone())
                .ok()?
                .path(window.path.clone())
                .ok()?
                .build()
                .await
            else {
                continue;
            };
            if !accessible
                .get_state()
                .await
                .is_ok_and(|state| state.contains(State::Active))
            {
                continue;
            }
            let collected = match CollectionProxy::builder(bus)
                .destination(window.name.clone())
                .ok()?
                .path(window.path.clone())
                .ok()?
                .build()
                .await
            {
                Ok(collection) => collection
                    .get_matches(focused_rule.clone(), SortOrder::Canonical, 1, false)
                    .await
                    .ok(),
                Err(_) => None,
            };
            match collected {
                Some(matches) => {
                    if let Some(focused) = matches.into_iter().next() {
                        let focused = AccessibleProxy::builder(bus)
                            .destination(focused.name.clone())
                            .ok()?
                            .path(focused.path.clone())
                            .ok()?
                            .build()
                            .await
                            .ok()?;
                        return Some(focused.get_role().await.ok()? == Role::PasswordText);
                    }
                }
                None => pending.push(window),
            }
        }
    }
    for _ in 0..512 {
        let object = pending.pop()?;
        if object.path.as_str().ends_with("/null") {
            continue;
        }
        let Ok(accessible) = AccessibleProxy::builder(bus)
            .destination(object.name.clone())
            .ok()?
            .path(object.path.clone())
            .ok()?
            .build()
            .await
        else {
            continue;
        };
        let Ok(state) = accessible.get_state().await else {
            continue;
        };
        if state.contains(State::Focused) {
            return Some(accessible.get_role().await.ok()? == Role::PasswordText);
        }
        if state.contains(State::Showing) {
            pending.extend(accessible.get_children().await.unwrap_or_default());
        }
    }
    None
}

#[cfg(target_os = "linux")]
async fn extract_fingerprint_atspi(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use atspi::connection::AccessibilityConnection;

    let conn = match AccessibilityConnection::new().await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to connect to AT-SPI bus: {:?}", e);
            return None;
        }
    };

    use atspi::CoordType;
    use atspi::proxy::{accessible::AccessibleProxy, component::ComponentProxy};
    let bus = conn.inner().connection();
    let root = AccessibleProxy::builder(bus)
        .destination("org.a11y.atspi.Registry")
        .ok()?
        .path("/org/a11y/atspi/accessible/root")
        .ok()?
        .build()
        .await
        .ok()?;
    let mut pending = root.get_children().await.ok()?;
    let mut seen = std::collections::HashSet::new();
    let mut best: Option<RecordedFingerprint> = None;
    for _ in 0..256 {
        let Some(object) = pending.pop() else {
            break;
        };
        if !seen.insert(object.clone()) || object.path.as_str().ends_with("/null") {
            continue;
        }
        let accessible = match AccessibleProxy::builder(bus)
            .destination(object.name.clone())
            .ok()?
            .path(object.path.clone())
            .ok()?
            .build()
            .await
        {
            Ok(proxy) => proxy,
            Err(_) => continue,
        };
        if let Ok(component) = ComponentProxy::builder(bus)
            .destination(object.name.clone())
            .ok()?
            .path(object.path.clone())
            .ok()?
            .build()
            .await
        {
            if let Ok(hit) = component
                .get_accessible_at_point(x, y, CoordType::Screen)
                .await
            {
                if hit != object && !hit.path.as_str().ends_with("/null") {
                    pending.push(hit);
                }
            }
            if let Ok((left, top, width, height)) = component.get_extents(CoordType::Screen).await {
                if width > 0
                    && height > 0
                    && x >= left
                    && y >= top
                    && x < left.saturating_add(width)
                    && y < top.saturating_add(height)
                {
                    let area = width as f64 * height as f64;
                    let previous_area = best
                        .as_ref()
                        .and_then(|fp| fp.bounding_box)
                        .map(|(x1, y1, x2, y2)| (x2 - x1) * (y2 - y1))
                        .unwrap_or(f64::INFINITY);
                    if area < previous_area {
                        best = Some(RecordedFingerprint {
                            id: flow_like_types::create_id(),
                            role: accessible.get_role_name().await.ok(),
                            name: accessible.name().await.ok().filter(|name| !name.is_empty()),
                            text: None,
                            bounding_box: Some((
                                left as f64,
                                top as f64,
                                left as f64 + width as f64,
                                top as f64 + height as f64,
                            )),
                        });
                    }
                }
            }
        }
        if let Ok(children) = accessible.get_children().await {
            pending.extend(children);
        }
    }
    best
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn extract_fingerprint_at(_x: i32, _y: i32) -> Option<RecordedFingerprint> {
    tracing::debug!("UI element fingerprinting not supported on this platform");
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn focused_element_is_secure() -> Option<bool> {
    None
}
