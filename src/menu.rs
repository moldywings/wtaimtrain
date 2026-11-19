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
use bevy::ui_widgets::{Activate, Button, SelectAllOnFocus, TextInput};

use crate::GameState;
use crate::flight_model::{AircraftCard, EngineKind, FlightModel};
use crate::settings::{
    Action, Crosshair, CrosshairColor, MAX_SENSITIVITY, MIN_SENSITIVITY, Settings, Units,
    default_bindings, is_builtin, key_name,
};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::input_focus::tab_navigation::TabNavigationPlugin)
            .init_resource::<Rebinding>()
            .init_resource::<MenuUi>()
            .init_resource::<ScrollMemory>()
            .add_observer(on_menu_click)
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
    Aircraft,
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dropdown {
    Crosshair,
    Color,
}

/// Scrollable lists, so their scroll position survives a menu rebuild.
#[derive(Component, Clone, Copy)]
enum ScrollList {
    Presets,
    Dropdown,
}

/// Remembered scroll offsets, kept outside `MenuUi` so scrolling doesn't
/// rebuild the menu.
#[derive(Resource, Default)]
struct ScrollMemory {
    presets: f32,
    dropdown: f32,
}

impl ScrollMemory {
    fn get_mut(&mut self, list: ScrollList) -> &mut f32 {
        match list {
            ScrollList::Presets => &mut self.presets,
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
    fields: [String; Field::ALL.len()],
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Power,
    Engines,
    Climb,
    TurnTime,
    TopSpeed,
    TopSpeedAltitude,
    Mass,
    WingArea,
    ClMax,
    RollRate,
}

impl Field {
    const ALL: [Field; 11] = [
        Field::Name,
        Field::Power,
        Field::Engines,
        Field::Climb,
        Field::TurnTime,
        Field::TopSpeed,
        Field::TopSpeedAltitude,
        Field::Mass,
        Field::WingArea,
        Field::ClMax,
        Field::RollRate,
    ];

    fn index(self) -> usize {
        Field::ALL.iter().position(|f| *f == self).unwrap()
    }

    /// Label and unit shown in the editor.
    fn label(self, units: Units, engine: EngineKind) -> (&'static str, &'static str) {
        match self {
            Field::Name => ("Name", ""),
            Field::Power => match engine {
                EngineKind::Prop => ("Power per engine", "hp"),
                EngineKind::Jet => ("Thrust per engine", "kgf"),
            },
            Field::Engines => ("Engines", ""),
            Field::Climb => ("Climb rate", units.climb_unit().0),
            Field::TurnTime => ("Sustained turn time", "s"),
            Field::TopSpeed => ("Top speed", units.speed_unit().0),
            Field::TopSpeedAltitude => ("  at altitude (optional)", units.altitude_unit().0),
            Field::Mass => ("Mass (optional)", "kg"),
            Field::WingArea => ("Wing area (optional)", "sq m"),
            Field::ClMax => ("Max lift coeff. (optional)", ""),
            Field::RollRate => ("Roll rate (optional)", "deg/s"),
        }
    }
}

impl Draft {
    fn from_card(card: &AircraftCard, units: Units) -> Draft {
        let (_, spd) = units.speed_unit();
        let (_, clm) = units.climb_unit();
        let (_, alt) = units.altitude_unit();
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
                Field::Power => n(card.power, 0),
                Field::Engines => card.engines.to_string(),
                Field::Climb => n(card.climb_rate * clm, 1),
                Field::TurnTime => n(card.turn_time, 1),
                Field::TopSpeed => n(card.top_speed * spd, 0),
                Field::TopSpeedAltitude => opt(card.top_speed_altitude.map(|v| v * alt), 0),
                Field::Mass => opt(card.mass, 0),
                Field::WingArea => opt(card.wing_area, 2),
                Field::ClMax => opt(card.cl_max, 2),
                Field::RollRate => opt(card.roll_rate, 0),
            };
        }
        Draft {
            engine: Some(card.engine),
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
            engine: self.engine.unwrap_or(base.engine),
            power: num(Field::Power, true, base.power, 1.0)?,
            engines: num(Field::Engines, true, base.engines as f32, 1.0)?
                .round()
                .max(1.0) as u32,
            climb_rate: num(Field::Climb, true, base.climb_rate, clm)?,
            turn_time: num(Field::TurnTime, true, base.turn_time, 1.0)?,
            top_speed: num(Field::TopSpeed, true, base.top_speed, spd)?,
            top_speed_altitude: optional(Field::TopSpeedAltitude, alt)?,
            mass: optional(Field::Mass, 1.0)?,
            wing_area: optional(Field::WingArea, 1.0)?,
            cl_max: optional(Field::ClMax, 1.0)?,
            roll_rate: optional(Field::RollRate, 1.0)?,
        };
        if card.power <= 0.0 || card.climb_rate <= 0.0 || card.top_speed <= 0.0 {
            return Err("Power, climb rate and top speed must be above zero.".into());
        }
        if card.turn_time < 5.0 {
            return Err("Turn time looks too short (under 5 s).".into());
        }
        Ok(card)
    }
}

#[derive(Component, Clone)]
enum MenuAction {
    Resume,
    Page(Page),
    CycleUnits,
    SensitivityDown,
    SensitivityUp,
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
    SavePreset,
    CyclePlayerAircraft,
    CycleTargetAircraft,
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
    // Testing aid: WTAT_PAGE=aircraft opens straight onto the aircraft page.
    if std::env::var("WTAT_PAGE").is_ok_and(|p| p == "aircraft") {
        ui.page = Page::Aircraft;
    }
}

/// Copy what's typed into the draft, without triggering a rebuild.
fn sync_draft(mut ui: ResMut<MenuUi>, inputs: Query<(&Field, &EditableText)>) {
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
                    for (page, name) in [(Page::Settings, "Settings"), (Page::Aircraft, "Aircraft")]
                    {
                        let mut b = r.spawn(button(MenuAction::Page(page), name, px(120)));
                        if ui.page == page {
                            b.insert(Selected);
                        }
                    }
                });
                match ui.page {
                    Page::Settings => settings_page(panel, &settings, &rebinding, &ui, &scroll),
                    Page::Aircraft => aircraft_page(panel, &settings, &ui, &scroll),
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

                col.spawn(section_header("Units"));
                col.spawn(button(
                    MenuAction::CycleUnits,
                    settings.units.label(),
                    percent(100),
                ));

                col.spawn(section_header("Mouse sensitivity"));
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

                col.spawn(section_header("Crosshair"));
                let styles: Vec<_> = Crosshair::ALL
                    .iter()
                    .map(|c| Choice {
                        action: MenuAction::PickCrosshair(*c),
                        label: c.label(),
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
                );

                col.spawn(section_header("Crosshair color"));
                let colors: Vec<_> = CrosshairColor::ALL
                    .iter()
                    .map(|c| Choice {
                        action: MenuAction::PickColor(*c),
                        label: c.label(),
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
                );

                col.spawn(section_header("Mouse"));
                col.spawn(label(
                    "Move: aim\nRight click: toggle zoom\nLeft click: shoot (coming soon)",
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
                col.spawn(section_header("You fly"));
                col.spawn(button(
                    MenuAction::CyclePlayerAircraft,
                    &settings.player_aircraft,
                    percent(100),
                ));
                col.spawn(section_header("Target flies"));
                col.spawn(button(
                    MenuAction::CycleTargetAircraft,
                    &settings.target_aircraft,
                    percent(100),
                ));

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
                for field in Field::ALL {
                    let (name, unit) = field.label(units, engine);
                    col.spawn(row()).with_children(|r| {
                        r.spawn((label(name, 15.0, TEXT), grow()));
                        r.spawn(text_field(field, ui.draft.get(field)));
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
                             Climb {:.0} {cu}. Turn {:.1} s. Stalls at {:.0} {su}.",
                            card.name,
                            p.top_speed * sf,
                            card.extras().top_speed_altitude * af,
                            p.top_speed_sea_level * sf,
                            p.climb_rate * cf,
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
                        filled.push(format!("mass {:.0} kg", e.mass));
                    }
                    if card.wing_area.is_none() {
                        filled.push(format!("wing area {:.1} sq m", e.wing_area));
                    }
                    if card.cl_max.is_none() {
                        filled.push(format!("max lift {:.2}", e.cl_max));
                    }
                    if card.roll_rate.is_none() {
                        filled.push(format!("roll rate {:.0} deg/s", e.roll_rate));
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
                    if (p.turn_time - card.turn_time).abs() > card.turn_time * 0.05 {
                        misses.push("turn time");
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

fn grow() -> Node {
    Node {
        flex_grow: 1.0,
        ..default()
    }
}

fn text_field(field: Field, value: &str) -> impl Bundle {
    (
        Node {
            width: px(150),
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
        TabIndex(field.index() as i32),
        SelectAllOnFocus,
        field,
    )
}

/// One entry in a drop-down list.
struct Choice {
    action: MenuAction,
    label: &'static str,
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
) {
    let open = ui.open == Some(kind);
    col.spawn(Node {
        width: percent(100),
        ..default()
    })
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
            b.spawn((label(current, 16.0, TEXT), grow()));
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
            GlobalZIndex(20),
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
                    b.spawn(label(choice.label, 16.0, TEXT));
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
        MenuAction::CycleUnits => {
            settings.units = settings.units.next();
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
        MenuAction::CyclePlayerAircraft => {
            settings.player_aircraft = next_name(&settings, &settings.player_aircraft);
        }
        MenuAction::CycleTargetAircraft => {
            settings.target_aircraft = next_name(&settings, &settings.target_aircraft);
        }
    }
}

fn next_name(settings: &Settings, current: &str) -> String {
    let names: Vec<&String> = settings.aircraft.iter().map(|c| &c.name).collect();
    let i = names.iter().position(|n| *n == current).unwrap_or(0);
    names[(i + 1) % names.len()].clone()
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
        for units in [Units::Metric, Units::Imperial, Units::Nautical] {
            let back = Draft::from_card(&yak, units).to_card(&yak, units).unwrap();
            assert!((back.top_speed - yak.top_speed).abs() < 0.3, "{units:?}");
            assert!((back.climb_rate - yak.climb_rate).abs() < 0.05, "{units:?}");
            assert_eq!(back.engines, 1);
        }
    }

    #[test]
    fn editor_reads_war_thunder_style_numbers() {
        let yak = AircraftCard::yak3();
        let mut d = Draft::from_card(&yak, Units::Nautical);
        d.fields[Field::Climb.index()] = "4,193".into();
        d.fields[Field::TopSpeed.index()] = "352".into();
        let card = d.to_card(&yak, Units::Nautical).unwrap();
        assert!((card.climb_rate - 21.30).abs() < 0.01);
        assert!((card.top_speed - 181.08).abs() < 0.05);
        d.fields[Field::Power.index()] = "lots".into();
        assert!(d.to_card(&yak, Units::Nautical).is_err());
    }

    #[test]
    fn sensitivity_steps_and_clamps() {
        assert!((step_sensitivity(1.0, 1.0) - 1.1).abs() < 1e-4);
        assert!((step_sensitivity(1.0, -1.0) - 0.95).abs() < 1e-4);
        assert!((step_sensitivity(0.1, -1.0) - 0.1).abs() < 1e-4);
        assert!((step_sensitivity(5.0, 1.0) - 5.0).abs() < 1e-4);
    }
}
