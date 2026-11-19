//! The between-stage service screen (`\game\setrep`, FSM state 3): each
//! player gets 60 minutes to change the car's setup (tyres, suspension,
//! brakes, steering, gear ratio) and repair damaged parts before the next
//! stage.
//!
//! - [`service_pending`] is `FUN_0047a190` (screen renderer `FUN_00479040`), re-evaluated every frame: the
//!   rally school at level 2 from task 3, never the armed special event or
//!   the attract demo, always a time trial or difficulty 0, otherwise the
//!   country's service-point bit for the upcoming stage (`0x4f6004`).
//! - [`ServiceScreen::update`] is `FUN_00476f70`: a 12-frame fade in, the menu
//!   input (`FUN_004aee60` over the menus [`ServiceScreen`] builds, `0x477720`),
//!   and after Continue a 12-frame fade out, then the next player or the end
//!   (state 3 returns 4).
//! - The per-player bytes live in the live block (`P = live + 0x9c p`): each
//!   category's cost `P[0x90 + 2k] & 0x3f` and value `P[0x91 + 2k]` (current in
//!   bits 0-2, the value at the service's start in 3-5); each part's repair
//!   cost `A = P[0x9a + 3i] & 0x3f`, condition `B = P[0x9b + 3i] & 0x7f` (100
//!   intact) and starting condition `C = P[0x9c + 3i] & 0x7f`.
//!
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.service-screen`. The tables come from the user's
//! executable.

use crate::descriptor_menu::{
    ItemKind, Menu, MenuEvent, MenuId, MenuItem, MenuLinks, MenuSet, MenuSound,
};
use crate::frontend_menu::ValueWidget;
use crate::screen_fade::ScreenFade;

/// The time budget each player starts with (`DAT_004f605c`, minutes).
pub const SERVICE_BUDGET: i32 = 60;
/// The fades' length (`FUN_0048df80(12)`, `FUN_0048dfa0(12)`).
pub const SERVICE_FADE_FRAMES: u32 = 12;

/// The executable's tables the screen reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct ServiceTables<'a> {
    /// `0x4f6004`: the service points, one stage bit per country.
    pub mask: &'a [u8],
    /// `0x4f5fc8`: each country's default setup (9 x 5 values).
    pub country_defaults: &'a [u8],
    /// `0x4f5ff8` onward: the school tasks' defaults (from task 3, 5 values
    /// each; the native reads on past the two tasks' ten bytes).
    pub school_defaults: &'a [u8],
}

/// `FUN_0047a190`. `school` is `DAT_00d3ca88`, `special` `DAT_0055001c`.
pub fn service_pending(live: &[u8], school: bool, special: bool, mask: &[u8]) -> bool {
    if school {
        let b = live[0x6c0];
        return b & 0x18 == 0x10 && b & 0xe0 >= 0x60;
    }
    if special || live[0] & 1 != 0 {
        return false;
    }
    if live[0x166] & 3 == 2 || live[0x18] & 0x18 == 0 {
        return true;
    }
    let country = usize::from(live[0x168] & 0xf);
    let stage = u32::from(live[0x168] >> 4);
    let bit = if stage < 8 { 1u8 << stage } else { 0 };
    mask.get(country).is_some_and(|&m| m & bit != 0)
}

/// The item keys and callbacks of the screen's menus.
pub mod key {
    /// The five categories (1..=5) and parts (6..=10).
    pub const CATEGORY: i32 = 1;
    pub const PART: i32 = 6;
    pub const VIEW_STAGES: i32 = 0xb;
    pub const CONTINUE: i32 = 0xc;
    pub const RESET: i32 = 0xd;
}

/// Host callback ids (native addresses).
pub mod callback {
    pub const SETUP_APPLY: u32 = 0x479a90;
    pub const REPAIR_TOGGLE: u32 = 0x4799d0;
    pub const CONTINUE: u32 = 0x479970;
    pub const RESET: u32 = 0x4795e0;
    pub const VIEW_RETURN: u32 = 0x479ba0;
    pub const VIEW_ENTER: u32 = 0x479bc0;
}

/// The sound tables: the root menus' (`0x5fef90`, no back cue) and the
/// sub-pages' (`0x5fefa0`).
pub const ROOT_SOUNDS: u32 = 0x5fef90;
pub const PAGE_SOUNDS: u32 = 0x5fefa0;

/// The cue bank's slots (`sound\new\`): wheel, up/down, toggle, forward,
/// back, do, undo.
pub mod cue {
    pub const UPDOWN: u8 = 1;
    pub const TOGGLE: u8 = 2;
    pub const FORWARD: u8 = 3;
    pub const BACK: u8 = 4;
    pub const DO: u8 = 5;
    pub const UNDO: u8 = 6;
}

/// The menu-engine cue for a sound-table slot (`479de0`..`479e40`).
pub const fn menu_cue(table: u32, sound: MenuSound) -> Option<u8> {
    match sound {
        MenuSound::Move => Some(cue::UPDOWN),
        MenuSound::ValueChange => Some(cue::TOGGLE),
        MenuSound::Enter => Some(cue::FORWARD),
        MenuSound::Back if table == PAGE_SOUNDS => Some(cue::BACK),
        MenuSound::Back => None,
    }
}

/// A host effect of one update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceEffect {
    /// A cue on channel 0 (stop, play once, full volume); `ok` false plays
    /// nothing (the helpers' success byte).
    Cue { slot: u8, ok: bool },
}

/// The screen's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceScreen {
    pub set: MenuSet,
    pub setup_root: MenuId,
    pub categories: [MenuId; 5],
    pub repair: MenuId,
    pub bottom: MenuId,
    pub view_stages: MenuId,
    /// `DAT_005fef88`.
    pub current: MenuId,
    /// `DAT_005fef84`: 0 fade in, 1 input, 2 fade out.
    pub phase: u8,
    /// `DAT_005fef7c`, `DAT_004f605a`.
    pub player: u8,
    pub players: u8,
    /// `DAT_004f605c` (and its start `DAT_004f6060`).
    pub budget: i32,
    /// `DAT_005fef80`: the fade level the draws scale colours by.
    pub fade_level: i32,
    /// The stage view's plaque frame and typewriter counters.
    pub stage_view: StageViewReveal,
    /// The shared fade channel `0x82b37c` (the host carries it between
    /// this screen and the race).
    pub fade: ScreenFade,
    finished: bool,
    school: bool,
    instant: bool,
}

/// `DAT_005fdf50` (the plaque frame, 0..=14) and the six typewriter
/// counters `FUN_0046b770` reveals the stage facts by (`5fcc00`, `5fd0ac`,
/// `5fda8c`, `5fdf54`, `5fd54c`, `5fda94`); only entering the stage view
/// resets them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageViewReveal {
    pub frame: u8,
    pub reveal: [usize; 6],
}

/// The session facts the screen reads besides the live block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServiceContext {
    /// `DAT_00d3ca88`.
    pub school: bool,
    /// `DAT_00550018`: a restart.
    pub restart: bool,
    /// `DAT_00d3ca94`: a network session.
    pub network: bool,
    /// Split screen or head to head (outside the school): two players.
    pub two_players: bool,
    /// `DAT_00bbabec`: every fade is instant.
    pub instant_fades: bool,
}

fn base(player: u8) -> usize {
    usize::from(player) * 0x9c
}

impl ServiceScreen {
    /// `FUN_00479c80` and `FUN_00477090` for player 0.
    pub fn open(live: &mut [u8], context: ServiceContext, tables: ServiceTables) -> Self {
        let mut screen = Self {
            set: MenuSet::default(),
            setup_root: MenuId(0),
            categories: [MenuId(0); 5],
            repair: MenuId(0),
            bottom: MenuId(0),
            view_stages: MenuId(0),
            current: MenuId(0),
            phase: 0,
            player: 0,
            players: if !context.school && context.two_players {
                2
            } else {
                1
            },
            budget: SERVICE_BUDGET,
            fade_level: 0,
            stage_view: StageViewReveal::default(),
            fade: ScreenFade::default(),
            finished: false,
            school: context.school,
            instant: context.instant_fades,
        };
        screen.begin_player(live, context, tables);
        screen
    }

    /// Whether the last update ended the screen (state 3 returns 4).
    pub const fn finished(&self) -> bool {
        self.finished
    }

    /// `FUN_00477090`: the fade in, the starting values (the country's or
    /// the school task's defaults on a first stage, conditions reset to
    /// intact), the budget, the snapshots, and the menus.
    fn begin_player(&mut self, live: &mut [u8], context: ServiceContext, tables: ServiceTables) {
        let p = base(self.player);
        let first = live[0x167] & 0x3f == 0 || live[0x168] & 0xf0 == 0;
        let time_trial = live[0x166] & 3 == 2;
        self.fade
            .fade_in(SERVICE_FADE_FRAMES, context.instant_fades);
        self.phase = 0;
        let country = usize::from(live[0x168] & 0xf);
        let set_values = |live: &mut [u8], table: &[u8], at: usize| {
            for k in 0..5 {
                let v = table.get(at + k).copied().unwrap_or(0);
                let byte = &mut live[p + 0x91 + 2 * k];
                *byte = (*byte & 0xf8) | (v & 7);
            }
        };
        let reset_damage = if context.school {
            let task = usize::from(live[0x6c0] >> 5);
            set_values(live, tables.school_defaults, task.saturating_sub(3) * 5);
            true
        } else if time_trial {
            if (!context.restart && first) || context.network {
                set_values(live, tables.country_defaults, country * 5);
            }
            true
        } else if first {
            set_values(live, tables.country_defaults, country * 5);
            true
        } else {
            false
        };
        if reset_damage {
            for i in 0..5 {
                let b = &mut live[p + 0x9b + 3 * i];
                *b = (*b & 0xe4) | 0x64;
                live[p + 0x9a + 3 * i] &= 0xc0;
            }
        }
        self.budget = SERVICE_BUDGET;
        for k in 0..5 {
            let v = live[p + 0x91 + 2 * k];
            live[p + 0x91 + 2 * k] = (v & 0xc7) | ((v & 7) << 3);
        }
        for i in 0..5 {
            let b = live[p + 0x9b + 3 * i] & 0x7f;
            let c = &mut live[p + 0x9c + 3 * i];
            *c = (*c & 0x80) | b;
        }
        self.build_menus(live);
    }

    /// `0x477720`: the setup root, the five category pages, the repair
    /// list, the bottom bar and the stage view, linked setup -> (repair ->)
    /// bottom -> setup; the bottom bar's first item has the focus.
    fn build_menus(&mut self, live: &[u8]) {
        let p = base(self.player);
        let mut set = MenuSet::default();
        let setup_root = set.add(Menu::new(-1, None, Some(ROOT_SOUNDS)));
        let counts = [4, 3, 3, 3, 5];
        let categories: [MenuId; 5] = std::array::from_fn(|k| {
            let mut menu = Menu::new(-1, Some(setup_root), Some(PAGE_SOUNDS));
            menu.wrap = false;
            let mut item = MenuItem::new(ItemKind::Value, -1, 0x478d50);
            item.key = key::CATEGORY + k as i32;
            item.value = ValueWidget {
                value: i32::from(live[p + 0x91 + 2 * k] & 7),
                value_count: counts[k],
                wrap: false,
            };
            item.activate = Some(callback::SETUP_APPLY);
            menu.items.push(item);
            set.add(menu)
        });
        for (k, &category) in categories.iter().enumerate() {
            let mut item = MenuItem::new(ItemKind::Submenu(category), 0x10 + k as i32, 0x478380);
            item.key = key::CATEGORY + k as i32;
            set.menu_mut(setup_root).items.push(item);
        }
        set.menu_mut(setup_root).on_predraw = Some(0x478070);
        let repair = set.add(Menu::new(-1, None, Some(ROOT_SOUNDS)));
        for (i, text) in [0x15, 0x16, 0x12, 0x17, 0x18].into_iter().enumerate() {
            let mut item = MenuItem::new(ItemKind::Value, text, 0x4788e0);
            item.key = key::PART + i as i32;
            item.value = ValueWidget {
                value: 0,
                value_count: 1,
                wrap: false,
            };
            item.activate = Some(callback::REPAIR_TOGGLE);
            set.menu_mut(repair).items.push(item);
        }
        set.menu_mut(repair).on_predraw = Some(0x478780);
        let bottom = set.add(Menu::new(-1, None, Some(ROOT_SOUNDS)));
        let view_stages = set.add(Menu::new(-1, Some(bottom), Some(PAGE_SOUNDS)));
        {
            let menu = set.menu_mut(bottom);
            menu.horizontal = true;
            menu.wrap = false;
            menu.on_predraw = Some(0x478c90);
            let mut view = MenuItem::new(ItemKind::Submenu(view_stages), 0x19, 0x478cc0);
            view.key = key::VIEW_STAGES;
            let mut go = MenuItem::new(ItemKind::Action, 5, 0x478cc0);
            go.key = key::CONTINUE;
            go.activate = Some(callback::CONTINUE);
            let mut reset = MenuItem::new(ItemKind::Action, 0x1a, 0x478cc0);
            reset.key = key::RESET;
            reset.activate = Some(callback::RESET);
            menu.items.extend([view, go, reset]);
        }
        {
            let menu = set.menu_mut(view_stages);
            menu.wrap = false;
            menu.on_enter = Some(callback::VIEW_ENTER);
            let mut item = MenuItem::new(ItemKind::Value, -1, 0x4790a0);
            item.key = key::VIEW_STAGES;
            item.activate = Some(callback::VIEW_RETURN);
            menu.items.push(item);
        }
        let repairs = (0..5).any(|i| live[p + 0x9a + 3 * i] & 0x3f != 0);
        if repairs {
            set.menu_mut(setup_root).links = MenuLinks {
                previous: Some((bottom, key::CONTINUE)),
                next: Some((repair, 0)),
                ..MenuLinks::default()
            };
            set.menu_mut(repair).links = MenuLinks {
                previous: Some((setup_root, 0)),
                next: Some((bottom, key::CONTINUE)),
                ..MenuLinks::default()
            };
            set.menu_mut(bottom).links = MenuLinks {
                decrement: Some((repair, 0)),
                increment: Some((setup_root, 0)),
                ..MenuLinks::default()
            };
        } else {
            set.menu_mut(setup_root).links = MenuLinks {
                previous: Some((bottom, key::CONTINUE)),
                next: Some((bottom, key::CONTINUE)),
                ..MenuLinks::default()
            };
            set.menu_mut(bottom).links = MenuLinks {
                decrement: Some((setup_root, 0)),
                increment: Some((setup_root, 0)),
                ..MenuLinks::default()
            };
        }
        set.activate(bottom, None, 0);
        self.set = set;
        self.setup_root = setup_root;
        self.categories = categories;
        self.repair = repair;
        self.bottom = bottom;
        self.view_stages = view_stages;
        self.current = bottom;
    }

    /// `FUN_00476f70`: one FSM tick with the player's input bits
    /// (`FUN_0043baa0`, edges); `true` once the screen has ended.
    pub fn update(
        &mut self,
        live: &mut [u8],
        input: u32,
        context: ServiceContext,
        tables: ServiceTables,
    ) -> (bool, Vec<ServiceEffect>) {
        let mut effects = Vec::new();
        self.instant = context.instant_fades;
        self.fade_level = self.fade.evaluate();
        match self.phase {
            0 => {
                if self.fade.idle() {
                    self.phase = 1;
                }
            }
            1 => {
                let (next, events) = self.set.step(self.current, input);
                self.run_events(live, &events, tables, &mut effects);
                // A callback may have switched the menu (`FUN_004af3a0`).
                let next = match self.set.pending.take() {
                    Some(pending) => {
                        let (_, post) = self.set.resolve_pending(self.current);
                        self.run_events(live, &post, tables, &mut effects);
                        Some(pending)
                    }
                    None => next,
                };
                if let Some(next) = next {
                    let previous = self.current;
                    self.current = next;
                    let events = self.set.activate(next, Some(previous), 0);
                    self.run_events(live, &events, tables, &mut effects);
                }
            }
            _ => {
                if self.fade.idle() {
                    self.player += 1;
                    if self.player >= self.players {
                        self.finished = true;
                    } else {
                        self.begin_player(live, context, tables);
                    }
                }
            }
        }
        (self.finished, effects)
    }

    fn run_events(
        &mut self,
        live: &mut [u8],
        events: &[MenuEvent],
        tables: ServiceTables,
        effects: &mut Vec<ServiceEffect>,
    ) {
        for event in events {
            match *event {
                MenuEvent::Sound {
                    table, sound, ok, ..
                } => {
                    if let Some(slot) = menu_cue(table, sound) {
                        effects.push(ServiceEffect::Cue { slot, ok });
                    }
                }
                MenuEvent::Activate {
                    menu,
                    item,
                    callback,
                } => {
                    self.activate(live, menu, item, callback, effects);
                }
                MenuEvent::Enter { menu, callback, .. } if callback == callback::VIEW_ENTER => {
                    self.view_stages_enter(live, menu, tables);
                }
                _ => {}
            }
        }
    }

    fn activate(
        &mut self,
        live: &mut [u8],
        menu: MenuId,
        item: usize,
        id: u32,
        effects: &mut Vec<ServiceEffect>,
    ) {
        let p = base(self.player);
        let cue = |slot| ServiceEffect::Cue { slot, ok: true };
        match id {
            // `0x479a90`: the first change away from the start value pays the
            // category's cost once, going back refunds it, and a change the
            // budget cannot cover is refused.
            callback::SETUP_APPLY => {
                let entry = &self.set.menu(menu).items[item];
                let k = (entry.key - key::CATEGORY) as usize;
                let new = entry.value.value as u8;
                let cost = i32::from(live[p + 0x90 + 2 * k] & 0x3f);
                let v = live[p + 0x91 + 2 * k];
                let (current, start) = (v & 7, (v >> 3) & 7);
                if current == new {
                    effects.push(cue(cue::FORWARD));
                } else if new == start {
                    self.budget += cost;
                    live[p + 0x91 + 2 * k] = (v & 0xf8) | new;
                    effects.push(cue(cue::FORWARD));
                } else if self.budget < cost && current == start {
                    effects.push(cue(cue::FORWARD));
                } else {
                    if current == start {
                        self.budget -= cost;
                    }
                    live[p + 0x91 + 2 * k] = (v & 0xf8) | new;
                    effects.push(cue(cue::DO));
                }
                self.set.pending = Some(self.setup_root);
            }
            // `0x4799d0`: repair a part (its cost, condition 100) or undo it.
            callback::REPAIR_TOGGLE => {
                let i = (self.set.menu(menu).items[item].key - key::PART) as usize;
                let a = p + 0x9a + 3 * i;
                let (b, c) = (a + 1, a + 2);
                if live[b] & 0x7f == 100 {
                    if live[c] & 0x7f == 100 {
                        return;
                    }
                    live[b] = (live[b] & 0x80) | (live[c] & 0x7f);
                    self.budget += i32::from(live[a] & 0x3f);
                    effects.push(cue(cue::UNDO));
                } else {
                    let t = i32::from(live[a] & 0x3f);
                    if self.budget < t {
                        return;
                    }
                    live[b] = (live[b] & 0xe4) | 0x64;
                    self.budget -= t;
                    effects.push(cue(cue::DO));
                }
            }
            // `0x479970`: fade out to the next player or the end.
            callback::CONTINUE => {
                self.phase = 2;
                self.fade.fade_out(SERVICE_FADE_FRAMES, self.instant);
                effects.push(cue(cue::FORWARD));
            }
            // `0x4795e0`: the budget, start values and conditions restored.
            callback::RESET => {
                self.budget = SERVICE_BUDGET;
                for k in 0..5 {
                    let v = live[p + 0x91 + 2 * k];
                    live[p + 0x91 + 2 * k] = (v & 0xf8) | ((v >> 3) & 7);
                    let category = self.categories[k];
                    self.set.menu_mut(category).items[0].value.value = i32::from((v >> 3) & 7);
                }
                for i in 0..5 {
                    let b = p + 0x9b + 3 * i;
                    live[b] = (live[b] & 0x80) | (live[b + 1] & 0x7f);
                }
                effects.push(cue(cue::UNDO));
            }
            // `0x479ba0`: back to the bottom bar.
            callback::VIEW_RETURN => {
                self.set.pending = Some(self.bottom);
                effects.push(cue(cue::FORWARD));
            }
            _ => {}
        }
    }

    /// `0x479bc0`: the stages until the next service point.
    fn view_stages_enter(&mut self, live: &[u8], menu: MenuId, tables: ServiceTables) {
        let school = self.school;
        let time_trial = live[0x166] & 3 == 2;
        let count = if school || time_trial {
            1
        } else {
            let stage = i32::from(live[0x168] >> 4);
            let country = usize::from(live[0x168] & 0xf);
            let difficulty = live[0x18] & 0x18 != 0;
            let last = if difficulty {
                i32::from(live[0x165] & 0xf)
            } else {
                3
            };
            let mask = tables.mask.get(country).copied().unwrap_or(0);
            let mut j = stage + 1;
            while j < last && difficulty && (j >= 8 || mask & (1 << j) == 0) {
                j += 1;
            }
            j - stage
        };
        let item = &mut self.set.menu_mut(menu).items[0];
        item.value.value_count = count;
        item.value.value = 0;
        self.stage_view = StageViewReveal::default();
    }

    /// The menus and the screen phase for the draw.
    pub fn menus(&self) -> &MenuSet {
        &self.set
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor_menu::bits;

    const MASK: [u8; 9] = [0x15, 0x65, 0x15, 0x55, 0x15, 0x65, 0x15, 0x55, 0x0f];

    fn live() -> Vec<u8> {
        let mut live = vec![0u8; 0x6c4];
        live[0x18] = 0x08; // a difficulty
        live[0x168] = 0x20; // country 0, stage 2
        live[0x167] = 1;
        for k in 0..5 {
            live[0x90 + 2 * k] = [3, 15, 10, 12, 20][k];
            live[0x91 + 2 * k] = [0, 1, 1, 1, 2][k];
        }
        for i in 0..5 {
            live[0x9b + 3 * i] = 100;
        }
        live
    }

    fn tables() -> ServiceTables<'static> {
        ServiceTables {
            mask: &MASK,
            country_defaults: &[1; 45],
            school_defaults: &[2; 25],
        }
    }

    /// Run until the input phase.
    fn ready(screen: &mut ServiceScreen, live: &mut [u8]) {
        while screen.phase == 0 {
            screen.update(live, 0, ServiceContext::default(), tables());
        }
    }

    fn press(screen: &mut ServiceScreen, live: &mut [u8], input: u32) -> Vec<ServiceEffect> {
        screen
            .update(live, input, ServiceContext::default(), tables())
            .1
    }

    #[test]
    fn service_points_follow_the_country_mask() {
        let mut live = live();
        assert!(
            service_pending(&live, false, false, &MASK),
            "stage 2 of country 0"
        );
        live[0x168] = 0x30;
        assert!(!service_pending(&live, false, false, &MASK));
        live[0x18] = 0;
        assert!(
            service_pending(&live, false, false, &MASK),
            "difficulty 0: every stage"
        );
        assert!(
            !service_pending(&live, false, true, &MASK),
            "the armed special event"
        );
        live[0] = 1;
        assert!(
            !service_pending(&live, false, false, &MASK),
            "the attract demo"
        );
    }

    #[test]
    fn a_setup_change_costs_its_time_once_and_reverting_refunds() {
        let mut live = live();
        let mut screen = ServiceScreen::open(&mut live, ServiceContext::default(), tables());
        assert_eq!(screen.current, screen.bottom, "the bar has the focus");
        assert_eq!(live[0x91] >> 3 & 7, 0, "the start value is snapshot");
        ready(&mut screen, &mut live);
        // Up to the setup root's last item (gear ratio), Return, Left, Return.
        press(&mut screen, &mut live, bits::UP);
        assert_eq!(screen.current, screen.setup_root);
        assert_eq!(screen.set.menu(screen.setup_root).selected, 4);
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!(screen.current, screen.categories[4]);
        press(&mut screen, &mut live, bits::LEFT);
        let effects = press(&mut screen, &mut live, bits::CONFIRM);
        assert!(effects.contains(&ServiceEffect::Cue {
            slot: cue::DO,
            ok: true
        }));
        assert_eq!(live[0x99] & 7, 1);
        assert_eq!(screen.budget, 60 - 20);
        assert_eq!(screen.current, screen.setup_root, "back to the root");
        // Another change in the same category costs nothing more; back to the
        // start value refunds.
        press(&mut screen, &mut live, bits::CONFIRM);
        press(&mut screen, &mut live, bits::LEFT);
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!((live[0x99] & 7, screen.budget), (0, 40));
        press(&mut screen, &mut live, bits::CONFIRM);
        press(&mut screen, &mut live, bits::RIGHT);
        press(&mut screen, &mut live, bits::RIGHT);
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!((live[0x99] & 7, screen.budget), (2, 60));
    }

    #[test]
    fn repairs_cost_time_and_continue_fades_out() {
        let mut live = live();
        live[0x9a] = 30; // electrics need 30 minutes
        live[0x9b] = 40;
        let mut screen = ServiceScreen::open(&mut live, ServiceContext::default(), tables());
        ready(&mut screen, &mut live);
        // The bar's Up reaches the repair list.
        press(&mut screen, &mut live, bits::UP);
        assert_eq!(screen.current, screen.repair);
        press(&mut screen, &mut live, bits::UP);
        press(&mut screen, &mut live, bits::UP);
        press(&mut screen, &mut live, bits::UP);
        press(&mut screen, &mut live, bits::UP);
        assert_eq!(screen.set.menu(screen.repair).selected, 0);
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!((live[0x9b] & 0x7f, screen.budget), (100, 30));
        // Confirming again undoes the repair.
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!((live[0x9b] & 0x7f, screen.budget), (40, 60));
        // Down past the list's last part reaches the bar's Continue.
        for _ in 0..4 {
            press(&mut screen, &mut live, bits::DOWN);
        }
        assert_eq!(screen.current, screen.repair);
        press(&mut screen, &mut live, bits::DOWN);
        assert_eq!(screen.current, screen.bottom);
        assert_eq!(screen.set.menu(screen.bottom).selected, 1, "Continue's key");
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!(screen.phase, 2);
        let mut frames = 0;
        while !screen
            .update(&mut live, 0, ServiceContext::default(), tables())
            .0
        {
            frames += 1;
            assert!(frames < 30);
        }
        assert_eq!(frames, SERVICE_FADE_FRAMES + 2);
    }

    #[test]
    fn a_first_stage_takes_the_country_defaults_and_full_condition() {
        let mut live = live();
        live[0x168] = 0x00;
        live[0x9b] = 50;
        let _ = ServiceScreen::open(&mut live, ServiceContext::default(), tables());
        assert_eq!(live[0x91] & 7, 1, "the country's tyres");
        assert_eq!(live[0x9b] & 0x7f, 100);
        assert_eq!(live[0x9c] & 0x7f, 100, "the start condition snapshot");
    }

    #[test]
    fn the_stage_view_counts_to_the_next_service_point() {
        let mut live = live();
        live[0x165] = 6;
        let mut screen = ServiceScreen::open(&mut live, ServiceContext::default(), tables());
        ready(&mut screen, &mut live);
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!(screen.current, screen.view_stages);
        // Country 0 services at stages 0, 2, 4: from stage 2, two stages.
        assert_eq!(
            screen.set.menu(screen.view_stages).items[0]
                .value
                .value_count,
            2
        );
        press(&mut screen, &mut live, bits::CONFIRM);
        assert_eq!(screen.current, screen.bottom);
    }
}
