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
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, TextCursorStyle};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button, TextInput};

use crate::GameState;
use crate::flight_model::{AircraftCard, EngineKind, FlightModel};
use crate::settings::{
    Action, MAX_SENSITIVITY, MIN_SENSITIVITY, Settings, Units, default_bindings, key_name,
};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rebinding>()
            .init_resource::<MenuUi>()
            .add_observer(on_menu_click)
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
}

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
        let mut fields: [String; Field::ALL.len()] = Default::default();
        for f in Field::ALL {
            fields[f.index()] = match f {
                Field::Name => card.name.clone(),
                Field::Power => n(card.power, 0),
                Field::Engines => card.engines.to_string(),
                Field::Climb => n(card.climb_rate * clm, 1),
                Field::TurnTime => n(card.turn_time, 1),
                Field::TopSpeed => n(card.top_speed * spd, 0),
                Field::TopSpeedAltitude => n(card.top_speed_altitude * alt, 0),
                Field::Mass => n(card.mass, 0),
                Field::WingArea => n(card.wing_area, 2),
                Field::ClMax => n(card.cl_max, 2),
                Field::RollRate => n(card.roll_rate, 0),
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

    /// Build a card from the fields. Blank optional fields keep the base
    /// card's value (the preset being edited).
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
            top_speed_altitude: num(Field::TopSpeedAltitude, false, base.top_speed_altitude, alt)?,
            mass: num(Field::Mass, false, base.mass, 1.0)?,
            wing_area: num(Field::WingArea, false, base.wing_area, 1.0)?,
            cl_max: num(Field::ClMax, false, base.cl_max, 1.0)?,
            roll_rate: num(Field::RollRate, false, base.roll_rate, 1.0)?,
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
    CycleCrosshair,
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
                    Page::Settings => settings_page(panel, &settings, &rebinding),
                    Page::Aircraft => aircraft_page(panel, &settings, &ui),
                }
            });
        });
}

fn settings_page(panel: &mut ChildSpawnerCommands, settings: &Settings, rebinding: &Rebinding) {
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
                col.spawn(button(
                    MenuAction::CycleCrosshair,
                    settings.crosshair.label(),
                    percent(100),
                ));

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

fn aircraft_page(panel: &mut ChildSpawnerCommands, settings: &Settings, ui: &MenuUi) {
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

                col.spawn(section_header("Presets (click to edit)"));
                for card in &settings.aircraft {
                    let mut b = col.spawn(button(
                        MenuAction::EditPreset(card.name.clone()),
                        &card.name,
                        percent(100),
                    ));
                    if card.name == ui.editing {
                        b.insert(Selected);
                    }
                }
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
            cols.spawn(column(px(460))).with_children(|col| {
                col.spawn(section_header(
                    "Stat card: enter what you know, the rest is worked out",
                ));
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
                            card.top_speed_altitude * af,
                            p.top_speed_sea_level * sf,
                            p.climb_rate * cf,
                            p.turn_time,
                            p.stall_speed * sf,
                        ),
                        13.0,
                        DIM,
                    ));
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
        field,
    )
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

fn on_menu_click(
    activate: On<Activate>,
    actions: Query<&MenuAction>,
    mut settings: ResMut<Settings>,
    mut rebinding: ResMut<Rebinding>,
    mut ui: ResMut<MenuUi>,
    mut next: ResMut<NextState<GameState>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(action) = actions.get(activate.entity) else {
        return;
    };
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
        MenuAction::CycleCrosshair => settings.crosshair = settings.crosshair.next(),
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
            if settings.aircraft.len() <= 1 {
                ui.message = "You need at least one preset.".into();
            } else {
                let name = ui.editing.clone();
                settings.delete_card(&name);
                let card = settings.aircraft[0].clone();
                ui.draft = Draft::from_card(&card, settings.units);
                ui.editing = card.name;
                ui.message = format!("Deleted \"{name}\".");
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
                Ok(card) => {
                    let name = card.name.clone();
                    // Renaming a saved preset carries over who was flying it.
                    if ui.editing != name && settings.aircraft.iter().any(|c| c.name == ui.editing)
                    {
                        if settings.player_aircraft == ui.editing {
                            settings.player_aircraft = name.clone();
                        }
                        if settings.target_aircraft == ui.editing {
                            settings.target_aircraft = name.clone();
                        }
                        let old = ui.editing.clone();
                        settings.aircraft.retain(|c| c.name != old);
                    }
                    settings.save_card(card);
                    ui.editing = name.clone();
                    ui.message = format!("Saved \"{name}\".");
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
