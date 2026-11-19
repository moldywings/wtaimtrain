//! Pause menu (Esc) with two pages:
//! - Settings: units, mouse sensitivity, crosshair, key bindings, quit.
//! - Aircraft: flight model presets built from stat cards, and which preset
//!   you and the target fly.
//!
//! The menu is rebuilt whenever a setting changes, which keeps it simple.
//! Text fields are copied into a draft every frame so a rebuild never loses
//! what you typed.

use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input_focus::tab_navigation::{TabGroup, TabIndex};
use bevy::picking::hover::{HoverMap, Hovered};
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle};
use bevy::ui::Pressed;
use bevy::ui_widgets::{
    Activate, Button, SelectAllOnFocus, Slider, SliderPrecision, SliderRange, SliderThumb,
    SliderValue, TextInput, TrackClick, ValueChange,
};

use crate::GameState;
use crate::flight_model::{AircraftCard, EngineKind, FlightModel};
use crate::settings::{
    Action, Crosshair, CrosshairColor, MAX_SENSITIVITY, MIN_SENSITIVITY, RangeId, Scenario,
    Settings, UnitKind, Units, default_bindings, is_builtin, key_name,
};
use crate::weapons::{
    BELLY_BACK_MAX, BELLY_BACK_MIN, CONVERGENCE_CHOICES, GunCard, Loadout, MILS_TO_M_AT_500, Slot,
    TracerColor, WING_OUT_MAX, WING_OUT_MIN, ballistics_at, is_builtin_gun,
};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::input_focus::tab_navigation::TabNavigationPlugin)
            .init_resource::<Rebinding>()
            .init_resource::<MenuUi>()
            .init_resource::<ScrollMemory>()
            .add_observer(on_menu_click)
            .add_observer(on_slider_change)
            .add_observer(on_scroll)
            .add_systems(OnEnter(GameState::Paused), open_menu)
            .add_systems(OnExit(GameState::Paused), (despawn_menu, cancel_rebind))
            .add_systems(
                Update,
                (
                    toggle_pause,
                    capture_rebind
                        .after(toggle_pause)
                        .run_if(in_state(GameState::Paused)),
                    sync_draft.run_if(in_state(GameState::Paused)),
                    rebuild_menu
                        .after(capture_rebind)
                        .after(sync_draft)
                        .run_if(in_state(GameState::Paused)),
                    style_buttons,
                    send_scroll_events.run_if(in_state(GameState::Paused)),
                ),
            );
    }
}

/// The action waiting for a new key, if the player clicked a binding.
#[derive(Resource, Default)]
struct Rebinding(Option<Action>);

#[derive(Component)]
struct PauseMenu;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Settings,
    Units,
    Aircraft,
    Weapons,
    Scenario,
}

/// Menu state that isn't a saved setting.
#[derive(Resource, Default)]
struct MenuUi {
    page: Page,
    /// Name of the preset open in the editor.
    editing: String,
    draft: Draft,
    /// Feedback after saving.
    message: String,
    /// The drop-down list that's open, if any.
    open: Option<Dropdown>,
    /// Weapons page: the gun preset open in the editor, its fields, the
    /// aircraft preset whose loadout is shown, and feedback.
    gun_editing: String,
    gun_draft: GunDraft,
    loadout_for: String,
    gun_message: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dropdown {
    Crosshair,
    Color,
    Gun(Slot),
    LoadoutAircraft,
    Tracer,
    PlayerAircraft,
    TargetAircraft,
    /// One belly or wing position's gun.
    StationGun(Slot, u8),
    Unit(UnitKind),
    Convergence,
}

/// Scrollable lists, so their scroll position survives a menu rebuild.
#[derive(Component, Clone, Copy)]
enum ScrollList {
    Presets,
    Guns,
    Dropdown,
}

/// Remembered scroll offsets, kept outside `MenuUi` so scrolling doesn't
/// rebuild the menu.
#[derive(Resource, Default)]
struct ScrollMemory {
    presets: f32,
    guns: f32,
    dropdown: f32,
}

impl ScrollMemory {
    fn get_mut(&mut self, list: ScrollList) -> &mut f32 {
        match list {
            ScrollList::Presets => &mut self.presets,
            ScrollList::Guns => &mut self.guns,
            ScrollList::Dropdown => &mut self.dropdown,
        }
    }
}

/// Height of one button row in a list, including the gap.
const ROW_STEP: f32 = 31.0;
/// Show this many rows in a drop-down before it scrolls.
const DROPDOWN_ROWS: f32 = 5.5;
/// Show this many presets before the list scrolls.
const PRESET_ROWS: f32 = 7.5;

/// The aircraft editor's fields, as typed, in the current display units.
#[derive(Clone, Default)]
struct Draft {
    engine: Option<EngineKind>,
    afterburner: Option<bool>,
    fields: [String; Field::ALL.len()],
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Power,
    Engines,
    AbThrust,
    Climb,
    SustainedRate,
    SustainedSpeed,
    InstantRate,
    InstantSpeed,
    TopSpeed,
    TopSpeedAltitude,
    Mass,
    WingArea,
    ClMax,
    RollRate,
    RollSpeed,
}

impl Field {
    const ALL: [Field; 16] = [
        Field::Name,
        Field::Power,
        Field::Engines,
        Field::AbThrust,
        Field::Climb,
        Field::InstantRate,
        Field::InstantSpeed,
        Field::SustainedRate,
        Field::SustainedSpeed,
        Field::TopSpeed,
        Field::TopSpeedAltitude,
        Field::Mass,
        Field::WingArea,
        Field::ClMax,
        Field::RollRate,
        Field::RollSpeed,
    ];

    fn index(self) -> usize {
        Field::ALL.iter().position(|f| *f == self).unwrap()
    }

    /// Label and unit shown in the editor.
    fn label(self, units: Units, engine: EngineKind) -> (&'static str, &'static str) {
        match self {
            Field::Name => ("Name", ""),
            Field::Power => match engine {
                EngineKind::Prop => ("Power per engine", units.get(UnitKind::Power).name),
                EngineKind::Jet => ("Thrust per engine", units.get(UnitKind::Thrust).name),
            },
            Field::Engines => ("Engines", ""),
            Field::AbThrust => (
                "  with afterburner (optional)",
                units.get(UnitKind::Thrust).name,
            ),
            Field::Climb => ("Climb rate", units.climb_unit().0),
            Field::SustainedRate => ("Max sustained turn", "deg/s"),
            Field::SustainedSpeed => ("  sustained turn speed", units.speed_unit().0),
            Field::InstantRate => ("Max instant. turn", "deg/s"),
            Field::InstantSpeed => ("  instantaneous turn speed", units.speed_unit().0),
            Field::TopSpeed => ("Top speed", units.speed_unit().0),
            Field::TopSpeedAltitude => ("  at altitude (optional)", units.altitude_unit().0),
            Field::Mass => ("Mass (optional)", units.get(UnitKind::Mass).name),
            Field::WingArea => ("Wing area (optional)", units.get(UnitKind::Area).name),
            Field::ClMax => ("Max lift coeff. (optional)", ""),
            Field::RollRate => ("Best roll (optional)", "deg/s"),
            Field::RollSpeed => ("  best roll speed", units.speed_unit().0),
        }
    }
}

impl Draft {
    fn from_card(card: &AircraftCard, units: Units) -> Draft {
        let (_, spd) = units.speed_unit();
        let (_, clm) = units.climb_unit();
        let (_, alt) = units.altitude_unit();
        let pwr = power_factor(units, card.engine);
        let (_, kg) = units.unit(UnitKind::Mass);
        let (_, area) = units.unit(UnitKind::Area);
        let n = |v: f32, decimals: usize| {
            let s = format!("{v:.decimals$}");
            if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                s
            }
        };
        let opt = |v: Option<f32>, decimals: usize| v.map(|v| n(v, decimals)).unwrap_or_default();
        let mut fields: [String; Field::ALL.len()] = Default::default();
        for f in Field::ALL {
            fields[f.index()] = match f {
                Field::Name => card.name.clone(),
                Field::Power => n(card.power * pwr, power_decimals(units, card.engine)),
                Field::Engines => card.engines.to_string(),
                Field::AbThrust => opt(
                    card.afterburner_thrust
                        .map(|v| v * units.unit(UnitKind::Thrust).1),
                    units.get(UnitKind::Thrust).decimals,
                ),
                Field::Climb => n(card.climb_rate * clm, 1),
                Field::SustainedRate => n(card.sustained_turn_rate, 2),
                Field::SustainedSpeed => n(card.sustained_turn_speed * spd, 0),
                Field::InstantRate => n(card.instant_turn_rate, 2),
                Field::InstantSpeed => n(card.instant_turn_speed * spd, 0),
                Field::TopSpeed => n(card.top_speed * spd, 0),
                Field::TopSpeedAltitude => opt(card.top_speed_altitude.map(|v| v * alt), 0),
                Field::Mass => opt(card.mass.map(|v| v * kg), 0),
                Field::WingArea => opt(card.wing_area.map(|v| v * area), 2),
                Field::ClMax => opt(card.cl_max, 2),
                Field::RollRate => opt(card.roll_rate, 0),
                Field::RollSpeed => opt(card.roll_speed.map(|v| v * spd), 0),
            };
        }
        Draft {
            engine: Some(card.engine),
            afterburner: Some(card.afterburner),
            fields,
        }
    }

    fn get(&self, f: Field) -> &str {
        self.fields[f.index()].trim()
    }

    /// Build a card from the fields. Blank optional fields stay blank, and
    /// the flight model fills them in.
    fn to_card(&self, base: &AircraftCard, units: Units) -> Result<AircraftCard, String> {
        let (_, spd) = units.speed_unit();
        let (_, clm) = units.climb_unit();
        let (_, alt) = units.altitude_unit();
        let engine = self.engine.unwrap_or(base.engine);
        let pwr = power_factor(units, engine);
        let (_, kg) = units.unit(UnitKind::Mass);
        let (_, area) = units.unit(UnitKind::Area);
        let name = self.get(Field::Name).to_string();
        if name.is_empty() {
            return Err("Give the preset a name.".into());
        }
        let num = |f: Field, required: bool, fallback: f32, scale: f32| -> Result<f32, String> {
            let text = self.get(f);
            if text.is_empty() {
                return if required {
                    Err(format!("{} is needed.", f.label(units, EngineKind::Prop).0))
                } else {
                    Ok(fallback)
                };
            }
            match text.replace(',', "").parse::<f32>() {
                Ok(v) if v.is_finite() && v >= 0.0 => Ok(v / scale),
                _ => Err(format!(
                    "\"{text}\" isn't a number ({}).",
                    f.label(units, EngineKind::Prop).0.trim()
                )),
            }
        };
        let optional = |f: Field, scale: f32| -> Result<Option<f32>, String> {
            if self.get(f).is_empty() {
                Ok(None)
            } else {
                num(f, true, 0.0, scale).map(Some)
            }
        };
        let card = AircraftCard {
            name,
            engine,
            power: num(Field::Power, true, base.power, pwr)?,
            engines: num(Field::Engines, true, base.engines as f32, 1.0)?
                .round()
                .max(1.0) as u32,
            climb_rate: num(Field::Climb, true, base.climb_rate, clm)?,
            sustained_turn_rate: num(Field::SustainedRate, true, base.sustained_turn_rate, 1.0)?,
            sustained_turn_speed: num(Field::SustainedSpeed, true, base.sustained_turn_speed, spd)?,
            instant_turn_rate: num(Field::InstantRate, true, base.instant_turn_rate, 1.0)?,
            instant_turn_speed: num(Field::InstantSpeed, true, base.instant_turn_speed, spd)?,
            top_speed: num(Field::TopSpeed, true, base.top_speed, spd)?,
            top_speed_altitude: optional(Field::TopSpeedAltitude, alt)?,
            mass: optional(Field::Mass, kg)?,
            wing_area: optional(Field::WingArea, area)?,
            cl_max: optional(Field::ClMax, 1.0)?,
            roll_rate: optional(Field::RollRate, 1.0)?,
            roll_speed: optional(Field::RollSpeed, spd)?,
            afterburner: self.afterburner.unwrap_or(base.afterburner),
            afterburner_thrust: optional(Field::AbThrust, units.unit(UnitKind::Thrust).1)?,
            weapons: base.weapons.clone(),
        };
        if card.power <= 0.0 || card.climb_rate <= 0.0 || card.top_speed <= 0.0 {
            return Err("Power, climb rate and top speed must be above zero.".into());
        }
        if !(1.0..=90.0).contains(&card.sustained_turn_rate)
            || !(1.0..=120.0).contains(&card.instant_turn_rate)
        {
            return Err("Turn rates are in degrees per second (a Yak-3 sustains about 24).".into());
        }
        if card.sustained_turn_speed < 20.0 || card.instant_turn_speed < 20.0 {
            return Err("Give a speed for each turn rate.".into());
        }
        Ok(card)
    }
}

/// Factor from the stored power (hp) or thrust (kgf) to the shown unit.
fn power_factor(units: Units, engine: EngineKind) -> f32 {
    match engine {
        EngineKind::Prop => units.unit(UnitKind::Power).1,
        EngineKind::Jet => units.unit(UnitKind::Thrust).1,
    }
}

fn power_decimals(units: Units, engine: EngineKind) -> usize {
    match engine {
        EngineKind::Prop => units.get(UnitKind::Power).decimals,
        EngineKind::Jet => units.get(UnitKind::Thrust).decimals,
    }
}

/// The gun editor's fields.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum GunField {
    Name,
    Velocity,
    Mass,
    FireRate,
    Caliber,
    Spread,
    Hits,
}

impl GunField {
    const ALL: [GunField; 7] = [
        GunField::Name,
        GunField::Velocity,
        GunField::Mass,
        GunField::FireRate,
        GunField::Caliber,
        GunField::Spread,
        GunField::Hits,
    ];

    fn index(self) -> usize {
        GunField::ALL.iter().position(|f| *f == self).unwrap()
    }

    fn label(self) -> (&'static str, &'static str) {
        match self {
            GunField::Name => ("Name", ""),
            GunField::Velocity => ("Muzzle velocity", "m/s"),
            GunField::Mass => ("Projectile mass", "kg"),
            GunField::FireRate => ("Fire rate", "rpm"),
            GunField::Caliber => ("Caliber (optional)", "mm"),
            GunField::Spread => ("Spread at 500 m (optional)", "m"),
            GunField::Hits => ("Hits to kill (optional)", "hits"),
        }
    }
}

#[derive(Clone, Default)]
struct GunDraft {
    fields: [String; GunField::ALL.len()],
    tracer: TracerColor,
}

impl GunDraft {
    fn from_card(gun: &GunCard) -> GunDraft {
        let n = |v: f32| {
            let s = format!("{v:.3}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        };
        let mut fields: [String; GunField::ALL.len()] = Default::default();
        for f in GunField::ALL {
            fields[f.index()] = match f {
                GunField::Name => gun.name.clone(),
                GunField::Velocity => n(gun.muzzle_velocity),
                GunField::Mass => n(gun.projectile_mass),
                GunField::FireRate => n(gun.fire_rate),
                GunField::Caliber => gun.caliber.map(n).unwrap_or_default(),
                GunField::Spread => gun
                    .spread
                    .map(|v| n(v * MILS_TO_M_AT_500))
                    .unwrap_or_default(),
                GunField::Hits => gun.hits_to_kill.map(n).unwrap_or_default(),
            };
        }
        GunDraft {
            fields,
            tracer: gun.tracer,
        }
    }

    fn get(&self, f: GunField) -> &str {
        self.fields[f.index()].trim()
    }

    fn to_card(&self) -> Result<GunCard, String> {
        let name = self.get(GunField::Name).to_string();
        if name.is_empty() {
            return Err("Give the gun a name.".into());
        }
        let num = |f: GunField| -> Result<Option<f32>, String> {
            let text = self.get(f);
            if text.is_empty() {
                return Ok(None);
            }
            match text.replace(',', "").parse::<f32>() {
                Ok(v) if v.is_finite() && v > 0.0 => Ok(Some(v)),
                _ => Err(format!(
                    "\"{text}\" isn't a number above zero ({}).",
                    f.label().0
                )),
            }
        };
        let need = |f: GunField| -> Result<f32, String> {
            num(f)?.ok_or_else(|| format!("{} is needed.", f.label().0))
        };
        let gun = GunCard {
            name,
            muzzle_velocity: need(GunField::Velocity)?,
            projectile_mass: need(GunField::Mass)?,
            fire_rate: need(GunField::FireRate)?,
            caliber: num(GunField::Caliber)?,
            spread: num(GunField::Spread)?.map(|m| m / MILS_TO_M_AT_500),
            tracer: self.tracer,
            hits_to_kill: num(GunField::Hits)?.map(|h| h.round().max(1.0)),
        };
        if gun.projectile_mass > 5.0 {
            return Err("Projectile mass is in kg (a 20 mm round is about 0.1).".into());
        }
        Ok(gun)
    }
}

#[derive(Component, Clone)]
enum MenuAction {
    Resume,
    Page(Page),
    SetUnit(UnitKind, u8),
    ToggleLead,
    ToggleEnemyFire,
    PickScenario(Scenario),
    /// Move a start distance: (which, which end: 0 near, 1 far; steps).
    Range(RangeId, u8, i32),
    SensitivityDown,
    SensitivityUp,
    LookSensitivityDown,
    LookSensitivityUp,
    ZoomSensitivityDown,
    ZoomSensitivityUp,
    ToggleDropdown(Dropdown),
    PickCrosshair(Crosshair),
    PickColor(CrosshairColor),
    Rebind(Action),
    ResetBindings,
    Quit,
    EditPreset(String),
    NewPreset,
    DeletePreset,
    ToggleEngine,
    ToggleAfterburner,
    SavePreset,
    PickPlayerAircraft(String),
    PickTargetAircraft(String),
    EditGun(String),
    NewGun,
    DeleteGun,
    SaveGun,
    PickLoadoutAircraft(String),
    PickTracer(TracerColor),
    PickMountGun(Slot, String),
    MountCount(Slot, i32),
    PickStationGun(Slot, u8, String),
    /// Gun targeting distance, m (0 = off).
    SetConvergence(u32),
}

const PANEL_BG: Color = Color::srgba(0.08, 0.09, 0.11, 0.94);
const BUTTON_BG: Color = Color::srgb(0.18, 0.20, 0.24);
const BUTTON_HOVER: Color = Color::srgb(0.27, 0.30, 0.36);
const BUTTON_PRESS: Color = Color::srgb(0.35, 0.55, 0.40);
const BUTTON_SELECTED: Color = Color::srgb(0.42, 0.34, 0.16);
const TEXT: Color = Color::srgb(0.92, 0.92, 0.94);
const DIM: Color = Color::srgb(0.62, 0.64, 0.68);
const ACCENT: Color = Color::srgb(1.0, 0.80, 0.35);

fn toggle_pause(
    keys: Res<ButtonInput<KeyCode>>,
    rebinding: Res<Rebinding>,
    state: Res<State<GameState>>,
    mut next: ResMut<NextState<GameState>>,
) {
    // While choosing a key, Esc cancels the rebind instead (see capture_rebind).
    if !keys.just_pressed(KeyCode::Escape) || rebinding.0.is_some() {
        return;
    }
    next.set(match state.get() {
        GameState::Playing => GameState::Paused,
        GameState::Paused => GameState::Playing,
    });
}

fn capture_rebind(
    mut inputs: MessageReader<KeyboardInput>,
    mut rebinding: ResMut<Rebinding>,
    mut settings: ResMut<Settings>,
) {
    let Some(action) = rebinding.0 else {
        inputs.clear();
        return;
    };
    for input in inputs.read() {
        if input.state != ButtonState::Pressed {
            continue;
        }
        if input.key_code != KeyCode::Escape {
            settings.rebind(action, input.key_code);
        }
        rebinding.0 = None;
        break;
    }
}

fn cancel_rebind(mut rebinding: ResMut<Rebinding>) {
    rebinding.0 = None;
}

fn despawn_menu(mut commands: Commands, menus: Query<Entity, With<PauseMenu>>) {
    for menu in &menus {
        commands.entity(menu).despawn();
    }
}

/// Open the editor on the preset you're flying.
fn open_menu(settings: Res<Settings>, mut ui: ResMut<MenuUi>) {
    ui.editing = settings.player_card().name.clone();
    ui.draft = Draft::from_card(settings.player_card(), settings.units);
    ui.message.clear();
    ui.gun_message.clear();
    ui.open = None;
    if !settings.guns.iter().any(|g| g.name == ui.gun_editing) {
        ui.gun_editing = settings.guns[0].name.clone();
    }
    ui.gun_draft = GunDraft::from_card(settings.gun(&ui.gun_editing));
    ui.loadout_for = settings.player_aircraft.clone();
    // Testing aid: WTAT_PAGE=aircraft or weapons opens straight onto that page.
    match std::env::var("WTAT_PAGE").as_deref() {
        Ok("aircraft") => ui.page = Page::Aircraft,
        Ok("weapons") => ui.page = Page::Weapons,
        Ok("scenario") => ui.page = Page::Scenario,
        Ok("units") => ui.page = Page::Units,
        _ => {}
    }
}

/// Copy what's typed into the draft, without triggering a rebuild.
fn sync_draft(
    mut ui: ResMut<MenuUi>,
    inputs: Query<(&Field, &EditableText)>,
    gun_inputs: Query<(&GunField, &EditableText)>,
) {
    // A click just replaced the draft (new preset, save, ...): the fields on
    // screen are stale until the rebuild, so don't copy them back.
    if ui.is_changed() {
        return;
    }
    let ui = ui.bypass_change_detection();
    for (field, text) in &inputs {
        let value = text.value().to_string();
        if ui.draft.fields[field.index()] != value {
            ui.draft.fields[field.index()] = value;
        }
    }
    for (field, text) in &gun_inputs {
        let value = text.value().to_string();
        if ui.gun_draft.fields[field.index()] != value {
            ui.gun_draft.fields[field.index()] = value;
        }
    }
}

fn rebuild_menu(
    mut commands: Commands,
    settings: Res<Settings>,
    rebinding: Res<Rebinding>,
    ui: Res<MenuUi>,
    scroll: Res<ScrollMemory>,
    menus: Query<Entity, With<PauseMenu>>,
) {
    let exists = !menus.is_empty();
    if exists && !settings.is_changed() && !rebinding.is_changed() && !ui.is_changed() {
        return;
    }
    for menu in &menus {
        commands.entity(menu).despawn();
    }

    commands
        .spawn((
            PauseMenu,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            GlobalZIndex(10),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(px(22)),
                    row_gap: px(10),
                    border_radius: BorderRadius::all(px(10)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|panel| {
                panel.spawn(row()).with_children(|r| {
                    r.spawn((
                        label("Paused", 28.0, TEXT),
                        Node {
                            margin: UiRect::right(px(24)),
                            ..default()
                        },
                    ));
                    for (page, name) in [
                        (Page::Settings, "Settings"),
                        (Page::Units, "Units"),
                        (Page::Aircraft, "Aircraft"),
                        (Page::Weapons, "Weapons"),
                        (Page::Scenario, "Scenario"),
                    ] {
                        let mut b = r.spawn(button(MenuAction::Page(page), name, px(104)));
                        if ui.page == page {
                            b.insert(Selected);
                        }
                    }
                });
                match ui.page {
                    Page::Settings => settings_page(panel, &settings, &rebinding, &ui, &scroll),
                    Page::Units => units_page(panel, &settings, &ui, &scroll),
                    Page::Aircraft => aircraft_page(panel, &settings, &ui, &scroll),
                    Page::Weapons => weapons_page(panel, &settings, &ui, &scroll),
                    Page::Scenario => scenario_page(panel, &settings),
                }
            });
        });
}

fn settings_page(
    panel: &mut ChildSpawnerCommands,
    settings: &Settings,
    rebinding: &Rebinding,
    ui: &MenuUi,
    scroll: &ScrollMemory,
) {
    panel
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: px(28),
            ..default()
        })
        .with_children(|cols| {
            // Left column: general settings.
            cols.spawn(column(px(300))).with_children(|col| {
                col.spawn(button(MenuAction::Resume, "Resume", percent(100)));

                col.spawn(section_header("Aim sensitivity"));
                col.spawn(row()).with_children(|r| {
                    r.spawn(button(MenuAction::SensitivityDown, "-", px(44)));
                    r.spawn((
                        label(&format!("{:.2}x", settings.sensitivity), 18.0, TEXT),
                        Node {
                            flex_grow: 1.0,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        TextLayout::justify(Justify::Center),
                    ));
                    r.spawn(button(MenuAction::SensitivityUp, "+", px(44)));
                });

                col.spawn(section_header("Free look sensitivity"));
                col.spawn(row()).with_children(|r| {
                    r.spawn(button(MenuAction::LookSensitivityDown, "-", px(44)));
                    r.spawn((
                        label(&format!("{:.2}x", settings.look_sensitivity), 18.0, TEXT),
                        Node {
                            flex_grow: 1.0,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        TextLayout::justify(Justify::Center),
                    ));
                    r.spawn(button(MenuAction::LookSensitivityUp, "+", px(44)));
                });

                col.spawn(section_header("Zoomed aim sensitivity"));
                col.spawn(row()).with_children(|r| {
                    r.spawn(button(MenuAction::ZoomSensitivityDown, "-", px(44)));
                    r.spawn((
                        label(&format!("{:.2}x", settings.zoom_sensitivity), 18.0, TEXT),
                        Node {
                            flex_grow: 1.0,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        TextLayout::justify(Justify::Center),
                    ));
                    r.spawn(button(MenuAction::ZoomSensitivityUp, "+", px(44)));
                });

                col.spawn(section_header("Crosshair"));
                let styles: Vec<_> = Crosshair::ALL
                    .iter()
                    .map(|c| Choice {
                        action: MenuAction::PickCrosshair(*c),
                        label: c.label().into(),
                        selected: *c == settings.crosshair,
                        swatch: None,
                    })
                    .collect();
                dropdown(
                    col,
                    Dropdown::Crosshair,
                    ui,
                    scroll,
                    settings.crosshair.label(),
                    None,
                    &styles,
                    percent(100),
                );

                col.spawn(section_header("Crosshair color"));
                let colors: Vec<_> = CrosshairColor::ALL
                    .iter()
                    .map(|c| Choice {
                        action: MenuAction::PickColor(*c),
                        label: c.label().into(),
                        selected: *c == settings.crosshair_color,
                        swatch: Some(swatch(*c)),
                    })
                    .collect();
                dropdown(
                    col,
                    Dropdown::Color,
                    ui,
                    scroll,
                    settings.crosshair_color.label(),
                    Some(swatch(settings.crosshair_color)),
                    &colors,
                    percent(100),
                );

                col.spawn(section_header("Lead indicator (biggest gun)"));
                col.spawn(button(
                    MenuAction::ToggleLead,
                    if settings.lead_indicator { "On" } else { "Off" },
                    percent(100),
                ));

                col.spawn(section_header("Mouse"));
                col.spawn(label(
                    "Move: aim\nRight click: toggle zoom\nLeft click (hold): fire",
                    15.0,
                    TEXT,
                ));

                col.spawn(Node {
                    flex_grow: 1.0,
                    min_height: px(16),
                    ..default()
                });
                col.spawn(button(MenuAction::Quit, "Quit", percent(100)));
                col.spawn(label("Esc: resume", 14.0, DIM));
            });

            // Right column: key bindings.
            cols.spawn(column(px(380))).with_children(|col| {
                col.spawn(section_header("Key bindings (click one, then press a key)"));
                for action in Action::ALL {
                    let waiting = rebinding.0 == Some(action);
                    let key = if waiting {
                        "Press a key...".to_string()
                    } else {
                        key_name(settings.key(action))
                    };
                    col.spawn(row()).with_children(|r| {
                        r.spawn((
                            label(action.label(), 16.0, if waiting { ACCENT } else { TEXT }),
                            Node {
                                flex_grow: 1.0,
                                ..default()
                            },
                        ));
                        r.spawn(button(MenuAction::Rebind(action), &key, px(140)));
                    });
                }
                col.spawn(button(
                    MenuAction::ResetBindings,
                    "Reset key bindings",
                    percent(100),
                ));
            });
        });
}

fn aircraft_page(
    panel: &mut ChildSpawnerCommands,
    settings: &Settings,
    ui: &MenuUi,
    scroll: &ScrollMemory,
) {
    let units = settings.units;
    panel
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: px(28),
            ..default()
        })
        .with_children(|cols| {
            // Left: presets and who flies what.
            cols.spawn(column(px(260))).with_children(|col| {
                let choices = |pick: fn(String) -> MenuAction, current: &str| -> Vec<Choice> {
                    settings
                        .aircraft
                        .iter()
                        .map(|c| Choice {
                            action: pick(c.name.clone()),
                            label: c.name.clone(),
                            selected: c.name == current,
                            swatch: None,
                        })
                        .collect()
                };
                col.spawn(section_header("You fly"));
                col.spawn((Node::default(), GlobalZIndex(22)))
                    .with_children(|w| {
                        dropdown(
                            w,
                            Dropdown::PlayerAircraft,
                            ui,
                            scroll,
                            &settings.player_aircraft,
                            None,
                            &choices(MenuAction::PickPlayerAircraft, &settings.player_aircraft),
                            px(260),
                        );
                    });
                col.spawn(section_header("Target flies"));
                col.spawn((Node::default(), GlobalZIndex(21)))
                    .with_children(|w| {
                        dropdown(
                            w,
                            Dropdown::TargetAircraft,
                            ui,
                            scroll,
                            &settings.target_aircraft,
                            None,
                            &choices(MenuAction::PickTargetAircraft, &settings.target_aircraft),
                            px(260),
                        );
                    });

                col.spawn(section_header(&format!(
                    "Presets ({}, click to edit)",
                    settings.aircraft.len()
                )));
                col.spawn((
                    scroll_list(ScrollList::Presets, scroll.presets),
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(3),
                        max_height: px(ROW_STEP * PRESET_ROWS),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|list| {
                    for card in &settings.aircraft {
                        let text = if is_builtin(&card.name) {
                            format!("{} (built in)", card.name)
                        } else {
                            card.name.clone()
                        };
                        let mut b = list.spawn(button(
                            MenuAction::EditPreset(card.name.clone()),
                            &text,
                            percent(100),
                        ));
                        b.insert(Node {
                            width: percent(100),
                            height: px(28),
                            flex_shrink: 0.0,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(6)),
                            ..default()
                        });
                        if card.name == ui.editing {
                            b.insert(Selected);
                        }
                    }
                });
                col.spawn(row()).with_children(|r| {
                    r.spawn(button(MenuAction::NewPreset, "New", percent(48)));
                    r.spawn(button(MenuAction::DeletePreset, "Delete", percent(48)));
                });

                col.spawn(Node {
                    flex_grow: 1.0,
                    min_height: px(12),
                    ..default()
                });
                col.spawn(button(MenuAction::Resume, "Resume", percent(100)));
            });

            // Right: the stat card editor.
            let engine = ui.draft.engine.unwrap_or(EngineKind::Prop);
            cols.spawn((column(px(460)), TabGroup::new(0))).with_children(|col| {
                col.spawn(section_header(
                    "Stat card: enter what you know, the rest is worked out",
                ));
                if is_builtin(&ui.editing) {
                    col.spawn(label(
                        "Built-in preset. Your changes save as a new preset, \
                         and the stock Yak-3 stays as it is.",
                        13.0,
                        ACCENT,
                    ));
                }
                col.spawn(row()).with_children(|r| {
                    r.spawn((label("Engine type", 15.0, TEXT), grow()));
                    r.spawn(button(
                        MenuAction::ToggleEngine,
                        match engine {
                            EngineKind::Prop => "Propeller",
                            EngineKind::Jet => "Jet",
                        },
                        px(150),
                    ));
                    r.spawn(Node {
                        width: px(52),
                        ..default()
                    });
                });
                let afterburner = engine == EngineKind::Jet && ui.draft.afterburner == Some(true);
                if engine == EngineKind::Jet {
                    col.spawn(row()).with_children(|r| {
                        r.spawn((label("Afterburner", 15.0, TEXT), grow()));
                        r.spawn(button(
                            MenuAction::ToggleAfterburner,
                            if afterburner { "On" } else { "Off" },
                            px(150),
                        ));
                        r.spawn(Node {
                            width: px(52),
                            ..default()
                        });
                    });
                }
                for field in Field::ALL {
                    if field == Field::AbThrust && !afterburner {
                        continue;
                    }
                    // Turn rates share a row with their speed:
                    // "Max sustained turn [23.59] deg/s at [175] kt".
                    let speed_of = match field {
                        Field::SustainedSpeed | Field::InstantSpeed | Field::RollSpeed => continue,
                        Field::RollRate => Some(Field::RollSpeed),
                        Field::SustainedRate => Some(Field::SustainedSpeed),
                        Field::InstantRate => Some(Field::InstantSpeed),
                        _ => None,
                    };
                    let (name, unit) = field.label(units, engine);
                    col.spawn(row()).with_children(|r| {
                        r.spawn((label(name, 15.0, TEXT), grow()));
                        if let Some(speed) = speed_of {
                            r.spawn(text_field(
                                field.index() as i32,
                                ui.draft.get(field),
                                field,
                                58.0,
                            ));
                            r.spawn(label("deg/s at", 14.0, DIM));
                            r.spawn(text_field(
                                speed.index() as i32,
                                ui.draft.get(speed),
                                speed,
                                52.0,
                            ));
                            r.spawn((
                                label(units.speed_unit().0, 14.0, DIM),
                                Node {
                                    width: px(52),
                                    ..default()
                                },
                            ));
                            return;
                        }
                        r.spawn(text_field(
                            field.index() as i32,
                            ui.draft.get(field),
                            field,
                            150.0,
                        ));
                        r.spawn((
                            label(unit, 14.0, DIM),
                            Node {
                                width: px(52),
                                ..default()
                            },
                        ));
                    });
                }
                col.spawn(button(MenuAction::SavePreset, "Save preset", percent(100)));
                if !ui.message.is_empty() {
                    col.spawn(label(&ui.message, 14.0, ACCENT));
                }

                // What the saved preset actually does, to compare with the card.
                if let Some(card) = settings.aircraft.iter().find(|c| c.name == ui.editing) {
                    let p = FlightModel::from_card(card).check;
                    let (su, sf) = units.speed_unit();
                    let (cu, cf) = units.climb_unit();
                    let (au, af) = units.altitude_unit();
                    col.spawn(label(
                        &format!(
                            "Model check for \"{}\": top speed {:.0} {su} at {:.0} {au}, {:.0} {su} at sea level. \
                             Climb {:.0} {cu}. Turns {:.1} deg/s sustained at {:.0} {su}, \
                             {:.1} deg/s instantaneous at {:.0} {su} ({:.1} G). Best full circle {:.1} s. \
                             Stalls at {:.0} {su}.",
                            card.name,
                            p.top_speed * sf,
                            card.extras().top_speed_altitude * af,
                            p.top_speed_sea_level * sf,
                            p.climb_rate * cf,
                            p.sustained_turn_rate,
                            card.sustained_turn_speed * sf,
                            p.instant_turn_rate,
                            card.instant_turn_speed * sf,
                            card.instant_turn_g(),
                            p.turn_time,
                            p.stall_speed * sf,
                        ),
                        13.0,
                        DIM,
                    ));
                    // Show what the game picked for any blank optional field.
                    let e = card.extras();
                    let mut filled = vec![];
                    if card.top_speed_altitude.is_none() {
                        filled.push(format!("top speed altitude {:.0} {au}", e.top_speed_altitude * af));
                    }
                    if card.mass.is_none() {
                        filled.push(format!("mass {}", units.fmt(UnitKind::Mass, e.mass)));
                    }
                    if card.wing_area.is_none() {
                        filled.push(format!("wing area {}", units.fmt(UnitKind::Area, e.wing_area)));
                    }
                    if card.cl_max.is_none() {
                        filled.push(format!("max lift {:.2}", e.cl_max));
                    }
                    if card.roll_rate.is_none() {
                        filled.push(format!("roll rate {:.0} deg/s", e.roll_rate));
                    }
                    if card.roll_speed.is_none() {
                        filled.push(format!("best roll speed {}", units.speed(e.roll_speed)));
                    }
                    if !filled.is_empty() {
                        col.spawn(label(
                            &format!("Filled in for you: {}.", filled.join(", ")),
                            13.0,
                            DIM,
                        ));
                    }
                    // Say so when a card asks for something the model can't do.
                    let mut misses = vec![];
                    if (p.top_speed - card.top_speed).abs() > card.top_speed * 0.03 {
                        misses.push("top speed");
                    }
                    if (p.climb_rate - card.climb_rate).abs() > card.climb_rate * 0.05 {
                        misses.push("climb rate");
                    }
                    if (p.sustained_turn_rate - card.sustained_turn_rate).abs()
                        > card.sustained_turn_rate * 0.05
                    {
                        misses.push("sustained turn");
                    }
                    if (p.instant_turn_rate - card.instant_turn_rate).abs()
                        > card.instant_turn_rate * 0.05
                    {
                        misses.push("instantaneous turn");
                    }
                    if !misses.is_empty() {
                        col.spawn(label(
                            &format!(
                                "Can't fully match the {} with these numbers. Check the units, \
                                 or adjust mass, wing area or max lift.",
                                misses.join(" and ")
                            ),
                            13.0,
                            ACCENT,
                        ));
                    }
                }
            });
        });
}

fn weapons_page(
    panel: &mut ChildSpawnerCommands,
    settings: &Settings,
    ui: &MenuUi,
    scroll: &ScrollMemory,
) {
    panel
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: px(28),
            ..default()
        })
        .with_children(|cols| {
            // Left: gun presets and the gun editor.
            cols.spawn((column(px(390)), TabGroup::new(0)))
                .with_children(|col| {
                    col.spawn(section_header(&format!(
                        "Gun presets ({}, click to edit)",
                        settings.guns.len()
                    )));
                    col.spawn((
                        scroll_list(ScrollList::Guns, scroll.guns),
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: px(3),
                            max_height: px(ROW_STEP * 4.5),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ))
                    .with_children(|list| {
                        for gun in &settings.guns {
                            let text = if is_builtin_gun(&gun.name) {
                                format!("{} (built in)", gun.name)
                            } else {
                                gun.name.clone()
                            };
                            let mut b = list.spawn(button(
                                MenuAction::EditGun(gun.name.clone()),
                                &text,
                                percent(100),
                            ));
                            b.insert(Node {
                                width: percent(100),
                                height: px(28),
                                flex_shrink: 0.0,
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border_radius: BorderRadius::all(px(6)),
                                ..default()
                            });
                            if gun.name == ui.gun_editing {
                                b.insert(Selected);
                            }
                        }
                    });
                    col.spawn(row()).with_children(|r| {
                        r.spawn(button(MenuAction::NewGun, "New", percent(48)));
                        r.spawn(button(MenuAction::DeleteGun, "Delete", percent(48)));
                    });

                    col.spawn(section_header("Gun: enter what you know"));
                    if is_builtin_gun(&ui.gun_editing) {
                        col.spawn(label(
                            "Built-in gun. Your changes save as a new gun.",
                            13.0,
                            ACCENT,
                        ));
                    }
                    for field in GunField::ALL {
                        let (name, unit) = field.label();
                        col.spawn(row()).with_children(|r| {
                            r.spawn((label(name, 15.0, TEXT), grow()));
                            r.spawn(text_field(
                                100 + field.index() as i32,
                                ui.gun_draft.get(field),
                                field,
                                150.0,
                            ));
                            r.spawn((
                                label(unit, 14.0, DIM),
                                Node {
                                    width: px(40),
                                    ..default()
                                },
                            ));
                        });
                    }
                    col.spawn((row(), GlobalZIndex(16))).with_children(|r| {
                        r.spawn((label("Tracer color", 15.0, TEXT), grow()));
                        let colors: Vec<_> = TracerColor::ALL
                            .iter()
                            .map(|c| Choice {
                                action: MenuAction::PickTracer(*c),
                                label: c.label().into(),
                                selected: *c == ui.gun_draft.tracer,
                                swatch: Some(c.color()),
                            })
                            .collect();
                        dropdown(
                            r,
                            Dropdown::Tracer,
                            ui,
                            scroll,
                            ui.gun_draft.tracer.label(),
                            Some(ui.gun_draft.tracer.color()),
                            &colors,
                            px(150),
                        );
                        r.spawn(Node {
                            width: px(40),
                            ..default()
                        });
                    });
                    col.spawn(button(MenuAction::SaveGun, "Save gun", percent(100)));
                    if !ui.gun_message.is_empty() {
                        col.spawn(label(&ui.gun_message, 14.0, ACCENT));
                    }
                    if let Some(gun) = settings.guns.iter().find(|g| g.name == ui.gun_editing) {
                        let b = ballistics_at(gun, 1000.0);
                        let mut text = format!(
                            "\"{}\" at 1 km (sea level): {:.2} s flight, {:.0} m/s left, {:.1} m drop.",
                            gun.name, b.time, b.speed, b.drop
                        );
                        let mut filled = vec![];
                        if gun.caliber.is_none() {
                            filled.push(format!("caliber {:.1} mm", gun.caliber_mm()));
                        }
                        if gun.spread.is_none() {
                            filled.push(format!("spread {:.1} m at 500 m", gun.spread_at_500()));
                        }
                        if gun.hits_to_kill.is_none() {
                            filled.push(format!("{} hits to destroy", gun.hits_to_destroy()));
                        }
                        if !filled.is_empty() {
                            text += &format!(" Filled in for you: {}.", filled.join(", "));
                        }
                        col.spawn(label(&text, 13.0, DIM));
                    }
                });

            // Right: where the guns go on an aircraft preset.
            cols.spawn(column(px(400))).with_children(|col| {
                col.spawn(section_header("Loadout for aircraft preset"));
                let aircraft: Vec<_> = settings
                    .aircraft
                    .iter()
                    .map(|c| Choice {
                        action: MenuAction::PickLoadoutAircraft(c.name.clone()),
                        label: c.name.clone(),
                        selected: c.name == ui.loadout_for,
                        swatch: None,
                    })
                    .collect();
                let card = settings.card(&ui.loadout_for);
                col.spawn((Node::default(), GlobalZIndex(20)))
                    .with_children(|w| {
                        dropdown(
                            w,
                            Dropdown::LoadoutAircraft,
                            ui,
                            scroll,
                            &card.name,
                            None,
                            &aircraft,
                            px(400),
                        );
                    });
                if is_builtin(&card.name) {
                    col.spawn(label(
                        "Built-in preset. Changing its guns saves a copy.",
                        13.0,
                        ACCENT,
                    ));
                }
                let loadout = &card.weapons;
                let gun_choices = |pick: &dyn Fn(String) -> MenuAction, current: &str| {
                    settings
                        .guns
                        .iter()
                        .map(|g| Choice {
                            action: pick(g.name.clone()),
                            label: g.name.clone(),
                            selected: g.name == settings.gun(current).name,
                            swatch: None,
                        })
                        .collect::<Vec<_>>()
                };
                for slot in Slot::ALL {
                    let mount = loadout.mount(slot);
                    let count = mount.count.min(slot.max());
                    let mixed = (0..count as usize).any(|i| mount.gun_at(i) != mount.gun);
                    col.spawn(section_header(&format!(
                        "{} (up to {})",
                        slot.label(),
                        slot.max()
                    )));
                    col.spawn((row(), GlobalZIndex(slot_z(slot, 0))))
                        .with_children(|r| {
                            let guns = gun_choices(
                                &|g| MenuAction::PickMountGun(slot, g),
                                &mount.gun,
                            );
                            dropdown(
                                r,
                                Dropdown::Gun(slot),
                                ui,
                                scroll,
                                if mixed {
                                    "Mixed (set each below)"
                                } else {
                                    &settings.gun(&mount.gun).name
                                },
                                None,
                                &guns,
                                px(250),
                            );
                            r.spawn(button(MenuAction::MountCount(slot, -1), "-", px(36)));
                            r.spawn((
                                label(&count.to_string(), 18.0, TEXT),
                                Node {
                                    width: px(30),
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                TextLayout::justify(Justify::Center),
                            ));
                            r.spawn(button(MenuAction::MountCount(slot, 1), "+", px(36)));
                        });
                    let placed = matches!(slot, Slot::Belly | Slot::Wings);
                    if placed && count >= 2 {
                        // A gun for each position, so calibers can be mixed.
                        col.spawn((row(), GlobalZIndex(slot_z(slot, 1))))
                            .with_children(|r| {
                                r.spawn((
                                    label("Each", 13.0, DIM),
                                    Node {
                                        width: px(36),
                                        ..default()
                                    },
                                ));
                                for i in 0..count as u8 {
                                    let gun = mount.gun_at(i as usize).to_string();
                                    let guns = gun_choices(
                                        &|g| MenuAction::PickStationGun(slot, i, g),
                                        &gun,
                                    );
                                    let short = format!(
                                        "{:.0} mm",
                                        settings.gun(&gun).caliber_mm()
                                    );
                                    dropdown(
                                        r,
                                        Dropdown::StationGun(slot, i),
                                        ui,
                                        scroll,
                                        &short,
                                        None,
                                        &guns,
                                        px(84.0),
                                    );
                                }
                            });
                    }
                    if placed && count > 0 {
                        let (id, value, min, max, text) = if slot == Slot::Belly {
                            (
                                SliderId::BellyBack,
                                loadout.belly_back,
                                BELLY_BACK_MIN,
                                BELLY_BACK_MAX,
                                "Behind spinner",
                            )
                        } else {
                            (
                                SliderId::WingOut,
                                loadout.wing_out,
                                WING_OUT_MIN,
                                WING_OUT_MAX,
                                "From wing root",
                            )
                        };
                        col.spawn(row()).with_children(|r| {
                            r.spawn((
                                label(text, 13.0, DIM),
                                Node {
                                    width: px(110),
                                    ..default()
                                },
                            ));
                            r.spawn(slider(id, value, min, max, px(210)));
                            r.spawn((
                                label(&slider_text(id, value, settings.units), 15.0, TEXT),
                                TextLayout::no_wrap(),
                                SliderLabel(id),
                            ));
                        });
                    }
                }
                col.spawn((row(), GlobalZIndex(12))).with_children(|r| {
                    r.spawn((label("Gun targeting distance", 16.0, TEXT), grow()));
                    let choices: Vec<_> = CONVERGENCE_CHOICES
                        .iter()
                        .map(|m| Choice {
                            action: MenuAction::SetConvergence(*m as u32),
                            label: convergence_label(settings.units, *m),
                            selected: *m == loadout.convergence,
                            swatch: None,
                        })
                        .collect();
                    dropdown(
                        r,
                        Dropdown::Convergence,
                        ui,
                        scroll,
                        &convergence_label(settings.units, loadout.convergence),
                        None,
                        &choices,
                        px(210),
                    );
                });
                let guns = loadout.guns();
                let mass_per_s: f32 = guns
                    .iter()
                    .map(|(_, g, _)| {
                        let g = settings.gun(g);
                        g.projectile_mass * g.fire_rate / 60.0
                    })
                    .sum();
                col.spawn(label(
                    &format!(
                        "{} guns, {:.2} kg of rounds per second. {}; nose guns fire \
                         straight ahead.",
                        loadout.total_guns(),
                        mass_per_s,
                        if loadout.convergence > 0.0 {
                            format!(
                                "Wing and belly guns angle in sideways to meet at {}",
                                convergence_text(settings.units, loadout.convergence)
                            )
                        } else {
                            "Targeting distance is off, so wing and belly guns fire parallel"
                                .into()
                        }
                    ),
                    13.0,
                    DIM,
                ));
                col.spawn(Node {
                    flex_grow: 1.0,
                    min_height: px(12),
                    ..default()
                });
                col.spawn(button(MenuAction::Resume, "Resume", percent(100)));
            });
        });
}

fn units_page(
    panel: &mut ChildSpawnerCommands,
    settings: &Settings,
    ui: &MenuUi,
    scroll: &ScrollMemory,
) {
    panel.spawn(column(px(560))).with_children(|col| {
        col.spawn(label(
            "Pick a unit for each. Presets, the readout and the menus all \
             convert by themselves.",
            14.0,
            DIM,
        ));
        for (n, kind) in UnitKind::ALL.into_iter().enumerate() {
            // Higher rows draw over lower ones, so an open list covers
            // the rows beneath it.
            col.spawn((row(), GlobalZIndex(30 - n as i32)))
                .with_children(|r| {
                    r.spawn((label(kind.label(), 16.0, TEXT), grow()));
                    let current = settings.units.index(kind);
                    let choices: Vec<_> = kind
                        .options()
                        .iter()
                        .enumerate()
                        .map(|(i, d)| Choice {
                            action: MenuAction::SetUnit(kind, i as u8),
                            label: d.name.to_string(),
                            selected: i == current,
                            swatch: None,
                        })
                        .collect();
                    dropdown(
                        r,
                        Dropdown::Unit(kind),
                        ui,
                        scroll,
                        settings.units.get(kind).name,
                        None,
                        &choices,
                        px(160),
                    );
                });
        }
        col.spawn(button(MenuAction::Resume, "Resume", percent(100)));
    });
}

fn scenario_page(panel: &mut ChildSpawnerCommands, settings: &Settings) {
    panel
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: px(28),
            ..default()
        })
        .with_children(|cols| {
            cols.spawn(column(px(330))).with_children(|col| {
                col.spawn(section_header("Scenario"));
                for sc in Scenario::ALL {
                    let mut b = col.spawn(button(
                        MenuAction::PickScenario(sc),
                        sc.label(),
                        percent(100),
                    ));
                    if settings.scenario == sc {
                        b.insert(Selected);
                    }
                }
                col.spawn(label(settings.scenario.description(), 14.0, DIM));
                col.spawn(section_header("Enemy shoots back"));
                col.spawn(button(
                    MenuAction::ToggleEnemyFire,
                    if settings.enemy_fires { "On" } else { "Off" },
                    percent(100),
                ));
                col.spawn(label(
                    "When on, the target fires its own guns at you whenever they're \
                     lined up, and in a dogfight it hunts you instead of ramming.",
                    13.0,
                    DIM,
                ));
                col.spawn(label(
                    "Destroy the target and it resets 2 seconds later. It also resets \
                     when a plane crashes, the target gets away, or you press R.",
                    13.0,
                    DIM,
                ));
            });
            cols.spawn(column(px(330))).with_children(|col| {
                let ids = settings.scenario.ranges();
                if ids.is_empty() {
                    col.spawn(section_header("Start distances"));
                    col.spawn(label(
                        "All in one uses the distances set on each scenario. Pick a \
                         scenario to change its own.",
                        13.0,
                        DIM,
                    ));
                }
                for id in ids {
                    col.spawn(section_header(&format!(
                        "{}: {}",
                        settings.scenario.label(),
                        id.label().to_lowercase()
                    )));
                    let span = settings.ranges.get(*id);
                    for (end, name, value) in
                        [(0u8, "Nearest", span.min), (1u8, "Farthest", span.max)]
                    {
                        let name = match (id.is_height(), end) {
                            (true, 0) => "Lowest",
                            (true, _) => "Highest",
                            _ => name,
                        };
                        let text = if id.is_height() {
                            settings.units.altitude(value)
                        } else {
                            settings.units.range(value)
                        };
                        col.spawn(row()).with_children(|r| {
                            r.spawn((
                                label(name, 15.0, TEXT),
                                Node {
                                    width: px(80),
                                    ..default()
                                },
                            ));
                            r.spawn((label(&text, 17.0, TEXT), grow()));
                            r.spawn(button(MenuAction::Range(*id, end, -1), "-", px(36)));
                            r.spawn(button(MenuAction::Range(*id, end, 1), "+", px(36)));
                        });
                    }
                }
                if !ids.is_empty() {
                    col.spawn(label(
                        "Each run picks a random value between these. All in one uses \
                         them too.",
                        13.0,
                        DIM,
                    ));
                }
                col.spawn(Node {
                    flex_grow: 1.0,
                    min_height: px(16),
                    ..default()
                });
                col.spawn(button(MenuAction::Resume, "Resume", percent(100)));
            });
        });
}

/// Convergence range in meters, with feet too outside metric.
fn convergence_text(units: Units, m: f32) -> String {
    // Short distances, like War Thunder's list: meters or feet.
    let (name, per) = units.altitude_unit();
    format!("{:.0} {name}", m * per)
}

/// The convergence dropdown's text for a distance, m ("No" when off).
fn convergence_label(units: Units, m: f32) -> String {
    if m > 0.0 {
        convergence_text(units, m)
    } else {
        "No".into()
    }
}

/// Higher rows draw over lower ones, so an open gun list covers the rows
/// beneath it.
fn slot_z(slot: Slot, sub: i32) -> i32 {
    20 - 2 * Slot::ALL.iter().position(|s| *s == slot).unwrap_or(0) as i32 - sub
}

/// Which loadout setting a slider moves.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum SliderId {
    BellyBack,
    WingOut,
}

/// The text showing a slider's value.
#[derive(Component)]
struct SliderLabel(SliderId);

fn slider_text(_id: SliderId, m: f32, units: Units) -> String {
    let (name, per) = units.altitude_unit();
    format!("{:.1} {name}", m * per)
}

/// A horizontal slider: a rail with a round thumb you drag.
fn slider(id: SliderId, value: f32, min: f32, max: f32, width: Val) -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Stretch,
            height: px(20),
            width,
            margin: UiRect::right(px(10)),
            ..default()
        },
        Hovered::default(),
        id,
        Slider {
            track_click: TrackClick::Snap,
            ..default()
        },
        SliderValue(value),
        SliderRange::new(min, max),
        SliderPrecision(1),
        Children::spawn((
            Spawn((
                Node {
                    height: px(6),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
                BackgroundColor(BUTTON_HOVER),
            )),
            Spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    right: px(16),
                    top: px(0),
                    bottom: px(0),
                    ..default()
                },
                children![(
                    SliderThumb,
                    Node {
                        width: px(16),
                        height: px(16),
                        top: px(2),
                        position_type: PositionType::Absolute,
                        left: percent((value - min) / (max - min) * 100.0),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(ACCENT),
                )],
            )),
        )),
    )
}

/// Dragging a slider: move its thumb and label as it goes, and save the
/// setting when let go.
#[allow(clippy::too_many_arguments)]
fn on_slider_change(
    change: On<ValueChange<f32>>,
    sliders: Query<(&SliderId, &SliderRange, &Children)>,
    tracks: Query<&Children, Without<SliderThumb>>,
    mut thumbs: Query<&mut Node, With<SliderThumb>>,
    mut labels: Query<(&SliderLabel, &mut Text)>,
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    mut ui: ResMut<MenuUi>,
) {
    let Ok((id, range, children)) = sliders.get(change.source) else {
        return;
    };
    let value = range.clamp(change.value);
    commands.entity(change.source).insert(SliderValue(value));
    for child in children.iter() {
        for t in tracks.get(child).into_iter().flatten() {
            if let Ok(mut node) = thumbs.get_mut(*t) {
                node.left = percent(range.thumb_position(value) * 100.0);
            }
        }
    }
    for (l, mut text) in &mut labels {
        if l.0 == *id {
            text.0 = slider_text(*id, value, settings.units);
        }
    }
    if change.is_final {
        let id = *id;
        edit_loadout(&mut settings, &mut ui, |l| match id {
            SliderId::BellyBack => l.belly_back = value,
            SliderId::WingOut => l.wing_out = value,
        });
    }
}

fn grow() -> Node {
    Node {
        flex_grow: 1.0,
        ..default()
    }
}

fn text_field(tab: i32, value: &str, field: impl Component, width: f32) -> impl Bundle {
    (
        Node {
            width: px(width),
            height: px(26),
            padding: UiRect::axes(px(6), px(3)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(4)),
            ..default()
        },
        BorderColor::all(Color::srgb(0.35, 0.38, 0.44)),
        BackgroundColor(Color::srgb(0.12, 0.13, 0.16)),
        TextInput,
        EditableText {
            max_characters: Some(32),
            allow_newlines: false,
            ..EditableText::new(value)
        },
        TextLayout::no_wrap(),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(TEXT),
        TextCursorStyle::default(),
        // Clicking only focuses widgets that have a tab index.
        TabIndex(tab),
        SelectAllOnFocus,
        field,
    )
}

/// One entry in a drop-down list.
struct Choice {
    action: MenuAction,
    label: String,
    selected: bool,
    swatch: Option<Color>,
}

/// The swatch shown next to a crosshair color. "Default" shows the gold
/// most styles use.
fn swatch(c: CrosshairColor) -> Color {
    c.color().unwrap_or(crate::hud::RETICLE_GOLD)
}

fn color_chip(color: Color) -> impl Bundle {
    (
        Node {
            width: px(14),
            height: px(14),
            margin: UiRect::right(px(8)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(3)),
            ..default()
        },
        BackgroundColor(color),
        BorderColor::all(Color::srgba(0.0, 0.0, 0.0, 0.6)),
    )
}

/// A button that opens a scrollable list below it.
#[allow(clippy::too_many_arguments)]
fn dropdown(
    col: &mut ChildSpawnerCommands,
    kind: Dropdown,
    ui: &MenuUi,
    scroll: &ScrollMemory,
    current: &str,
    current_swatch: Option<Color>,
    choices: &[Choice],
    width: Val,
) {
    let open = ui.open == Some(kind);
    col.spawn(Node { width, ..default() })
        .with_children(|wrap| {
            let mut head = wrap.spawn((
                Button,
                Hovered::default(),
                MenuAction::ToggleDropdown(kind),
                Node {
                    width: percent(100),
                    height: px(28),
                    padding: UiRect::horizontal(px(10)),
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(BUTTON_BG),
            ));
            if open {
                head.insert(Selected);
            }
            head.with_children(|b| {
                if let Some(c) = current_swatch {
                    b.spawn(color_chip(c));
                }
                b.spawn((label(current, 16.0, TEXT), grow(), TextLayout::no_wrap()));
                b.spawn(label(if open { "^" } else { "v" }, 15.0, DIM));
            });

            if !open {
                return;
            }
            wrap.spawn((
                scroll_list(ScrollList::Dropdown, scroll.dropdown),
                Node {
                    position_type: PositionType::Absolute,
                    top: px(31),
                    left: px(0),
                    width: percent(100),
                    // Narrow lists still fit the gun names.
                    min_width: px(210),
                    max_height: px(ROW_STEP * DROPDOWN_ROWS),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(3),
                    padding: UiRect::all(px(3)),
                    overflow: Overflow::scroll_y(),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.11, 0.13)),
                BorderColor::all(Color::srgb(0.35, 0.38, 0.44)),
                GlobalZIndex(40),
            ))
            .with_children(|list| {
                for choice in choices {
                    let mut item = list.spawn((
                        Button,
                        Hovered::default(),
                        choice.action.clone(),
                        Node {
                            width: percent(100),
                            height: px(28),
                            flex_shrink: 0.0,
                            padding: UiRect::horizontal(px(8)),
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(4)),
                            ..default()
                        },
                        BackgroundColor(BUTTON_BG),
                    ));
                    if choice.selected {
                        item.insert(Selected);
                    }
                    item.with_children(|b| {
                        if let Some(c) = choice.swatch {
                            b.spawn(color_chip(c));
                        }
                        b.spawn(label(&choice.label, 16.0, TEXT));
                    });
                }
            });
        });
}

fn scroll_list(list: ScrollList, offset: f32) -> impl Bundle {
    (list, ScrollPosition(Vec2::new(0.0, offset)))
}

/// Line height for wheels that scroll by lines.
const WHEEL_LINE: f32 = 24.0;

/// Mouse wheel scrolling for the lists, sent to whatever is under the pointer
/// and passed up to the nearest scrollable parent.
fn send_scroll_events(
    mut wheel: MessageReader<MouseWheel>,
    hover_map: Res<HoverMap>,
    mut commands: Commands,
) {
    for event in wheel.read() {
        let mut dy = -event.y;
        if event.unit == MouseScrollUnit::Line {
            dy *= WHEEL_LINE;
        }
        for pointer_map in hover_map.values() {
            for entity in pointer_map.keys().copied() {
                commands.trigger(Scroll { entity, dy });
            }
        }
    }
}

#[derive(EntityEvent, Debug)]
#[entity_event(propagate, auto_propagate)]
struct Scroll {
    entity: Entity,
    dy: f32,
}

fn on_scroll(
    mut scroll: On<Scroll>,
    mut lists: Query<(&ScrollList, &mut ScrollPosition, &ComputedNode)>,
    mut memory: ResMut<ScrollMemory>,
) {
    let Ok((list, mut pos, computed)) = lists.get_mut(scroll.entity) else {
        return;
    };
    let max = ((computed.content_size().y - computed.size().y) * computed.inverse_scale_factor())
        .max(0.0);
    let y = (pos.y + scroll.dy).clamp(0.0, max);
    if y != pos.y {
        pos.y = y;
        *memory.bypass_change_detection().get_mut(*list) = y;
    }
    scroll.propagate(false);
}

/// Marks the active tab or preset button.
#[derive(Component)]
struct Selected;

fn column(width: Val) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        width,
        row_gap: px(5),
        ..default()
    }
}

#[allow(clippy::too_many_arguments)]
fn on_menu_click(
    activate: On<Activate>,
    actions: Query<&MenuAction>,
    mut settings: ResMut<Settings>,
    mut rebinding: ResMut<Rebinding>,
    mut ui: ResMut<MenuUi>,
    mut scroll: ResMut<ScrollMemory>,
    mut next: ResMut<NextState<GameState>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(action) = actions.get(activate.entity) else {
        return;
    };
    // Any click other than opening a list closes the open one.
    if !matches!(action, MenuAction::ToggleDropdown(_)) && ui.open.is_some() {
        ui.open = None;
    }
    match action.clone() {
        MenuAction::Resume => next.set(GameState::Playing),
        MenuAction::Page(page) => ui.page = page,
        MenuAction::ToggleLead => settings.lead_indicator = !settings.lead_indicator,
        MenuAction::ToggleEnemyFire => settings.enemy_fires = !settings.enemy_fires,
        MenuAction::PickScenario(sc) => settings.scenario = sc,
        MenuAction::Range(id, end, steps) => {
            settings.ranges.step(id, end, steps);
        }
        MenuAction::SetUnit(kind, i) => {
            settings.units.set(kind, i);
            // Re-show the editor in the new units.
            let card = settings.card(&ui.editing).clone();
            ui.draft = Draft::from_card(&card, settings.units);
        }
        MenuAction::SensitivityDown => {
            settings.sensitivity = step_sensitivity(settings.sensitivity, -1.0);
        }
        MenuAction::SensitivityUp => {
            settings.sensitivity = step_sensitivity(settings.sensitivity, 1.0);
        }
        MenuAction::LookSensitivityDown => {
            settings.look_sensitivity = step_sensitivity(settings.look_sensitivity, -1.0);
        }
        MenuAction::LookSensitivityUp => {
            settings.look_sensitivity = step_sensitivity(settings.look_sensitivity, 1.0);
        }
        MenuAction::ZoomSensitivityDown => {
            settings.zoom_sensitivity = step_sensitivity(settings.zoom_sensitivity, -1.0);
        }
        MenuAction::ZoomSensitivityUp => {
            settings.zoom_sensitivity = step_sensitivity(settings.zoom_sensitivity, 1.0);
        }
        MenuAction::ToggleDropdown(d) => {
            if ui.open == Some(d) {
                ui.open = None;
            } else {
                ui.open = Some(d);
                // Open with the current choice in view.
                let index = match d {
                    Dropdown::Crosshair => {
                        Crosshair::ALL.iter().position(|c| *c == settings.crosshair)
                    }
                    Dropdown::Color => CrosshairColor::ALL
                        .iter()
                        .position(|c| *c == settings.crosshair_color),
                    Dropdown::Gun(slot) => {
                        let gun = &settings.card(&ui.loadout_for).weapons.mount(slot).gun;
                        settings.guns.iter().position(|g| &g.name == gun)
                    }
                    Dropdown::LoadoutAircraft => settings
                        .aircraft
                        .iter()
                        .position(|c| c.name == ui.loadout_for),
                    Dropdown::PlayerAircraft => settings
                        .aircraft
                        .iter()
                        .position(|c| c.name == settings.player_aircraft),
                    Dropdown::TargetAircraft => settings
                        .aircraft
                        .iter()
                        .position(|c| c.name == settings.target_aircraft),
                    Dropdown::Tracer => TracerColor::ALL
                        .iter()
                        .position(|c| *c == ui.gun_draft.tracer),
                    Dropdown::StationGun(slot, i) => {
                        let loadout = &settings.card(&ui.loadout_for).weapons;
                        let gun = loadout.mount(slot).gun_at(i as usize).to_string();
                        settings.guns.iter().position(|g| g.name == gun)
                    }
                    Dropdown::Unit(kind) => Some(settings.units.index(kind)),
                    Dropdown::Convergence => {
                        let c = settings.card(&ui.loadout_for).weapons.convergence;
                        CONVERGENCE_CHOICES.iter().position(|v| *v == c)
                    }
                };
                scroll.dropdown = (index.unwrap_or(0) as f32 - 2.0).max(0.0) * ROW_STEP;
            }
        }
        MenuAction::PickCrosshair(c) => settings.crosshair = c,
        MenuAction::PickColor(c) => settings.crosshair_color = c,
        MenuAction::Rebind(a) => rebinding.0 = Some(a),
        MenuAction::ResetBindings => settings.bindings = default_bindings(),
        MenuAction::Quit => {
            exit.write(AppExit::Success);
        }
        MenuAction::EditPreset(name) => {
            let card = settings.card(&name).clone();
            ui.draft = Draft::from_card(&card, settings.units);
            ui.editing = card.name;
            ui.message.clear();
        }
        MenuAction::NewPreset => {
            let mut card = settings.card(&ui.editing).clone();
            card.name = unique_name(&settings, "New aircraft");
            ui.draft = Draft::from_card(&card, settings.units);
            ui.editing = card.name.clone();
            ui.message = "Fill in the stat card, then Save preset.".into();
        }
        MenuAction::DeletePreset => {
            let name = ui.editing.clone();
            if !settings.aircraft.iter().any(|c| c.name == name) {
                // A new preset that was never saved: just drop the draft.
                let card = settings.aircraft[0].clone();
                ui.draft = Draft::from_card(&card, settings.units);
                ui.editing = card.name;
                ui.message = format!("Discarded \"{name}\".");
            } else {
                match settings.delete_card(&name) {
                    Ok(()) => {
                        let card = settings.aircraft[0].clone();
                        ui.draft = Draft::from_card(&card, settings.units);
                        ui.editing = card.name;
                        ui.message = format!("Deleted \"{name}\".");
                    }
                    Err(e) => ui.message = e,
                }
            }
        }
        MenuAction::ToggleEngine => {
            let ui = ui.as_mut();
            ui.draft.engine = Some(match ui.draft.engine.unwrap_or(EngineKind::Prop) {
                EngineKind::Prop => EngineKind::Jet,
                EngineKind::Jet => EngineKind::Prop,
            });
        }
        MenuAction::ToggleAfterburner => {
            let ui = ui.as_mut();
            ui.draft.afterburner = Some(!ui.draft.afterburner.unwrap_or(false));
        }
        MenuAction::SavePreset => {
            let base = settings.card(&ui.editing).clone();
            match ui.draft.to_card(&base, settings.units) {
                Ok(mut card) => {
                    // The built-in Yak-3 is never overwritten: save a copy.
                    let copied = is_builtin(&card.name);
                    if copied {
                        card.name = unique_name(&settings, &format!("{} copy", card.name.trim()));
                    }
                    let name = card.name.clone();
                    let replaced = settings.aircraft.iter().any(|c| c.name == name);
                    let old = ui.editing.clone();
                    let kept_old = old != name && settings.aircraft.iter().any(|c| c.name == old);
                    match settings.save_card(card) {
                        Ok(()) => {
                            ui.message = if copied {
                                format!(
                                    "The Yak-3 is built in, so this was saved as \"{name}\". \
                                     Rename it and save again if you like."
                                )
                            } else if replaced {
                                format!("Saved \"{name}\".")
                            } else if kept_old {
                                format!("Saved new preset \"{name}\". \"{old}\" is still there.")
                            } else {
                                format!("Saved new preset \"{name}\".")
                            };
                            ui.draft.fields[Field::Name.index()] = name.clone();
                            ui.editing = name;
                        }
                        Err(e) => ui.message = e,
                    }
                }
                Err(e) => ui.message = e,
            }
        }
        MenuAction::PickPlayerAircraft(name) => settings.player_aircraft = name,
        MenuAction::PickTargetAircraft(name) => settings.target_aircraft = name,
        MenuAction::EditGun(name) => {
            let gun = settings.gun(&name).clone();
            ui.gun_draft = GunDraft::from_card(&gun);
            ui.gun_editing = gun.name;
            ui.gun_message.clear();
        }
        MenuAction::NewGun => {
            let mut gun = settings.gun(&ui.gun_editing).clone();
            gun.name = unique_gun_name(&settings, "New gun");
            ui.gun_draft = GunDraft::from_card(&gun);
            ui.gun_editing = gun.name;
            ui.gun_message = "Fill in the gun's numbers, then Save gun.".into();
        }
        MenuAction::DeleteGun => {
            let name = ui.gun_editing.clone();
            let saved = settings.guns.iter().any(|g| g.name == name);
            let result = if saved {
                settings.delete_gun(&name)
            } else {
                Ok(())
            };
            match result {
                Ok(()) => {
                    let gun = settings.guns[0].clone();
                    ui.gun_draft = GunDraft::from_card(&gun);
                    ui.gun_editing = gun.name;
                    ui.gun_message = if saved {
                        format!(
                            "Deleted \"{name}\". Mounts that used it now carry the {}.",
                            settings.guns[0].name
                        )
                    } else {
                        format!("Discarded \"{name}\".")
                    };
                }
                Err(e) => ui.gun_message = e,
            }
        }
        MenuAction::SaveGun => match ui.gun_draft.to_card() {
            Ok(mut gun) => {
                let copied = is_builtin_gun(&gun.name);
                if copied {
                    gun.name = unique_gun_name(&settings, &format!("{} copy", gun.name.trim()));
                }
                let name = gun.name.clone();
                let replaced = settings.guns.iter().any(|g| g.name == name);
                let old = ui.gun_editing.clone();
                let kept_old = old != name && settings.guns.iter().any(|g| g.name == old);
                match settings.save_gun(gun) {
                    Ok(()) => {
                        ui.gun_message = if copied {
                            format!(
                                "The ShVAK is built in, so this was saved as \"{name}\". \
                                 Rename it and save again if you like."
                            )
                        } else if replaced {
                            format!("Saved \"{name}\".")
                        } else if kept_old {
                            format!("Saved new gun \"{name}\". \"{old}\" is still there.")
                        } else {
                            format!("Saved new gun \"{name}\".")
                        };
                        ui.gun_draft.fields[GunField::Name.index()] = name.clone();
                        ui.gun_editing = name;
                    }
                    Err(e) => ui.gun_message = e,
                }
            }
            Err(e) => ui.gun_message = e,
        },
        MenuAction::PickTracer(c) => {
            ui.gun_draft.tracer = c;
            ui.gun_message = "Tracer color set. Save gun to keep it.".into();
        }
        MenuAction::PickLoadoutAircraft(name) => {
            ui.loadout_for = name;
            ui.gun_message.clear();
        }
        MenuAction::PickMountGun(slot, gun) => {
            edit_loadout(&mut settings, &mut ui, |l| l.mount_mut(slot).set_all(&gun));
        }
        MenuAction::PickStationGun(slot, i, gun) => {
            edit_loadout(&mut settings, &mut ui, |l| {
                l.mount_mut(slot).set_at(i as usize, &gun)
            });
        }
        MenuAction::MountCount(slot, delta) => {
            edit_loadout(&mut settings, &mut ui, |l| {
                let m = l.mount_mut(slot);
                m.count = (m.count as i32 + delta).clamp(0, slot.max() as i32) as u32;
            });
        }
        MenuAction::SetConvergence(m) => {
            edit_loadout(&mut settings, &mut ui, |l| l.convergence = m as f32);
        }
    }
}

/// Change the loadout of the aircraft shown on the weapons page. The
/// built-in Yak-3 isn't changed: a copy gets the new loadout, and if you
/// were flying the Yak-3 you now fly the copy.
fn edit_loadout(settings: &mut Settings, ui: &mut MenuUi, change: impl FnOnce(&mut Loadout)) {
    let name = ui.loadout_for.clone();
    let mut card = settings.card(&name).clone();
    let before = card.weapons.clone();
    change(&mut card.weapons);
    if card.weapons == before {
        return;
    }
    if is_builtin(&card.name) {
        card.name = unique_name(settings, &format!("{} copy", card.name));
        let flying = settings.player_aircraft == name;
        ui.gun_message = format!(
            "The Yak-3 is built in, so this loadout was saved on a copy, \"{}\"{}.",
            card.name,
            if flying { ", which you now fly" } else { "" }
        );
        if flying {
            settings.player_aircraft = card.name.clone();
        }
        ui.loadout_for = card.name.clone();
    }
    if let Err(e) = settings.save_card(card) {
        ui.gun_message = e;
    }
}

fn unique_gun_name(settings: &Settings, base: &str) -> String {
    let taken = |n: &str| settings.guns.iter().any(|g| g.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .unwrap()
}

fn unique_name(settings: &Settings, base: &str) -> String {
    let taken = |n: &str| settings.aircraft.iter().any(|c| c.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .unwrap()
}

/// Steps of 0.05 below 1x and 0.1 above, so fine tuning is easy.
fn step_sensitivity(current: f32, dir: f32) -> f32 {
    let step = if current < 1.0 || (current == 1.0 && dir < 0.0) {
        0.05
    } else {
        0.1
    };
    let v = ((current + dir * step) / 0.05).round() * 0.05;
    v.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY)
}

#[allow(clippy::type_complexity)]
fn style_buttons(
    mut buttons: Query<(&Hovered, Has<Pressed>, Has<Selected>, &mut BackgroundColor), With<Button>>,
) {
    for (hovered, pressed, selected, mut bg) in &mut buttons {
        let color = if selected && !pressed {
            BUTTON_SELECTED
        } else {
            button_color(hovered.get(), pressed)
        };
        if bg.0 != color {
            bg.0 = color;
        }
    }
}

fn button_color(hovered: bool, pressed: bool) -> Color {
    match (hovered, pressed) {
        (_, true) => BUTTON_PRESS,
        (true, false) => BUTTON_HOVER,
        (false, false) => BUTTON_BG,
    }
}

fn label(text: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn section_header(text: &str) -> impl Bundle {
    (
        label(text, 14.0, DIM),
        Node {
            margin: UiRect::top(px(6)),
            ..default()
        },
    )
}

fn row() -> impl Bundle {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: px(8),
        ..default()
    }
}

fn button(action: MenuAction, text: &str, width: Val) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        action,
        Node {
            width,
            height: px(28),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(BUTTON_BG),
        children![label(text, 16.0, TEXT)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_card_round_trips_through_the_editor() {
        let yak = AircraftCard::yak3();
        for units in [Units::METRIC, Units::IMPERIAL, Units::NAUTICAL] {
            let back = Draft::from_card(&yak, units).to_card(&yak, units).unwrap();
            assert!((back.top_speed - yak.top_speed).abs() < 0.3, "{units:?}");
            assert!((back.climb_rate - yak.climb_rate).abs() < 0.05, "{units:?}");
            assert!((back.power - yak.power).abs() < 0.1, "{units:?}");
            assert!(
                (back.mass.unwrap() - yak.mass.unwrap()).abs() < 0.6,
                "{units:?}"
            );
            assert!(
                (back.wing_area.unwrap() - yak.wing_area.unwrap()).abs() < 0.01,
                "{units:?}"
            );
            assert_eq!(back.engines, 1);
        }
    }

    #[test]
    fn editor_reads_war_thunder_style_numbers() {
        let yak = AircraftCard::yak3();
        let mut d = Draft::from_card(&yak, Units::NAUTICAL);
        d.fields[Field::Climb.index()] = "4,193".into();
        d.fields[Field::TopSpeed.index()] = "352".into();
        let card = d.to_card(&yak, Units::NAUTICAL).unwrap();
        assert!((card.climb_rate - 21.30).abs() < 0.01);
        assert!((card.top_speed - 181.08).abs() < 0.05);
        d.fields[Field::Power.index()] = "lots".into();
        assert!(d.to_card(&yak, Units::NAUTICAL).is_err());
    }

    #[test]
    fn sensitivity_steps_and_clamps() {
        assert!((step_sensitivity(1.0, 1.0) - 1.1).abs() < 1e-4);
        assert!((step_sensitivity(1.0, -1.0) - 0.95).abs() < 1e-4);
        assert!((step_sensitivity(0.1, -1.0) - 0.1).abs() < 1e-4);
        assert!((step_sensitivity(5.0, 1.0) - 5.0).abs() < 1e-4);
    }
}
