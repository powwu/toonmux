use crate::{json, ui, xdo::Xdo};
use gdk::keys::{self, Key};
use gtk::prelude::*;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::BufReader,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        RwLock,
    },
};

const USIZE_BITS: usize = std::mem::size_of::<usize>() * 8;

pub type AtomicKey = AtomicU32;

#[derive(Debug)]
pub struct AtomicBitSet(AtomicUsize);

#[derive(Debug)]
pub struct BitSetIter {
    bits: usize,
    offset: usize,
}

#[derive(Debug)]
pub struct State {
    pub xdo: Xdo,
    pub hidden: AtomicBool,
    pub mirroring: AtomicBool,
    pub main_bindings: MainBindings,
    pub controllers: RwLock<Vec<Controller>>,
    pub routes: RwLock<FxHashMap<Key, Vec<(usize, Action)>>>,
    pub talking: AtomicBitSet,
    pub raw_device: RwLock<Option<String>>,
}

#[derive(Debug)]
pub struct Controller {
    /// We use `window = 0` to represent no window being associated with this
    /// controller.
    pub window: AtomicU64,
    /// We use `mirror = usize::MAX` to represent no mirroring ("none").
    pub mirror: AtomicUsize,
    pub mirrored: AtomicBitSet,
    pub bindings: Bindings,
    /// Per-controller xdo handle for nested displays. When set, key sends use
    /// this instead of the global xdo.
    pub xdo: std::sync::Mutex<Option<crate::xdo::Xdo>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MainBindings {
    pub forward: AtomicKey,
    pub back: AtomicKey,
    pub left: AtomicKey,
    pub right: AtomicKey,
    pub jump: AtomicKey,
    pub dismount: AtomicKey,
    pub throw: AtomicKey,
    pub talk: AtomicKey,
    pub toggle_mirroring: AtomicKey,
    pub camera_toggle: AtomicKey,
    pub tasks: AtomicKey,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bindings {
    pub forward: AtomicKey,
    pub back: AtomicKey,
    pub left: AtomicKey,
    pub right: AtomicKey,
    pub jump: AtomicKey,
    pub dismount: AtomicKey,
    pub throw: AtomicKey,
    pub low_throw: AtomicKey,
    pub talk: AtomicKey,
    pub keepalive: AtomicKey,
    pub camera_toggle: AtomicKey,
    pub debug: AtomicKey,
    pub tasks: AtomicKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Simple(Key),
    LowThrow(Key),
    Talk(Key),
    Keepalive(Key),
    Debug(Key),
}

impl AtomicBitSet {
    #[inline(always)]
    pub fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 0
    }

    #[inline(always)]
    pub fn insert(&self, i: usize) {
        self.0.fetch_or(1 << i, Ordering::SeqCst);
    }

    #[inline(always)]
    pub fn remove(&self, i: usize) {
        self.0.fetch_and(!(1 << i), Ordering::SeqCst);
    }

    /// Returns the previous value.
    #[inline(always)]
    pub fn toggle(&self, i: usize) -> bool {
        let mask = 1 << i;

        self.0.fetch_xor(mask, Ordering::SeqCst) & mask != 0
    }

    /// Only performs **one** load.
    #[inline(always)]
    pub fn iter(&self) -> BitSetIter {
        let bits = self.0.load(Ordering::SeqCst);

        BitSetIter {
            bits,
            // Optimizing for the empty set case.
            offset: if bits == 0 { ::std::usize::MAX } else { 0 },
        }
    }
}

impl Iterator for BitSetIter {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.offset < USIZE_BITS {
            let set_bit = if (self.bits & (1 << self.offset)) != 0 {
                Some(self.offset)
            } else {
                None
            };

            self.offset += 1;

            if set_bit.is_some() {
                return set_bit;
            }
        }

        None
    }
}

impl State {
    /// Returns `None` iff xdo instance creation fails.
    pub fn new() -> Option<Self> {
        Xdo::new()
            .map(|xdo| Self {
                xdo,
                hidden: AtomicBool::new(false),
                mirroring: AtomicBool::new(true),
                main_bindings: Default::default(),
                controllers: RwLock::new(vec![
                    Default::default(),
                    Default::default(),
                    Controller {
                        window: AtomicU64::new(0),
                        mirror: AtomicUsize::new(0),
                        mirrored: AtomicBitSet::new(),
                        bindings: Bindings {
                            forward: AtomicKey::new(*keys::constants::Up),
                            back: AtomicKey::new(*keys::constants::Down),
                            left: AtomicKey::new(*keys::constants::Left),
                            right: AtomicKey::new(*keys::constants::Right),
                            ..Default::default()
                        },
                        xdo: std::sync::Mutex::new(None),
                    },
                    Controller {
                        window: AtomicU64::new(0),
                        mirror: AtomicUsize::new(0),
                        mirrored: AtomicBitSet::new(),
                        bindings: Bindings {
                            forward: AtomicKey::new(*keys::constants::Up),
                            back: AtomicKey::new(*keys::constants::Down),
                            left: AtomicKey::new(*keys::constants::Left),
                            right: AtomicKey::new(*keys::constants::Right),
                            ..Default::default()
                        },
                        xdo: std::sync::Mutex::new(None),
                    },
                ]),
                routes: Default::default(),
                talking: AtomicBitSet::new(),
                raw_device: RwLock::new(None),
            })
            .map(|mut state| {
                // Set up mirrored sets for controllers 2 and 3 mirroring 0.
                {
                    let ctls = state.controllers.get_mut().unwrap();
                    ctls[0].mirrored.insert(2);
                    ctls[0].mirrored.insert(3);
                }
                state.init();
                state
            })
    }

    pub fn from_json_file<P: AsRef<Path>>(
        json_path: P,
    ) -> Result<Self, String> {
        let f = File::open(json_path).map_err(|e| e.to_string())?;
        let buf_reader = BufReader::new(f);
        let json::State {
            main_bindings,
            controllers,
        } = json::State::from_reader(buf_reader)?;

        let controllers: Vec<_> = controllers
            .into_iter()
            .map(|c| Controller {
                window: AtomicU64::new(0),
                mirror: c.mirror,
                mirrored: AtomicBitSet::new(),
                bindings: c.bindings.into(),
                xdo: std::sync::Mutex::new(None),
            })
            .collect();
        for (i, controller) in controllers.iter().enumerate() {
            let mirror = controller.mirror.load(Ordering::SeqCst);
            if mirror != ::std::usize::MAX && mirror != i {
                controllers.get(mirror).map(|c| c.mirrored.insert(i));
            }
        }

        let xdo =
            Xdo::new().ok_or_else(|| "Failed to initialize xdo".to_owned())?;

        let mut state = Self {
            xdo,
            hidden: AtomicBool::new(false),
            mirroring: AtomicBool::new(true),
            main_bindings: main_bindings.into(),
            controllers: RwLock::new(controllers),
            routes: Default::default(),
            talking: AtomicBitSet::new(),
            raw_device: RwLock::new(None),
        };
        state.init();

        Ok(state)
    }

    fn init(&mut self) {
        // Loading initial routes.
        let r_lk = self.routes.get_mut().unwrap();

        for (ctl_ix, ctl) in
            self.controllers.get_mut().unwrap().iter().enumerate()
        {
            macro_rules! route_action {
                ( $action_id:ident, $action_ty:ident ) => {
                    let key = ctl.bindings.$action_id.load(Ordering::SeqCst);
                    if key != 0 {
                        r_lk.entry(key.into()).or_insert_with(Vec::new).push(
                            (
                                ctl_ix,
                                Action::$action_ty(
                                    self.main_bindings.$action_id(),
                                ),
                            ),
                        );
                    }
                };
            }

            route_action!(forward, Simple);
            route_action!(back, Simple);
            route_action!(left, Simple);
            route_action!(right, Simple);
            route_action!(jump, Simple);
            route_action!(dismount, Simple);
            route_action!(throw, Simple);
            route_action!(low_throw, LowThrow);
            route_action!(talk, Talk);
            // keepalive always maps to Home regardless of main bindings.
            {
                let key = ctl.bindings.keepalive.load(Ordering::SeqCst);
                if key != 0 {
                    r_lk.entry(key.into())
                        .or_insert_with(Vec::new)
                        .push((ctl_ix, Action::Keepalive(keys::constants::Home)));
                }
            }
            route_action!(camera_toggle, Simple);
            route_action!(tasks, Simple);
            // debug always sends shift+F1 regardless of main bindings.
            {
                let key = ctl.bindings.debug.load(Ordering::SeqCst);
                if key != 0 {
                    r_lk.entry(key.into())
                        .or_insert_with(Vec::new)
                        .push((ctl_ix, Action::Debug(keys::constants::F1)));
                }
            }
        }
    }

    pub fn is_bound_main(&self, key: &Key) -> bool {
        self.main_bindings.forward.load(Ordering::SeqCst) == **key
            || self.main_bindings.back.load(Ordering::SeqCst) == **key
            || self.main_bindings.left.load(Ordering::SeqCst) == **key
            || self.main_bindings.right.load(Ordering::SeqCst) == **key
            || self.main_bindings.jump.load(Ordering::SeqCst) == **key
            || self.main_bindings.dismount.load(Ordering::SeqCst) == **key
            || self.main_bindings.throw.load(Ordering::SeqCst) == **key
            || self.main_bindings.talk.load(Ordering::SeqCst) == **key
            || self.main_bindings.camera_toggle.load(Ordering::SeqCst) == **key
            || self.main_bindings.tasks.load(Ordering::SeqCst) == **key
    }

    pub fn reroute_main(&self, old_key: &Key, new_key: &Key) {
        // Getting a write lock on the routing state reader-writer lock.
        let mut r_lk = self.routes.write().unwrap();

        if **new_key != 0 {
            r_lk.values_mut().for_each(|dests| {
                dests
                    .iter_mut()
                    .filter(|(_, a)| a.key() == old_key)
                    .for_each(|(_, a)| a.set_key(new_key.clone()));
            });
        } else {
            r_lk.values_mut().for_each(|dests| {
                let mut i = 0;
                while let Some((_, a)) = dests.get(i) {
                    if a.key() == old_key {
                        dests.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
            });
        }

        // Relinquishing write lock on the routing state reader-writer lock.
    }

    pub fn reroute(
        &self,
        ctl_ix: usize,
        old_key: &Key,
        new_key: &Key,
        main_key: &Key,
        action: Option<Action>,
    ) {
        // Getting a write lock on the routing state reader-writer lock.
        let mut r_lk = self.routes.write().unwrap();

        // If we are rebinding and not adding a fresh new binding.
        if **old_key != 0 {
            // Remove the old routing.
            r_lk.get_mut(&old_key).map(|dests| {
                dests
                    .iter()
                    .enumerate()
                    .find(|(_, (i, a))| i == &ctl_ix && a.key() == main_key)
                    .map(|(j, _)| j)
                    .map(|j| dests.swap_remove(j));
            });
        }

        // Add new routing.
        if let Some(a) = action {
            r_lk.entry(new_key.clone())
                .or_insert_with(Vec::new)
                .push((ctl_ix, a));
        }

        // Relinquishing write lock on the routing state reader-writer lock.
    }

    /// NOTE/FIXME?: The fact that this method takes &ui::Interface is gross.
    pub fn remove_controller(&self, interface: &ui::Interface) {
        // Getting a write lock on the controllers state reader-writer lock.
        let mut ctls = self.controllers.write().unwrap();
        ctls.pop();
        let removed_ix = ctls.len();

        {
            // Getting a read lock on the controller UIs' reader-writer lock.
            let ctl_uis = interface.controller_uis.read().unwrap();
            for (ctl, ctl_ui) in ctls.iter().zip(ctl_uis.iter()) {
                if ctl.mirror.load(Ordering::SeqCst) == removed_ix {
                    ctl.mirror.store(::std::usize::MAX, Ordering::SeqCst);

                    ctl_ui.mirror.button.set_label("\u{22a3}");
                }

                ctl.mirrored.remove(removed_ix);
            }

            // Relinquishing read lock on the controller UIs' reader-writer
            // lock.
        }

        // Getting a write lock on the routing state reader-writer lock.
        let mut routes = self.routes.write().unwrap();

        for (_, dests) in routes.iter_mut() {
            let mut i = 0;
            while let Some((ctl_ix, _)) = dests.get_mut(i) {
                if *ctl_ix == removed_ix {
                    dests.swap_remove(i);
                } else {
                    i += 1;
                }
            }
        }

        // Relinquishing write lock on the routing state reader-writer lock.

        // Relinquishing write lock on the controllers state reader-writer
        // lock.
    }
}

impl Default for Controller {
    #[inline]
    fn default() -> Self {
        Self {
            window: AtomicU64::new(0),
            mirror: AtomicUsize::new(::std::usize::MAX),
            mirrored: AtomicBitSet::new(),
            bindings: Default::default(),
            xdo: std::sync::Mutex::new(None),
        }
    }
}

impl Controller {
    #[inline]
    pub fn from_template(template: &Self) -> Self {
        Self {
            window: AtomicU64::new(0),
            mirror: AtomicUsize::new(::std::usize::MAX),
            mirrored: AtomicBitSet::new(),
            bindings: template.bindings.clone(),
            xdo: std::sync::Mutex::new(None),
        }
    }

    #[inline(always)]
    pub fn has_mirror(&self) -> bool {
        self.mirror.load(Ordering::SeqCst) != ::std::usize::MAX
    }
}

impl MainBindings {
    #[inline(always)]
    pub fn forward(&self) -> Key {
        self.forward.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn back(&self) -> Key {
        self.back.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn left(&self) -> Key {
        self.left.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn right(&self) -> Key {
        self.right.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn jump(&self) -> Key {
        self.jump.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn dismount(&self) -> Key {
        self.dismount.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn throw(&self) -> Key {
        self.throw.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn talk(&self) -> Key {
        self.talk.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn toggle_mirroring(&self) -> Key {
        self.toggle_mirroring.load(Ordering::SeqCst).into()
    }

    /// One of these things is not like the others...
    #[inline(always)]
    pub fn low_throw(&self) -> Key {
        self.throw()
    }

    #[inline(always)]
    pub fn camera_toggle(&self) -> Key {
        self.camera_toggle.load(Ordering::SeqCst).into()
    }

    #[inline(always)]
    pub fn tasks(&self) -> Key {
        self.tasks.load(Ordering::SeqCst).into()
    }
}
impl Clone for MainBindings {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            forward: AtomicKey::new(self.forward.load(Ordering::SeqCst)),
            back: AtomicKey::new(self.back.load(Ordering::SeqCst)),
            left: AtomicKey::new(self.left.load(Ordering::SeqCst)),
            right: AtomicKey::new(self.right.load(Ordering::SeqCst)),
            jump: AtomicKey::new(self.jump.load(Ordering::SeqCst)),
            dismount: AtomicKey::new(self.dismount.load(Ordering::SeqCst)),
            throw: AtomicKey::new(self.throw.load(Ordering::SeqCst)),
            talk: AtomicKey::new(self.talk.load(Ordering::SeqCst)),
            toggle_mirroring: AtomicKey::new(
                self.toggle_mirroring.load(Ordering::SeqCst),
            ),
            camera_toggle: AtomicKey::new(
                self.camera_toggle.load(Ordering::SeqCst),
            ),
            tasks: AtomicKey::new(self.tasks.load(Ordering::SeqCst)),
        }
    }
}

impl Default for MainBindings {
    #[inline]
    fn default() -> Self {
        Self {
            forward: AtomicKey::new(*keys::constants::w),
            back: AtomicKey::new(*keys::constants::s),
            left: AtomicKey::new(*keys::constants::a),
            right: AtomicKey::new(*keys::constants::d),
            jump: AtomicKey::new(*keys::constants::space),
            dismount: AtomicKey::new(*keys::constants::Escape),
            throw: AtomicKey::new(*keys::constants::Super_L),
            talk: AtomicKey::new(*keys::constants::Return),
            toggle_mirroring: AtomicKey::new(*keys::constants::Shift_L),
            camera_toggle: AtomicKey::new(*keys::constants::Tab),
            tasks: AtomicKey::new(*keys::constants::e),
        }
    }
}

impl Clone for Bindings {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            forward: AtomicKey::new(self.forward.load(Ordering::SeqCst)),
            back: AtomicKey::new(self.back.load(Ordering::SeqCst)),
            left: AtomicKey::new(self.left.load(Ordering::SeqCst)),
            right: AtomicKey::new(self.right.load(Ordering::SeqCst)),
            jump: AtomicKey::new(self.jump.load(Ordering::SeqCst)),
            dismount: AtomicKey::new(self.dismount.load(Ordering::SeqCst)),
            throw: AtomicKey::new(self.throw.load(Ordering::SeqCst)),
            low_throw: AtomicKey::new(self.low_throw.load(Ordering::SeqCst)),
            talk: AtomicKey::new(self.talk.load(Ordering::SeqCst)),
            keepalive: AtomicKey::new(self.keepalive.load(Ordering::SeqCst)),
            camera_toggle: AtomicKey::new(
                self.camera_toggle.load(Ordering::SeqCst),
            ),
            debug: AtomicKey::new(self.debug.load(Ordering::SeqCst)),
            tasks: AtomicKey::new(self.tasks.load(Ordering::SeqCst)),
        }
    }
}

impl Default for Bindings {
    #[inline]
    fn default() -> Self {
        Self {
            forward: AtomicKey::new(*keys::constants::w),
            back: AtomicKey::new(*keys::constants::s),
            left: AtomicKey::new(*keys::constants::a),
            right: AtomicKey::new(*keys::constants::d),
            jump: AtomicKey::new(*keys::constants::space),
            dismount: AtomicKey::new(*keys::constants::Escape),
            throw: AtomicKey::new(*keys::constants::f),
            low_throw: AtomicKey::new(*keys::constants::r),
            talk: AtomicKey::new(*keys::constants::Return),
            keepalive: AtomicKey::new(*keys::constants::q),
            camera_toggle: AtomicKey::new(*keys::constants::Tab),
            debug: AtomicKey::new(*keys::constants::g),
            tasks: AtomicKey::new(*keys::constants::e),
        }
    }
}

impl Action {
    #[inline(always)]
    pub fn key(&self) -> &Key {
        match self {
            Self::Simple(key) => key,
            Self::LowThrow(key) => key,
            Self::Talk(key) => key,
            Self::Keepalive(key) => key,
            Self::Debug(key) => key,
        }
    }

    #[inline(always)]
    fn set_key(&mut self, key: Key) {
        match self {
            Self::Simple(k) => *k = key,
            Self::LowThrow(k) => *k = key,
            Self::Talk(k) => *k = key,
            Self::Keepalive(k) => *k = key,
            Self::Debug(k) => *k = key,
        }
    }
}
