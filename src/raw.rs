use crate::state::State;
use evdev::{Device, EventType, KeyCode};
use gtk::{
    prelude::{
        BoxExt, CellLayoutExt, ContainerExt, DialogExt, GtkListStoreExtManual,
        GtkWindowExt, TreeModelExt, TreeSelectionExt, TreeViewColumnExt,
        TreeViewExt, WidgetExt, WidgetExtManual,
    },
    Dialog, DialogFlags, ResponseType, ScrolledWindow,
};
use std::{
    fs,
    sync::{atomic::AtomicBool, atomic::Ordering, Arc},
    thread,
};

/// Show a device picker dialog. Returns the chosen `/dev/input/by-id/…` path.
pub fn pick_device(parent: &gtk::Window) -> Option<String> {
    let dialog = Dialog::with_buttons(
        Some("Select Input Device"),
        Some(parent),
        {
            let mut f = DialogFlags::empty();
            f.set(DialogFlags::MODAL, true);
            f.set(DialogFlags::DESTROY_WITH_PARENT, true);
            f
        },
        &[
            ("Cancel", ResponseType::Cancel),
            ("OK", ResponseType::Accept),
        ],
    );
    dialog.set_default_size(600, 300);

    let store =
        gtk::ListStore::new(&[glib::Type::STRING, glib::Type::STRING]);

    if let Ok(entries) = fs::read_dir("/dev/input/by-id") {
        for entry in entries.flatten() {
            let path = entry.path();
            let path_str = path.to_string_lossy().into_owned();
            if let Ok(dev) = Device::open(&path) {
                let name = dev.name().unwrap_or("(unknown)").to_owned();
                store.insert_with_values(None, &[(0, &path_str), (1, &name)]);
            }
        }
    }

    let tv = gtk::TreeView::with_model(&store);
    tv.append_column(&{
        let col = gtk::TreeViewColumn::new();
        TreeViewColumnExt::set_title(&col, "By-ID Path");
        let cell = gtk::CellRendererText::new();
        CellLayoutExt::pack_start(&col, &cell, true);
        CellLayoutExt::add_attribute(&col, &cell, "text", 0);
        col
    });
    tv.append_column(&{
        let col = gtk::TreeViewColumn::new();
        TreeViewColumnExt::set_title(&col, "Name");
        let cell = gtk::CellRendererText::new();
        CellLayoutExt::pack_start(&col, &cell, true);
        CellLayoutExt::add_attribute(&col, &cell, "text", 1);
        col
    });

    let sw = ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    sw.add(&tv);
    dialog.content_area().pack_start(&sw, true, true, 0);
    dialog.show_all();

    let resp = dialog.run();
    let result = if resp == ResponseType::Accept {
        tv.selection()
            .selected()
            .and_then(|(model, iter)| model.value(&iter, 0).get::<String>().ok())
    } else {
        None
    };
    unsafe { dialog.destroy(); }
    result
}

/// Spawn a thread that reads raw evdev events from `device_path` and
/// forwards them as xdo key events to all controller windows.
/// Returns a handle that stops the thread when dropped.
pub fn start_raw(device_path: String, state: Arc<State>) -> RawHandle {
    let running = Arc::new(AtomicBool::new(true));
    let running2 = Arc::clone(&running);

    let handle = thread::spawn(move || {
        let mut dev = match Device::open(&device_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("raw: failed to open {}: {}", device_path, e);
                return;
            }
        };

        while running2.load(Ordering::SeqCst) {
            let events = match dev.fetch_events() {
                Ok(e) => e,
                Err(_) => break,
            };

            let ctls = state.controllers.read().unwrap();
            let windows: Vec<u64> = ctls
                .iter()
                .map(|c| c.window.load(Ordering::SeqCst))
                .filter(|&w| w != 0)
                .collect();
            drop(ctls);

            for ev in events {
                if ev.event_type() != EventType::KEY {
                    continue;
                }
                let gdk_key = match evdev_code_to_gdk(ev.code()) {
                    Some(k) => k,
                    None => continue,
                };

                for &window in &windows {
                    match ev.value() {
                        1 => {
                            if let Err(code) =
                                state.xdo.send_key_down(window, &gdk_key)
                            {
                                eprintln!(
                                    "raw xdo: key down failed with code {}",
                                    code
                                );
                            }
                        }
                        0 => {
                            if let Err(code) =
                                state.xdo.send_key_up(window, &gdk_key)
                            {
                                eprintln!(
                                    "raw xdo: key up failed with code {}",
                                    code
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    RawHandle { running, handle: Some(handle) }
}

pub struct RawHandle {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for RawHandle {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn evdev_code_to_gdk(code: u16) -> Option<gdk::keys::Key> {
    use gdk::keys::constants as c;
    Some(match KeyCode(code) {
        KeyCode::KEY_ESC => c::Escape,
        KeyCode::KEY_1 => c::_1,
        KeyCode::KEY_2 => c::_2,
        KeyCode::KEY_3 => c::_3,
        KeyCode::KEY_4 => c::_4,
        KeyCode::KEY_5 => c::_5,
        KeyCode::KEY_6 => c::_6,
        KeyCode::KEY_7 => c::_7,
        KeyCode::KEY_8 => c::_8,
        KeyCode::KEY_9 => c::_9,
        KeyCode::KEY_0 => c::_0,
        KeyCode::KEY_MINUS => c::minus,
        KeyCode::KEY_EQUAL => c::equal,
        KeyCode::KEY_BACKSPACE => c::BackSpace,
        KeyCode::KEY_TAB => c::Tab,
        KeyCode::KEY_Q => c::q,
        KeyCode::KEY_W => c::w,
        KeyCode::KEY_E => c::e,
        KeyCode::KEY_R => c::r,
        KeyCode::KEY_T => c::t,
        KeyCode::KEY_Y => c::y,
        KeyCode::KEY_U => c::u,
        KeyCode::KEY_I => c::i,
        KeyCode::KEY_O => c::o,
        KeyCode::KEY_P => c::p,
        KeyCode::KEY_LEFTBRACE => c::bracketleft,
        KeyCode::KEY_RIGHTBRACE => c::bracketright,
        KeyCode::KEY_ENTER => c::Return,
        KeyCode::KEY_LEFTCTRL => c::Control_L,
        KeyCode::KEY_A => c::a,
        KeyCode::KEY_S => c::s,
        KeyCode::KEY_D => c::d,
        KeyCode::KEY_F => c::f,
        KeyCode::KEY_G => c::g,
        KeyCode::KEY_H => c::h,
        KeyCode::KEY_J => c::j,
        KeyCode::KEY_K => c::k,
        KeyCode::KEY_L => c::l,
        KeyCode::KEY_SEMICOLON => c::semicolon,
        KeyCode::KEY_APOSTROPHE => c::apostrophe,
        KeyCode::KEY_GRAVE => c::grave,
        KeyCode::KEY_LEFTSHIFT => c::Shift_L,
        KeyCode::KEY_BACKSLASH => c::backslash,
        KeyCode::KEY_Z => c::z,
        KeyCode::KEY_X => c::x,
        KeyCode::KEY_C => c::c,
        KeyCode::KEY_V => c::v,
        KeyCode::KEY_B => c::b,
        KeyCode::KEY_N => c::n,
        KeyCode::KEY_M => c::m,
        KeyCode::KEY_COMMA => c::comma,
        KeyCode::KEY_DOT => c::period,
        KeyCode::KEY_SLASH => c::slash,
        KeyCode::KEY_RIGHTSHIFT => c::Shift_L,
        KeyCode::KEY_LEFTALT => c::Alt_L,
        KeyCode::KEY_SPACE => c::space,
        KeyCode::KEY_CAPSLOCK => c::Caps_Lock,
        KeyCode::KEY_F1 => c::F1,
        KeyCode::KEY_F2 => c::F2,
        KeyCode::KEY_F3 => c::F3,
        KeyCode::KEY_F4 => c::F4,
        KeyCode::KEY_F5 => c::F5,
        KeyCode::KEY_F6 => c::F6,
        KeyCode::KEY_F7 => c::F7,
        KeyCode::KEY_F8 => c::F8,
        KeyCode::KEY_F9 => c::F9,
        KeyCode::KEY_F10 => c::F10,
        KeyCode::KEY_NUMLOCK => c::Num_Lock,
        KeyCode::KEY_SCROLLLOCK => c::Scroll_Lock,
        KeyCode::KEY_F11 => c::F11,
        KeyCode::KEY_F12 => c::F12,
        KeyCode::KEY_HOME => c::Home,
        KeyCode::KEY_UP => c::Up,
        KeyCode::KEY_PAGEUP => c::Page_Up,
        KeyCode::KEY_LEFT => c::Left,
        KeyCode::KEY_RIGHT => c::Right,
        KeyCode::KEY_END => c::End,
        KeyCode::KEY_DOWN => c::Down,
        KeyCode::KEY_PAGEDOWN => c::Page_Down,
        KeyCode::KEY_INSERT => c::Insert,
        KeyCode::KEY_DELETE => c::Delete,
        KeyCode::KEY_RIGHTCTRL => c::Control_L,
        KeyCode::KEY_RIGHTALT => c::Alt_L,
        KeyCode::KEY_LEFTMETA => c::Super_L,
        KeyCode::KEY_RIGHTMETA => c::Super_L,
        KeyCode::KEY_PAUSE => c::Pause,
        _ => return None,
    })
}
