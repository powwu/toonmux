// HOME KEY: 65360

use glib::ControlFlow;
use std::sync::{atomic::Ordering, Arc};
use crate::ui::Toonmux;
use crate::state::State;
use gtk::prelude::ToggleButtonExt;


fn keepalive_cycle(toonmux: &Arc<Toonmux>, state: &Arc<State>) {
    if toonmux.header.keepalivetoggle.is_active() {
        let ctls = state.controllers.read().unwrap();
        for window in ctls
            .iter()
            .map(|ctl| ctl.window.load(Ordering::SeqCst))
        {
            if let Err(code) =
                state.xdo.send_key(window, &gdk::keys::constants::Home)
            {
                eprintln!(
                    "xdo: sending key down failed with code {}.",
                    code,
                );
            }
        }
    }
}

pub fn start_keepalive(toonmux: &Arc<Toonmux>, state: &Arc<State>) {
    let toonmux = Arc::clone(toonmux);
    let state = Arc::clone(state);
    glib::timeout_add_seconds_local(30, move || {
        keepalive_cycle(&toonmux, &state);
        ControlFlow::Continue
    });
}
