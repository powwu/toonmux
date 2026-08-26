//! This module is where all of the gross `unsafe` stuff lives.

use gdk::{self, keys::Key};
use glib::GString;
use libc;
use libxdo_sys;
use std::{ffi::CString, num::NonZeroI32, os::raw::c_char};
use x11::xlib::{self, Window};

#[derive(Debug)]
pub struct Xdo {
    handle: *mut libxdo_sys::xdo_t,
}

impl Xdo {
    pub fn new() -> Option<Self> {
        let handle = unsafe { libxdo_sys::xdo_new(::std::ptr::null()) };
        if handle.is_null() {
            None
        } else {
            Some(Self { handle })
        }
    }

    pub fn for_display(display: &str) -> Option<Self> {
        let cstr = CString::new(display).ok()?;
        let handle = unsafe { libxdo_sys::xdo_new(cstr.as_ptr()) };
        if handle.is_null() { None } else { Some(Self { handle }) }
    }

    pub fn select_window_with_click(&self) -> Option<Window> {
        let mut window = Default::default();
        let res = unsafe {
            libxdo_sys::xdo_select_window_with_click(self.handle, &mut window)
        };

        if res == 0 {
            Some(window)
        } else {
            None
        }
    }

    #[inline]
    pub fn send_key_down(
        &self,
        window: Window,
        key: &Key,
    ) -> Result<(), NonZeroI32> {
        if window == 0 {
            return Ok(());
        }

        let keyval_name = key.name().expect("invalid `Key`");
        let res = unsafe {
            libxdo_sys::xdo_send_keysequence_window_down(
                self.handle,
                window,
                gstring_as_ptr(&keyval_name),
                0,
            )
        };

        if let Some(code) = NonZeroI32::new(res) {
            Err(code)
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn send_key_up(
        &self,
        window: Window,
        key: &Key,
    ) -> Result<(), NonZeroI32> {
        if window == 0 {
            return Ok(());
        }

        let keyval_name = key.name().expect("invalid `Key`");
        let res = unsafe {
            libxdo_sys::xdo_send_keysequence_window_up(
                self.handle,
                window,
                gstring_as_ptr(&keyval_name),
                0,
            )
        };

        if let Some(code) = NonZeroI32::new(res) {
            Err(code)
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn send_key(
        &self,
        window: Window,
        key: &Key,
    ) -> Result<(), NonZeroI32> {
        if window == 0 {
            return Ok(());
        }

        let keyval_name = key.name().expect("invalid `Key`");
        let res = unsafe {
            libxdo_sys::xdo_send_keysequence_window(
                self.handle,
                window,
                gstring_as_ptr(&keyval_name),
                0,
            )
        };

        if let Some(code) = NonZeroI32::new(res) {
            Err(code)
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn send_key_shifted(
        &self,
        window: Window,
        key: &Key,
    ) -> Result<(), NonZeroI32> {
        if window == 0 {
            return Ok(());
        }

        // Use XSendEvent directly so we control the state field exactly,
        // avoiding any ambient modifier bleed from xdo's keysequence API.
        unsafe {
            let dpy = (*self.handle).xdpy;
            let keyval_name = key.name().expect("invalid `Key`");
            let keycode = xlib::XKeysymToKeycode(
                dpy,
                xlib::XStringToKeysym(keyval_name.as_ptr() as *const libc::c_char),
            );
            let shift_keycode = xlib::XKeysymToKeycode(dpy, 0xffe1); // Shift_L

            let mut ev: xlib::XKeyEvent = std::mem::zeroed();
            ev.display = dpy;
            ev.window = window;
            ev.root = xlib::XDefaultRootWindow(dpy);
            ev.subwindow = 0;
            ev.time = 0;
            ev.x = 1;
            ev.y = 1;
            ev.x_root = 1;
            ev.y_root = 1;
            ev.same_screen = 1;

            // Shift_L down, state=0
            ev.type_ = xlib::KeyPress;
            ev.keycode = shift_keycode as u32;
            ev.state = 0;
            xlib::XSendEvent(dpy, window, 0, xlib::KeyPressMask, &mut ev as *mut _ as *mut xlib::XEvent);

            // key down, state=ShiftMask (0x1)
            ev.type_ = xlib::KeyPress;
            ev.keycode = keycode as u32;
            ev.state = xlib::ShiftMask;
            xlib::XSendEvent(dpy, window, 0, xlib::KeyPressMask, &mut ev as *mut _ as *mut xlib::XEvent);

            // key up, state=ShiftMask
            ev.type_ = xlib::KeyRelease;
            ev.keycode = keycode as u32;
            ev.state = xlib::ShiftMask;
            xlib::XSendEvent(dpy, window, 0, xlib::KeyReleaseMask, &mut ev as *mut _ as *mut xlib::XEvent);

            // Shift_L up, state=ShiftMask
            ev.type_ = xlib::KeyRelease;
            ev.keycode = shift_keycode as u32;
            ev.state = xlib::ShiftMask;
            xlib::XSendEvent(dpy, window, 0, xlib::KeyReleaseMask, &mut ev as *mut _ as *mut xlib::XEvent);

            xlib::XFlush(dpy);
        }

        Ok(())
    }
}

impl Drop for Xdo {
    fn drop(&mut self) {
        unsafe {
            libxdo_sys::xdo_free(self.handle);
        }
    }
}

/// A Toontown window found by [`find_toon_windows`].
#[derive(Debug, Clone)]
pub struct ToonWindow {
    pub window: Window,
    pub display: String,
    pub game: ToonGame,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToonGame {
    CorporateClash,
    ToontownRewritten,
}

impl ToonGame {
    pub fn name(&self) -> &'static str {
        match self {
            ToonGame::CorporateClash => "Corporate Clash",
            ToonGame::ToontownRewritten => "Toontown Rewritten",
        }
    }
}

/// Search all gamescope Xwayland displays for Toontown game windows.
/// Discovers displays via `gamescope-N` socket files in `$XDG_RUNTIME_DIR`,
/// which maps to nested display `:N+1`. Xwayland uses abstract Unix sockets
/// so no filesystem X socket entry exists to scan.
/// Returns windows sorted by (display, window_id) — a reasonable proxy for
/// launch order within each display.
pub fn find_toon_windows() -> Vec<ToonWindow> {
    let uid = unsafe { libc::getuid() };
    let runtime_dir = format!("/run/user/{}", uid);

    let entries = match std::fs::read_dir(&runtime_dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut results = Vec::new();

    for entry in entries.flatten() {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        // Match "gamescope-N" exactly (no suffix like -ei, .lock, -limiter-*)
        let Some(n_str) = fname.strip_prefix("gamescope-") else { continue };
        if !n_str.chars().all(|c| c.is_ascii_digit()) || n_str.is_empty() {
            continue;
        }
        let gs_num: u32 = n_str.parse().unwrap_or(u32::MAX);
        let display_str = format!(":{}", gs_num + 1);
        let display_cstr = match CString::new(display_str.as_str()) {
            Ok(s) => s,
            Err(_) => continue,
        };

        eprintln!("[magic] trying {}", display_str);
        let dpy = unsafe { x11::xlib::XOpenDisplay(display_cstr.as_ptr()) };
        if dpy.is_null() {
            eprintln!("[magic] XOpenDisplay failed for {}", display_str);
            continue;
        }

        let root = unsafe { x11::xlib::XDefaultRootWindow(dpy) };
        search_window_tree(dpy, root, &display_str, &mut results);
        unsafe { x11::xlib::XCloseDisplay(dpy) };
    }

    results.sort_by_key(|w| {
        let n: u32 = w.display[1..].parse().unwrap_or(0);
        (n, w.window)
    });

    // Search :0 last (lower priority than gamescope displays).
    {
        let display_str = ":0".to_owned();
        let display_cstr = CString::new(display_str.as_str()).unwrap();
        let dpy = unsafe { xlib::XOpenDisplay(display_cstr.as_ptr()) };
        if !dpy.is_null() {
            let root = unsafe { xlib::XDefaultRootWindow(dpy) };
            search_window_tree(dpy, root, &display_str, &mut results);
            unsafe { xlib::XCloseDisplay(dpy) };
        }
    }

    results
}

fn get_window_name(dpy: *mut x11::xlib::Display, w: Window) -> Option<String> {
    let mut name: *mut libc::c_char = std::ptr::null_mut();
    let ret = unsafe { x11::xlib::XFetchName(dpy, w, &mut name) };
    if ret == 0 || name.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    unsafe { x11::xlib::XFree(name as *mut libc::c_void) };
    Some(s)
}

fn search_window_tree(
    dpy: *mut x11::xlib::Display,
    w: Window,
    display: &str,
    results: &mut Vec<ToonWindow>,
) {
    if let Some(name) = get_window_name(dpy, w) {
        let game = if name.starts_with("Corporate Clash") {
            Some(ToonGame::CorporateClash)
        } else if name.starts_with("Toontown Rewritten") {
            Some(ToonGame::ToontownRewritten)
        } else {
            None
        };
        if let Some(game) = game {
            eprintln!("[magic] found {:?} window {:x} ('{}') on {}", game, w, name, display);
            results.push(ToonWindow { window: w, display: display.to_owned(), game });
        }
    }

    // Recurse into children.
    let mut root_ret: Window = 0;
    let mut parent_ret: Window = 0;
    let mut children: *mut Window = std::ptr::null_mut();
    let mut nchildren: libc::c_uint = 0;
    let ret = unsafe {
        x11::xlib::XQueryTree(dpy, w, &mut root_ret, &mut parent_ret, &mut children, &mut nchildren)
    };
    if ret == 0 || children.is_null() {
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(children, nchildren as usize) };
    for &child in slice {
        search_window_tree(dpy, child, display, results);
    }
    unsafe { x11::xlib::XFree(children as *mut libc::c_void) };
}

// SAFETY: libxdo uses Xlib which is thread-safe when XInitThreads() has been
// called (GTK calls it during gtk::init()). The raw pointer is only used
// through &self methods that do not mutate the xdo_t handle itself.
unsafe impl Send for Xdo {}
unsafe impl Sync for Xdo {}

/// Return value has the same lifetime as `gstring`.
#[inline(always)]
fn gstring_as_ptr(gstring: &GString) -> *const c_char {
    gstring.as_str().as_ptr() as *const c_char
}
