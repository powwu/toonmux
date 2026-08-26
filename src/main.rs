#![deny(clippy::all)]
#![deny(deprecated)]

mod json;
mod key;
mod state;
mod ui;
mod xdo;
mod keepalive;
mod raw;

use crate::key::{canonicalize_key, key_name};
use gdk;
use glib::Propagation;
use gtk::{prelude::*, Dialog, DialogFlags, Label, ResponseType};
use state::{Action, State};
use std::sync::{atomic::Ordering, Arc};

#[inline]
fn send_key_down(state: &state::State, ctl: &state::Controller, key: &gdk::keys::Key) {
    let window = ctl.window.load(Ordering::SeqCst);
    let xdo_guard = ctl.xdo.lock().unwrap();
    let xdo = xdo_guard.as_ref().unwrap_or(&state.xdo);
    if let Err(code) = xdo.send_key_down(window, key) {
        eprintln!("xdo: sending key down failed with code {}.", code);
    }
}

#[inline]
fn send_key_up(state: &state::State, ctl: &state::Controller, key: &gdk::keys::Key) {
    let window = ctl.window.load(Ordering::SeqCst);
    let xdo_guard = ctl.xdo.lock().unwrap();
    let xdo = xdo_guard.as_ref().unwrap_or(&state.xdo);
    if let Err(code) = xdo.send_key_up(window, key) {
        eprintln!("xdo: sending key up failed with code {}.", code);
    }
}

#[inline]
fn send_key(state: &state::State, ctl: &state::Controller, key: &gdk::keys::Key) {
    let window = ctl.window.load(Ordering::SeqCst);
    let xdo_guard = ctl.xdo.lock().unwrap();
    let xdo = xdo_guard.as_ref().unwrap_or(&state.xdo);
    if let Err(code) = xdo.send_key(window, key) {
        eprintln!("xdo: sending key failed with code {}.", code);
    }
}

#[inline]
fn send_key_shifted(state: &state::State, ctl: &state::Controller, key: &gdk::keys::Key) {
    let window = ctl.window.load(Ordering::SeqCst);
    let xdo_guard = ctl.xdo.lock().unwrap();
    let xdo = xdo_guard.as_ref().unwrap_or(&state.xdo);
    if let Err(code) = xdo.send_key_shifted(window, key) {
        eprintln!("xdo: sending shifted key failed with code {}.", code);
    }
}

fn main() -> Result<(), String> {
    // Initialize GTK.
    if gtk::init().is_err() {
        return Err("Failed to initialize GTK".to_owned());
    }

    {
        let css = gtk::CssProvider::new();
        css.load_from_data(b".autofill-button { background: #c9920e; color: #fff; font-family: 'Font Awesome 6 Free'; font-weight: 400; font-size: 14px; }").ok();
        gtk::StyleContext::add_provider_for_screen(
            &gdk::Screen::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    // Initialize internal state.
    let config_path = json::get_config_path()?;
    println!("Using {} as the config path...", config_path.display());
    let state = Arc::new(match State::from_json_file(&config_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Reading from {} failed with \"{}\"; using default \
                 configuration.",
                config_path.display(),
                e,
            );

            state::State::new()
                .ok_or_else(|| "Failed to initialize xdo".to_owned())?
        }
    });
    // Initialize the UI's state.
    let toonmux = Arc::new(ui::Toonmux::new(Arc::clone(&state), config_path));

    // Redirect key presses.
    {
        let state = Arc::clone(&state);
        let toonmux_ref = Arc::clone(&toonmux);
        toonmux.main_window.connect_key_press_event(move |_, e| {
            let event_key = canonicalize_key(e.keyval());

            // Handle controllers that are in the "talking" state.
            let talking = !state.talking.is_empty();
            if talking {
                // Getting a read lock on the controller state reader-writer
                // lock.
                let ctls = state.controllers.read().unwrap();

                for i in state.talking.iter() {
                    send_key_down(&state, &ctls[i], &event_key);
                }

                // Relinquishing read lock on the controller state
                // reader-writer lock.
            }

            // Handle mirror toggling.
            let mirroring = if !talking
                && event_key == state.main_bindings.toggle_mirroring()
            {
                let mirroring =
                    !state.mirroring.fetch_nand(true, Ordering::SeqCst);
                toonmux_ref.header.change_mirroring(mirroring);

                mirroring
            } else {
                state.mirroring.load(Ordering::SeqCst)
            };

            // Getting a read lock on the routing state reader-writer lock.
            let routes_state_lock = state.routes.read().unwrap();
            let maybe_routes = routes_state_lock.get(&event_key);

            if let Some(routes) = maybe_routes {
                // Getting a read lock on the controller state reader-writer
                // lock.
                let ctls = state.controllers.read().unwrap();

                for (ctl_ix, action) in routes {
                    let main_controller = &ctls[*ctl_ix];

                    let handle_action =
                        |(mirrored_or_ctl_ix, controller): (
                            usize,
                            &state::Controller,
                        )| {
                            match action {
                                Action::Simple(key) => {
                                    if !talking {
                                        send_key_down(&state, controller, key);
                                    }
                                }
                                Action::Keepalive(_) => {
                                    if !talking {
                                        send_key_down(&state, controller, &gdk::keys::constants::Home);
                                    }
                                }
                                Action::LowThrow(key) => {
                                    if !talking {
                                        send_key(&state, controller, key);
                                    }
                                }
                                Action::Talk(key) => {
                                    if mirrored_or_ctl_ix != *ctl_ix
                                        || !controller.has_mirror()
                                    {
                                        let was_talking = state
                                            .talking
                                            .toggle(mirrored_or_ctl_ix);
                                        if was_talking {
                                            send_key_up(&state, controller, key);
                                        } else {
                                            send_key(&state, controller, key);
                                        }
                                    }
                                }
                                Action::Debug(key) => {
                                    if !talking {
                                        send_key_shifted(&state, controller, key);
                                    }
                                }
                            }
                        };

                    handle_action((*ctl_ix, main_controller));
                    if mirroring {
                        main_controller
                            .mirrored
                            .iter()
                            .map(|i| (i, &ctls[i]))
                            .for_each(handle_action);
                    }
                }

                // Relinquishing read lock on the controller state
                // reader-writer lock.
            }

            Propagation::Stop

            // Relinquishing read lock on the routing state reader-writer lock.
        });
    }
    {
        let state = Arc::clone(&state);
        toonmux.main_window.connect_key_release_event(move |_, e| {
            let event_key = canonicalize_key(e.keyval());

            if !state.talking.is_empty() {
                // Handle controllers that are in the "talking" state.

                // Getting a read lock on the controller state reader-writer
                // lock.
                let ctls = state.controllers.read().unwrap();

                for i in state.talking.iter() {
                    send_key_up(&state, &ctls[i], &event_key);
                }

            // Relinquishing read lock on the controller state reader-writer
            // lock.
            } else {
                // Handle mirror toggling.
                let mirroring = state.mirroring.load(Ordering::SeqCst);

                // Getting a read lock on the routing state reader-writer lock.
                let routes_state_lock = state.routes.read().unwrap();
                let maybe_routes = routes_state_lock.get(&event_key);

                if let Some(routes) = maybe_routes {
                    // Getting a read lock on the controller state
                    // reader-writer lock.
                    let ctls = state.controllers.read().unwrap();

                    for (ctl_ix, action) in routes {
                        let main_controller = &ctls[*ctl_ix];

                        let handle_action =
                            |controller: &state::Controller| {
                                match action {
                                    Action::Simple(key) => {
                                        send_key_up(&state, controller, key);
                                    }
                                    Action::Keepalive(_) => {
                                        send_key_up(&state, controller, &gdk::keys::constants::Home);
                                    }
                                    Action::LowThrow(_) => (),
                                    Action::Talk(_) => (),
                                    Action::Debug(_) => (),
                                }
                            };

                        handle_action(main_controller);
                        if mirroring {
                            main_controller
                                .mirrored
                                .iter()
                                .map(|i| &ctls[i])
                                .for_each(handle_action);
                        }
                    }

                    // Relinquishing read lock on the controller state
                    // reader-writer lock.
                }

                // Relinquishing read lock on the routing state reader-writer
                // lock.
            }

            Propagation::Stop
        });
    }

    // Hook up expand/contract button.
    {
        let state = Arc::clone(&state);
        let toonmux_ref = Arc::clone(&toonmux);
        toonmux.header.expand.connect_clicked(move |_| {
            let prev_hidden = state.hidden.fetch_nand(true, Ordering::SeqCst);

            if !prev_hidden {
                toonmux_ref.header.add.hide();
                toonmux_ref.header.remove.hide();
                toonmux_ref.header.keepalivetoggle.hide();
                toonmux_ref.header.mode_combo.hide();
                toonmux_ref.interface.container.hide();
                toonmux_ref.main_window.resize(1, 1);
            } else {
                toonmux_ref.interface.container.show();
                toonmux_ref.header.add.show();
                toonmux_ref.header.remove.show();
                toonmux_ref.header.keepalivetoggle.show();
                toonmux_ref.header.mode_combo.show();
            }
        });
    }

    // Hook up mirror toggling button.
    {
        let state = Arc::clone(&state);
        let toonmux_ref = Arc::clone(&toonmux);
        toonmux.header.mirroring.connect_clicked(move |_| {
            toonmux_ref.header.change_mirroring(
                !state.mirroring.fetch_nand(true, Ordering::SeqCst),
            );
        });
    }

    // Dialog settings for the "set binding" popup UI.
    let dialog_flags = {
        let mut dialog_flags = DialogFlags::empty();
        dialog_flags.set(DialogFlags::MODAL, true);
        dialog_flags.set(DialogFlags::DESTROY_WITH_PARENT, true);
        dialog_flags.set(DialogFlags::USE_HEADER_BAR, false);

        dialog_flags
    };

    // Hook up add-a-controller button.
    {
        let state = Arc::clone(&state);
        let toonmux_ref = Arc::clone(&toonmux);
        toonmux.header.add.connect_clicked(move |_| {
            {
                // Getting a write lock on the controller state reader-writer
                // lock.
                let mut ctls_state = state.controllers.write().unwrap();

                // Update state.
                let new_ctl_state = ctls_state
                    .last()
                    .map(|cs| state::Controller::from_template(cs))
                    .unwrap_or_default();
                ctls_state.push(new_ctl_state);

                // Relinquishing write lock on the controller state
                // reader-writer lock.
            }

            // Getting a read lock on the controller state reader-writer lock.
            let ctls_state = state.controllers.read().unwrap();

            // Update other controller UIs to have new mirror menu item.
            for (ctl_ix, ctl_ui) in toonmux_ref
                .interface
                .controller_uis
                .read()
                .unwrap()
                .iter()
                .enumerate()
            {
                hook_up_mirror_menu_item(
                    &state,
                    &toonmux_ref,
                    ctl_ix,
                    ctls_state.len() - 1,
                    ctl_ui.mirror.add_menu_item(),
                );
            }

            // Update UI to have new controller.
            let new_ctl_state = ctls_state.last().unwrap();
            toonmux_ref
                .interface
                .add_controller(new_ctl_state, ctls_state.len());

            // Hook up new controller UI.
            hook_up_controller_ui(
                &state,
                &toonmux_ref,
                dialog_flags,
                ctls_state.len() - 1,
                toonmux_ref
                    .interface
                    .controller_uis
                    .read()
                    .unwrap()
                    .last()
                    .unwrap(),
            );

            // Relinquishing read lock on the controller state reader-writer
            // lock.
        });
    }

    // Hook up remove-a-controller button.
    {
        let state = Arc::clone(&state);
        let toonmux_ref = Arc::clone(&toonmux);
        toonmux.header.remove.connect_clicked(move |_| {
            state.remove_controller(&toonmux_ref.interface);
            toonmux_ref.interface.remove_controller();
            toonmux_ref.main_window.resize(1, 1);
        });
    }

    // Hook up keepalive toggle.
    keepalive::start_keepalive(&toonmux, &state);

    // Hook up mode combo (Normal / Raw).
    {
        let state = Arc::clone(&state);
        let toonmux_ref = Arc::clone(&toonmux);
        // Use a RefCell so the RawHandle can be stored/dropped on the GTK thread.
        let raw_handle: std::cell::RefCell<Option<raw::RawHandle>> =
            std::cell::RefCell::new(None);
        toonmux.header.mode_combo.connect_changed(move |combo| {
            match combo.active() {
                Some(1) => {
                    // Raw mode: pick device if not yet chosen.
                    let device = {
                        let guard = state.raw_device.read().unwrap();
                        guard.clone()
                    };
                    let device = device.or_else(|| {
                        let chosen =
                            raw::pick_device(&toonmux_ref.main_window);
                        if let Some(ref path) = chosen {
                            *state.raw_device.write().unwrap() =
                                Some(path.clone());
                        }
                        chosen
                    });
                    if let Some(path) = device {
                        *raw_handle.borrow_mut() =
                            Some(raw::start_raw(path, Arc::clone(&state)));
                    } else {
                        // User cancelled — revert to Normal.
                        combo.set_active(Some(0));
                    }
                }
                _ => {
                    // Normal mode: drop the raw handle to stop the thread.
                    *raw_handle.borrow_mut() = None;
                }
            }
        });
    }
    // Hook up main binding buttons.
    macro_rules! connect_main_key_binder {
        ( $key_id:ident, $key_name:expr, $needs_reroute:expr ) => {{
            let state = Arc::clone(&state);
            let toonmux_ref = Arc::clone(&toonmux);
            toonmux.interface.main_bindings_row.$key_id.connect_clicked(
                move |this| {
                    let key_choose_dialog = Dialog::with_buttons(
                        Some(concat!(
                            "Binding main \u{201c}",
                            $key_name,
                            "\u{201d} key",
                        )),
                        Some(&toonmux_ref.main_window),
                        dialog_flags,
                        &[
                            ("Clear", ResponseType::Other(0)),
                            ("Cancel", ResponseType::Cancel),
                        ],
                    );
                    key_choose_dialog.content_area().pack_start(
                        &Label::new(Some(concat!(
                            "Press a key to be bound to \u{201c}",
                            $key_name,
                            "\u{201d}.",
                        ))),
                        true,
                        false,
                        4,
                    );

                    {
                        let state = Arc::clone(&state);
                        key_choose_dialog.connect_key_press_event(
                            move |kcd, e| {
                                let old_key = state
                                    .main_bindings
                                    .$key_id
                                    .load(Ordering::SeqCst);
                                let new_key = canonicalize_key(e.keyval());

                                // If the user remaps the key to the same key,
                                // we don't need to do anything.
                                if old_key == *new_key {
                                    kcd.response(ResponseType::Cancel);

                                    return Propagation::Proceed;
                                }

                                // Make sure we aren't registering a duplicate
                                // main binding.
                                if state.is_bound_main(&new_key) {
                                    eprintln!(
                                        "Main bindings may not overlap.",
                                    );
                                    kcd.response(ResponseType::Cancel);

                                    return Propagation::Proceed;
                                }

                                // Store the new binding.
                                state
                                    .main_bindings
                                    .$key_id
                                    .store(*new_key, Ordering::SeqCst);

                                // Perform rerouting.
                                if $needs_reroute {
                                    state.reroute_main(
                                        &old_key.into(),
                                        &new_key,
                                    );
                                }

                                // Relinquish control to main window.
                                kcd.response(ResponseType::Accept);

                                Propagation::Proceed
                            },
                        );
                    }

                    key_choose_dialog.show_all();
                    let resp = key_choose_dialog.run();
                    // Unfortunately this method is now considered `unsafe`,
                    // but that's only because it's considered UB to call any
                    // methods on the object after destroying it, as you would
                    // expect.  We _do_ want to destroy the dialog here, rather
                    // than making it unresponsive until the user manually
                    // presses the 'X' button.
                    unsafe {
                        key_choose_dialog.destroy();
                    }

                    match resp {
                        // User pressed a key. State manipulation is already
                        // done by that handler so we just need to update what
                        // is displayed in the UI.
                        ResponseType::Accept => this.set_label(
                            key_name(
                                state
                                    .main_bindings
                                    .$key_id
                                    .load(Ordering::SeqCst)
                                    .into(),
                            )
                            .as_str(),
                        ),
                        // User pressed "Clear" button. We have to do state
                        // manipulation here in addition to updating the UI
                        // because this is effectively the "handler" for the
                        // "pressing the Clear button" event.
                        ResponseType::Other(0) => {
                            let new_key = 0;
                            // Store new (and empty) binding.
                            let old_key = state
                                .main_bindings
                                .$key_id
                                .swap(new_key, Ordering::SeqCst);

                            // Perform rerouting.
                            if $needs_reroute {
                                state.reroute_main(
                                    &old_key.into(),
                                    &new_key.into(),
                                );
                            }

                            this.set_label("");
                        }
                        // The user has cancelled.
                        _ => (),
                    }
                },
            );
        }};
    }

    connect_main_key_binder!(forward, "forward", true);
    connect_main_key_binder!(back, "back", true);
    connect_main_key_binder!(left, "left", true);
    connect_main_key_binder!(right, "right", true);
    connect_main_key_binder!(jump, "jump", true);
    connect_main_key_binder!(dismount, "dismount", true);
    connect_main_key_binder!(throw, "throw", true);
    connect_main_key_binder!(toggle_mirroring, "toggle mirroring", false);
    connect_main_key_binder!(camera_toggle, "camera toggle", true);
    connect_main_key_binder!(tasks, "tasks", true);

    // Hook up controller UI buttons.
    hook_up_controller_uis(&state, &toonmux, dialog_flags);

    // Hook up autofill button.
    {
        let state = Arc::clone(&state);
        let toonmux_ref = Arc::clone(&toonmux);
        toonmux.interface.autofill_button.connect_clicked(move |_| {
            let mut windows = xdo::find_toon_windows();
            if windows.is_empty() {
                return;
            }

            // If both games are present, ask which one to fill.
            let has_clash = windows.iter().any(|w| w.game == xdo::ToonGame::CorporateClash);
            let has_ttr   = windows.iter().any(|w| w.game == xdo::ToonGame::ToontownRewritten);
            if has_clash && has_ttr {
                let dialog = gtk::Dialog::with_buttons(
                    Some("Autofill — choose game"),
                    Some(&toonmux_ref.main_window),
                    dialog_flags,
                    &[
                        ("Corporate Clash",     gtk::ResponseType::Other(0)),
                        ("Toontown Rewritten",  gtk::ResponseType::Other(1)),
                        ("Cancel",              gtk::ResponseType::Cancel),
                    ],
                );
                dialog.show_all();
                let resp = dialog.run();
                unsafe { dialog.destroy(); }
                match resp {
                    gtk::ResponseType::Other(0) =>
                        windows.retain(|w| w.game == xdo::ToonGame::CorporateClash),
                    gtk::ResponseType::Other(1) =>
                        windows.retain(|w| w.game == xdo::ToonGame::ToontownRewritten),
                    _ => return,
                }
            }

            let ctls = state.controllers.read().unwrap();
            let ctl_uis = toonmux_ref.interface.controller_uis.read().unwrap();
            for (i, toon_win) in windows.iter().enumerate() {
                if i >= ctls.len() {
                    break;
                }
                let old = ctls[i].window.swap(toon_win.window, Ordering::SeqCst);
                // Create a per-controller xdo handle for the nested display.
                *ctls[i].xdo.lock().unwrap() =
                    xdo::Xdo::for_display(&toon_win.display);
                let pw = &ctl_uis[i].pick_window;
                if toon_win.window != 0 {
                    if old == 0 {
                        pw.set_label("-");
                        let ctx = pw.style_context();
                        ctx.remove_class("suggested-action");
                        ctx.add_class("destructive-action");
                    }
                }
            }
        });
    }

    // Make all the widgets within the UI visible.
    toonmux.main_window.show_all();

    // Start the GTK main event loop.
    gtk::main();

    Ok(())
}

#[inline]
fn hook_up_controller_uis(
    state: &Arc<State>,
    toonmux: &Arc<ui::Toonmux>,
    dialog_flags: DialogFlags,
) {
    // Getting a read lock on controller UIs' reader-writer lock.
    let ctl_uis = toonmux.interface.controller_uis.read().unwrap();

    for (ctl_ix, ctl_ui) in ctl_uis.iter().enumerate() {
        hook_up_controller_ui(state, toonmux, dialog_flags, ctl_ix, ctl_ui);
    }

    // Relinquishing read lock on controller UIs' reader-writer lock.
}

fn hook_up_controller_ui(
    state: &Arc<State>,
    toonmux: &Arc<ui::Toonmux>,
    dialog_flags: DialogFlags,
    ctl_ix: usize,
    ctl_ui: &ui::ControllerUi,
) {
    // Hook up the pick-a-window button.
    {
        let state = Arc::clone(state);
        ctl_ui.pick_window.connect_clicked(move |pw| {
            let ctls = state.controllers.read().unwrap();
            let ctl = &ctls[ctl_ix];
            let current = ctl.window.load(Ordering::SeqCst);
            if current != 0 {
                // Slot occupied — clear it.
                ctl.window.store(0, Ordering::SeqCst);
                *ctl.xdo.lock().unwrap() = None;
                pw.set_label("+");
                let ctx = pw.style_context();
                ctx.remove_class("destructive-action");
                ctx.add_class("suggested-action");
            } else {
                // Slot empty — pick a window.
                drop(ctls);
                if let Some(new_window) = state.xdo.select_window_with_click() {
                    let ctls = state.controllers.read().unwrap();
                    ctls[ctl_ix].window.store(new_window, Ordering::SeqCst);
                    pw.set_label("-");
                    let ctx = pw.style_context();
                    ctx.remove_class("suggested-action");
                    ctx.add_class("destructive-action");
                }
            }
        });
    }

    // Hook up the mirror menu.
    hook_up_mirror_menu(state, toonmux, ctl_ix, ctl_ui);

    macro_rules! connect_key_binder {
        ( $key_id:ident, $key_name:expr, $action_ty:ident ) => {{
            let state = Arc::clone(state);
            let toonmux = Arc::clone(toonmux);
            ctl_ui.$key_id.connect_clicked(move |this| {
                let key_choose_dialog = Dialog::with_buttons(
                    Some(concat!(
                        "Binding \u{201c}",
                        $key_name,
                        "\u{201d} key",
                    )),
                    Some(&toonmux.main_window),
                    dialog_flags,
                    &[
                        ("Clear", ResponseType::Other(0)),
                        ("Cancel", ResponseType::Cancel),
                    ],
                );
                key_choose_dialog.content_area().pack_start(
                    &Label::new(Some(concat!(
                        "Press a key to be bound to \u{201c}",
                        $key_name,
                        "\u{201d}.",
                    ))),
                    true,
                    false,
                    4,
                );

                {
                    let state = Arc::clone(&state);
                    key_choose_dialog.connect_key_press_event(
                        move |kcd, e| {
                            let new_key = canonicalize_key(e.keyval());
                            // Store the new binding.
                            let old_key = state.controllers.read().unwrap()
                                [ctl_ix]
                                .bindings
                                .$key_id
                                .swap(*new_key, Ordering::SeqCst);

                            // If the user remaps the key to the same key, we
                            // don't need to do anything.
                            if old_key == *new_key {
                                kcd.response(ResponseType::Cancel);

                                return Propagation::Proceed;
                            }

                            // Get the main key binding that this key maps to
                            // (i.e. the one that actually gets sent to
                            // clients).
                            let main_key = state.main_bindings.$key_id();

                            // Perform rerouting.
                            state.reroute(
                                ctl_ix,
                                &old_key.into(),
                                &new_key,
                                &main_key,
                                Some(Action::$action_ty(main_key.clone())),
                            );

                            // Relinquish control to main window.
                            kcd.response(ResponseType::Accept);

                            Propagation::Proceed
                        },
                    );
                }

                key_choose_dialog.show_all();
                let resp = key_choose_dialog.run();
                // Unfortunately this method is now considered `unsafe`, but
                // that's only because it's considered UB to call any methods
                // on the object after destroying it, as you would expect.  We
                // _do_want to destroy the dialog here, rather than making it
                // unresponsive until the user manually presses the 'X' button.
                unsafe {
                    key_choose_dialog.destroy();
                }

                match resp {
                    // User pressed a key. State manipulation is already done
                    // by that handler so we just need to update what is
                    // displayed in the UI.
                    ResponseType::Accept => this.set_label(
                        key_name(
                            state.controllers.read().unwrap()[ctl_ix]
                                .bindings
                                .$key_id
                                .load(Ordering::SeqCst)
                                .into(),
                        )
                        .as_str(),
                    ),
                    // User pressed "Clear" button. We have to do state
                    // manipulation here in addition to updating the UI because
                    // this is effectively the "handler" for the "pressing the
                    // Clear button" event.
                    ResponseType::Other(0) => {
                        let new_key = 0;
                        // Store new (and empty) binding.
                        let old_key = state.controllers.read().unwrap()
                            [ctl_ix]
                            .bindings
                            .$key_id
                            .swap(new_key, Ordering::SeqCst);

                        // Get the main key binding that this key maps to (i.e.
                        // the one that actually gets sent to clients).
                        let main_key = state.main_bindings.$key_id();

                        // Perform rerouting.
                        state.reroute(
                            ctl_ix,
                            &old_key.into(),
                            &new_key.into(),
                            &main_key,
                            None,
                        );

                        this.set_label("");
                    }
                    // The user has cancelled.
                    _ => (),
                }
            });
        }};
    }

    // Hook up keybinding buttons.
    connect_key_binder!(forward, "forward", Simple);
    connect_key_binder!(back, "back", Simple);
    connect_key_binder!(left, "left", Simple);
    connect_key_binder!(right, "right", Simple);
    connect_key_binder!(jump, "jump", Simple);
    connect_key_binder!(dismount, "dismount", Simple);
    connect_key_binder!(throw, "throw", Simple);
    connect_key_binder!(low_throw, "low throw", LowThrow);
    connect_key_binder!(talk, "talk", Talk);
    {
        let state = Arc::clone(state);
        let toonmux = Arc::clone(toonmux);
        ctl_ui.keepalive.connect_clicked(move |this| {
            let key_choose_dialog = Dialog::with_buttons(
                Some("Binding \u{201c}gags\u{201d} key"),
                Some(&toonmux.main_window),
                dialog_flags,
                &[
                    ("Clear", ResponseType::Other(0)),
                    ("Cancel", ResponseType::Cancel),
                ],
            );
            key_choose_dialog.content_area().pack_start(
                &Label::new(Some(
                    "Press a key to be bound to \u{201c}gags\u{201d}.",
                )),
                true,
                false,
                4,
            );

            {
                let state = Arc::clone(&state);
                key_choose_dialog.connect_key_press_event(move |kcd, e| {
                    let new_key = canonicalize_key(e.keyval());
                    let old_key = state.controllers.read().unwrap()[ctl_ix]
                        .bindings
                        .keepalive
                        .swap(*new_key, Ordering::SeqCst);

                    if old_key == *new_key {
                        kcd.response(ResponseType::Cancel);
                        return Propagation::Proceed;
                    }

                    let main_key = gdk::keys::constants::Home;
                    state.reroute(
                        ctl_ix,
                        &old_key.into(),
                        &new_key,
                        &main_key,
                        Some(Action::Keepalive(main_key)),
                    );

                    kcd.response(ResponseType::Accept);
                    Propagation::Proceed
                });
            }

            key_choose_dialog.show_all();
            let resp = key_choose_dialog.run();
            unsafe { key_choose_dialog.destroy(); }

            match resp {
                ResponseType::Accept => this.set_label(
                    key_name(
                        state.controllers.read().unwrap()[ctl_ix]
                            .bindings
                            .keepalive
                            .load(Ordering::SeqCst)
                            .into(),
                    )
                    .as_str(),
                ),
                ResponseType::Other(0) => {
                    let old_key = state.controllers.read().unwrap()[ctl_ix]
                        .bindings
                        .keepalive
                        .swap(0, Ordering::SeqCst);
                    let main_key = gdk::keys::constants::Home;
                    state.reroute(
                        ctl_ix,
                        &old_key.into(),
                        &0u32.into(),
                        &main_key,
                        None,
                    );
                    this.set_label("");
                }
                _ => (),
            }
        });
    }
    connect_key_binder!(camera_toggle, "camera toggle", Simple);
    connect_key_binder!(tasks, "tasks", Simple);
    // debug always sends shift+F1; wire it up manually.
    {
        let state = Arc::clone(state);
        let toonmux = Arc::clone(toonmux);
        ctl_ui.debug.connect_clicked(move |this| {
            let key_choose_dialog = Dialog::with_buttons(
                Some("Binding \u{201c}debug\u{201d} key"),
                Some(&toonmux.main_window),
                dialog_flags,
                &[
                    ("Clear", ResponseType::Other(0)),
                    ("Cancel", ResponseType::Cancel),
                ],
            );
            key_choose_dialog.content_area().pack_start(
                &Label::new(Some(
                    "Press a key to be bound to \u{201c}debug\u{201d}.",
                )),
                true,
                false,
                4,
            );

            {
                let state = Arc::clone(&state);
                key_choose_dialog.connect_key_press_event(move |kcd, e| {
                    let new_key = canonicalize_key(e.keyval());
                    let old_key = state.controllers.read().unwrap()[ctl_ix]
                        .bindings
                        .debug
                        .swap(*new_key, Ordering::SeqCst);

                    if old_key == *new_key {
                        kcd.response(ResponseType::Cancel);
                        return Propagation::Proceed;
                    }

                    let main_key = gdk::keys::constants::F1;
                    state.reroute(
                        ctl_ix,
                        &old_key.into(),
                        &new_key,
                        &main_key,
                        Some(Action::Debug(main_key)),
                    );

                    kcd.response(ResponseType::Accept);
                    Propagation::Proceed
                });
            }

            key_choose_dialog.show_all();
            let resp = key_choose_dialog.run();
            unsafe { key_choose_dialog.destroy(); }

            match resp {
                ResponseType::Accept => this.set_label(
                    key_name(
                        state.controllers.read().unwrap()[ctl_ix]
                            .bindings
                            .debug
                            .load(Ordering::SeqCst)
                            .into(),
                    )
                    .as_str(),
                ),
                ResponseType::Other(0) => {
                    let old_key = state.controllers.read().unwrap()[ctl_ix]
                        .bindings
                        .debug
                        .swap(0, Ordering::SeqCst);
                    let main_key = gdk::keys::constants::F1;
                    state.reroute(
                        ctl_ix,
                        &old_key.into(),
                        &0u32.into(),
                        &main_key,
                        None,
                    );
                    this.set_label("");
                }
                _ => (),
            }
        });
    }
}

fn hook_up_mirror_menu(
    state: &Arc<State>,
    toonmux: &Arc<ui::Toonmux>,
    ctl_ix: usize,
    ctl_ui: &ui::ControllerUi,
) {
    for (i, mirror_menu_item) in ctl_ui
        .mirror
        .menu
        .children()
        .into_iter()
        .map(|c| c.downcast::<gtk::MenuItem>().unwrap())
        .enumerate()
        .filter(|(_, mmi)| {
            mmi.downcast_ref::<gtk::SeparatorMenuItem>().is_none()
        })
    {
        hook_up_mirror_menu_item(state, toonmux, ctl_ix, i, mirror_menu_item);
    }
}

/// `i` is the index of `mirror_menu_item`.
fn hook_up_mirror_menu_item(
    state: &Arc<State>,
    toonmux: &Arc<ui::Toonmux>,
    ctl_ix: usize,
    i: usize,
    mirror_menu_item: gtk::MenuItem,
) {
    let state = Arc::clone(state);
    let toonmux = Arc::clone(toonmux);
    mirror_menu_item.connect_activate(move |_| {
        {
            // Getting a read lock on the controller state reader-writer
            // lock.
            let ctls = state.controllers.read().unwrap();

            let ctl = &ctls[ctl_ix];
            let new_mirror = i.wrapping_sub(1);

            // Store the new `mirror` value.
            let old_mirror = ctl.mirror.swap(new_mirror, Ordering::SeqCst);

            // Get rid of the `mirrored` entry for this controller, if any.
            if old_mirror != ::std::usize::MAX {
                ctls[old_mirror].mirrored.remove(ctl_ix);
            }

            // If we do set a mirror (i.e. not "none"), then update the
            // mirror's `mirrored` set to contain this controller.
            if new_mirror != ::std::usize::MAX {
                ctls[new_mirror].mirrored.insert(ctl_ix);
            }

            // Relinquishing read lock on the controller state
            // reader-writer lock.
        }

        // Getting a read lock on controller UIs' reader-writer lock.
        let ctl_uis = toonmux.interface.controller_uis.read().unwrap();
        if i == 0 {
            ctl_uis[ctl_ix].mirror.button.set_label("\u{22a3}");
        } else {
            ctl_uis[ctl_ix].mirror.button.set_label(&i.to_string());
        }

        // Relinquishing read lock on controller UIs' reader-writer lock.
    });
}
