//! macOS frontmost-window adapter with FR-1.4 secure-input probes.
//!
//! Two independent probes compose the `secure_input` answer:
//! 1. Carbon `IsSecureEventInputEnabled()` — system-wide, permission-free.
//! 2. Best-effort AX role check of the focused element (`AXSecureTextField`),
//!    only when Accessibility is trusted. Per EC-1.4, an unavailable or
//!    untrusted AX surface never fails the snapshot and never fabricates a
//!    positive; injection then degrades on its own conservative paths.

use super::{inject_context, FrontmostSnapshot};
use crate::{error::Error, inject::InjectContext, pipeline::AppContext};
use active_win_pos_rs::get_active_window;
use std::{
    ffi::{c_char, c_void, CStr},
    path::Path,
};

pub struct MacFrontmostSnapshot;

impl FrontmostSnapshot for MacFrontmostSnapshot {
    fn frontmost(&mut self) -> Result<AppContext, Error> {
        let window = get_active_window()
            .map_err(|_| Error::AxPerm("frontmost window is unavailable".into()))?;
        let bundle_id = bundle_identifier(&window.process_path)
            .unwrap_or_else(|| window.app_name.to_lowercase().replace(' ', "."));
        let accessibility_trusted = accessibility_trusted();
        Ok(AppContext {
            bundle_id,
            title: window.title,
            secure_input: secure_event_input(accessibility_trusted),
        })
    }
}

impl MacFrontmostSnapshot {
    /// Snapshot plus the pure FR-1.4 policy mapping, ready for the injector.
    pub fn injection_context(&mut self) -> Result<InjectContext, Error> {
        let snapshot = self.frontmost()?;
        let trusted = accessibility_trusted();
        Ok(inject_context(Some(&snapshot), trusted))
    }
}

impl crate::pipeline::ContextDetector for MacFrontmostSnapshot {
    fn snapshot(&mut self) -> Result<AppContext, Error> {
        self.frontmost()
    }
}

/// True while any process has secure event input enabled (password fields).
/// This Carbon probe requires no permission.
fn carbon_secure_input() -> bool {
    // SAFETY: no arguments; returns a plain Boolean per HIToolbox/Events.h.
    unsafe { IsSecureEventInputEnabled() != 0 }
}

/// Composed secure-input decision: Carbon first, then the AX role check only
/// when Accessibility trust makes it meaningful (EC-1.4).
fn secure_event_input(accessibility_trusted: bool) -> bool {
    if carbon_secure_input() {
        return true;
    }
    if accessibility_trusted && ax_focused_element_is_secure_field() == Some(true) {
        return true;
    }
    false
}

fn ax_focused_element_is_secure_field() -> Option<bool> {
    // SAFETY: every handle below follows CoreFoundation ownership rules —
    // objects obtained from Copy/Create are released on all exit paths.
    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return None;
        }
        let focused_attr = cf_string(K_AX_FOCUSED_ELEMENT);
        let mut focused: *mut c_void = std::ptr::null_mut();
        let status = AXUIElementCopyAttributeValue(system_wide, focused_attr, &mut focused);
        CFRelease(focused_attr.cast());
        if status != K_AX_ERROR_SUCCESS || focused.is_null() {
            CFRelease(system_wide.cast());
            return None;
        }

        let role_attr = cf_string(K_AX_ROLE);
        let mut role: *mut c_void = std::ptr::null_mut();
        let status = AXUIElementCopyAttributeValue(focused.cast(), role_attr, &mut role);
        CFRelease(role_attr.cast());
        CFRelease(focused.cast());
        CFRelease(system_wide.cast());
        if status != K_AX_ERROR_SUCCESS || role.is_null() {
            return None;
        }

        let matched = cf_string_matches(role.cast(), K_AX_SECURE_TEXT_FIELD_ROLE);
        CFRelease(role.cast());
        Some(matched)
    }
}

/// Whether this process may use the Accessibility API (and synthesize input).
pub fn accessibility_trusted() -> bool {
    // SAFETY: NULL options means "do not prompt", per HIServices headers.
    unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) != 0 }
}

const K_AX_ERROR_SUCCESS: i32 = 0;
const K_AX_FOCUSED_ELEMENT: &str = "AXFocusedUIElement";
const K_AX_ROLE: &str = "AXRole";
const K_AX_SECURE_TEXT_FIELD_ROLE: &str = "AXSecureTextField";
const K_CF_STRING_UTF8: u32 = 0x0800_0100;

type CFStringRef = *const c_void;
type CFTypeRef = *const c_void;
type AXUIElementRef = *mut c_void;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut *mut c_void,
    ) -> i32;
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;
    fn CFRelease(cf: CFTypeRef);
}

unsafe fn cf_string(value: &str) -> CFStringRef {
    let mut c = value.as_bytes().to_vec();
    c.push(0);
    CFStringCreateWithCString(
        std::ptr::null(),
        c.as_ptr().cast::<c_char>(),
        K_CF_STRING_UTF8,
    )
}

unsafe fn cf_string_matches(string: CFStringRef, expected: &str) -> bool {
    let mut buffer = [0u8; 128];
    let ok = CFStringGetCString(
        string,
        buffer.as_mut_ptr().cast::<c_char>(),
        buffer.len() as isize,
        K_CF_STRING_UTF8,
    );
    if ok == 0 {
        return false;
    }
    CStr::from_ptr(buffer.as_ptr().cast::<c_char>())
        .to_string_lossy()
        .eq_ignore_ascii_case(expected)
}

fn bundle_identifier(app_path: &Path) -> Option<String> {
    let plist = app_path.join("Contents/Info.plist");
    let text = std::fs::read_to_string(plist).ok()?;
    let key = text.find("CFBundleIdentifier")?;
    let after = &text[key..];
    let start = after.find("<string>")? + "<string>".len();
    let value = &after[start..];
    Some(value.split("</string>").next()?.trim().to_string())
}
