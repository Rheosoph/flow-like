use super::state::RecordedFingerprint;

#[cfg(target_os = "macos")]
pub fn extract_fingerprint_at(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCopyElementAtPosition(
            application: *const c_void,
            x: f32,
            y: f32,
            element: *mut *const c_void,
        ) -> i32;
        fn AXUIElementCreateSystemWide() -> *const c_void;
        fn AXUIElementCopyAttributeValue(
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
        fn CFRelease(cf: *const c_void);
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

    // SAFETY: Calls CoreFoundation FFI. The CString is valid for the duration of the call,
    // and CFStringCreateWithCString returns a retained CF object that must be CFRelease'd.
    unsafe fn create_cf_string(s: &str) -> *const c_void {
        let c_str = match std::ffi::CString::new(s) {
            Ok(c) => c,
            Err(_) => return ptr::null(),
        };
        unsafe { CFStringCreateWithCString(ptr::null(), c_str.as_ptr(), K_CF_STRING_ENCODING_UTF8) }
    }

    // SAFETY: Reads CF string contents into a buffer. Validates the CF type before reading.
    // Buffer is stack-allocated with fixed size, null-terminated by CFStringGetCString.
    unsafe fn cf_string_to_string(cf: *const c_void) -> Option<String> {
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

    // SAFETY: macOS Accessibility API requires FFI calls. All CF objects obtained via
    // Copy* functions are properly CFRelease'd on all code paths. Null checks prevent
    // dereferencing invalid pointers. The AXUIElement APIs are thread-safe.
    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return None;
        }

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

        let value_attr = create_cf_string("AXValue");
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
pub fn extract_fingerprint_at(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, UIA_ControlTypePropertyId, UIA_NamePropertyId,
    };

    // SAFETY: CoInitializeEx/CoUninitialize are COM initialization functions.
    // We call CoInitializeEx once at start and CoUninitialize on all return paths.
    // This is safe as long as we don't call COM from other threads without initialization.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = extract_fingerprint_windows_inner(x, y);

    unsafe {
        CoUninitialize();
    }

    result
}

#[cfg(target_os = "windows")]
fn extract_fingerprint_windows_inner(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, UIA_ControlTypePropertyId, UIA_NamePropertyId,
        UIA_ValueValuePropertyId,
    };
    use windows::core::ComInterface;

    let automation: IUIAutomation = match ComInterface::cast(&CUIAutomation) {
        Ok(a) => a,
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
        role: None,
        name: None,
        text: None,
        bounding_box: None,
    };

    // SAFETY: GetCurrentPropertyValue is safe, returns Result
    if let Ok(control_type) = unsafe { element.GetCurrentPropertyValue(UIA_ControlTypePropertyId) }
    {
        // SAFETY: as_raw() is safe, we're just reading the variant value
        let raw = unsafe { control_type.as_raw() };
        if let Ok(ct) = i32::try_from(raw.Anonymous.Anonymous.Anonymous.iVal) {
            fp.role = Some(control_type_to_string(ct));
        }
    }

    if let Ok(name) = unsafe { element.GetCurrentPropertyValue(UIA_NamePropertyId) } {
        let raw = unsafe { name.as_raw() };
        // Check if it's a BSTR (VT_BSTR = 8)
        if raw.Anonymous.Anonymous.vt == 8 {
            let bstr = unsafe { &raw.Anonymous.Anonymous.Anonymous.bstrVal };
            if !bstr.is_null() {
                let s = unsafe { bstr.to_string() };
                if !s.is_empty() {
                    fp.name = Some(s);
                }
            }
        }
    }

    if let Ok(value) = unsafe { element.GetCurrentPropertyValue(UIA_ValueValuePropertyId) } {
        let raw = unsafe { value.as_raw() };
        if raw.Anonymous.Anonymous.vt == 8 {
            let bstr = unsafe { &raw.Anonymous.Anonymous.Anonymous.bstrVal };
            if !bstr.is_null() {
                let s = unsafe { bstr.to_string() };
                if !s.is_empty() {
                    fp.text = Some(s);
                }
            }
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
pub fn extract_fingerprint_at(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use std::time::Duration;

    // AT-SPI2 requires async runtime - use blocking approach with timeout
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

    rt.block_on(async {
        tokio::time::timeout(Duration::from_millis(500), extract_fingerprint_atspi(x, y))
            .await
            .ok()
            .flatten()
    })
}

#[cfg(target_os = "linux")]
async fn extract_fingerprint_atspi(x: i32, y: i32) -> Option<RecordedFingerprint> {
    use atspi::connection::AccessibilityConnection;
    use atspi::proxy::accessible::AccessibleProxy;
    use atspi::zbus::Connection;

    let conn = match AccessibilityConnection::new().await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Failed to connect to AT-SPI bus: {:?}", e);
            return None;
        }
    };

    // Get the desktop (root) accessible
    let registry = match conn.registry() {
        r => r,
    };

    // Use GetAccessibleAtPoint via the registry
    // AT-SPI2 doesn't have a direct "element at point" - we'd need to traverse the tree
    // For now, we can try using atspi's built-in mechanisms if available

    // This is a simplified implementation - full AT-SPI traversal is complex
    // The atspi crate's API may vary; this gives a starting point
    tracing::debug!(
        "AT-SPI fingerprinting at ({}, {}) - traversal not fully implemented",
        x,
        y
    );

    // Return a placeholder for now - full implementation requires tree traversal
    // and hit-testing each accessible's bounding box
    Some(RecordedFingerprint {
        id: flow_like_types::create_id(),
        role: Some("Unknown".to_string()),
        name: None,
        text: None,
        bounding_box: None,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn extract_fingerprint_at(_x: i32, _y: i32) -> Option<RecordedFingerprint> {
    tracing::debug!("UI element fingerprinting not supported on this platform");
    None
}
