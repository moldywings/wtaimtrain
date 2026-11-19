//! Pause menu (Esc): resume, units, mouse sensitivity, key bindings, quit.
//! The menu is rebuilt whenever a setting changes, which keeps it simple.

use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};

use crate::GameState;
use crate::settings::{
    Action, MAX_SENSITIVITY, MIN_SENSITIVITY, Settings, default_bindings, key_name,
};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rebinding>()
            .add_observer(on_menu_click)
            .add_systems(OnExit(GameState::Paused), (despawn_menu, cancel_rebind))
            .add_systems(
                Update,
                (
                    toggle_pause,
                    capture_rebind
                        .after(toggle_pause)
                        .run_if(in_state(GameState::Paused)),
                    rebuild_menu
                        .after(capture_rebind)
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

#[derive(Component, Clone, Copy)]
enum MenuAction {
    Resume,
    CycleUnits,
    SensitivityDown,
    SensitivityUp,
    Rebind(Action),
    ResetBindings,
    Quit,
}

const PANEL_BG: Color = Color::srgba(0.08, 0.09, 0.11, 0.94);
const BUTTON_BG: Color = Color::srgb(0.18, 0.20, 0.24);
const BUTTON_HOVER: Color = Color::srgb(0.27, 0.30, 0.36);
const BUTTON_PRESS: Color = Color::srgb(0.35, 0.55, 0.40);
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

fn rebuild_menu(
    mut commands: Commands,
    settings: Res<Settings>,
    rebinding: Res<Rebinding>,
    menus: Query<Entity, With<PauseMenu>>,
) {
    let exists = !menus.is_empty();
    if exists && !settings.is_changed() && !rebinding.is_changed() {
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
                panel.spawn(label("Paused", 28.0, TEXT));
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

                            col.spawn(section_header("Mouse"));
                            col.spawn(label(
                                "Move: aim\nRight click: toggle zoom\nLeft click: shoot (step 2)",
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
                                        label(
                                            action.label(),
                                            16.0,
                                            if waiting { ACCENT } else { TEXT },
                                        ),
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
            });
        });
}

fn column(width: Val) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        width,
        row_gap: px(6),
        ..default()
    }
}

fn on_menu_click(
    activate: On<Activate>,
    actions: Query<&MenuAction>,
    mut settings: ResMut<Settings>,
    mut rebinding: ResMut<Rebinding>,
    mut next: ResMut<NextState<GameState>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(action) = actions.get(activate.entity) else {
        return;
    };
    match *action {
        MenuAction::Resume => next.set(GameState::Playing),
        MenuAction::CycleUnits => settings.units = settings.units.next(),
        MenuAction::SensitivityDown => {
            settings.sensitivity = step_sensitivity(settings.sensitivity, -1.0);
        }
        MenuAction::SensitivityUp => {
            settings.sensitivity = step_sensitivity(settings.sensitivity, 1.0);
        }
        MenuAction::Rebind(a) => rebinding.0 = Some(a),
        MenuAction::ResetBindings => settings.bindings = default_bindings(),
        MenuAction::Quit => {
            exit.write(AppExit::Success);
        }
    }
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

fn style_buttons(mut buttons: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<Button>>) {
    for (hovered, pressed, mut bg) in &mut buttons {
        let color = button_color(hovered.get(), pressed);
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
            height: px(30),
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
    use super::step_sensitivity;

    #[test]
    fn sensitivity_steps_and_clamps() {
        assert!((step_sensitivity(1.0, 1.0) - 1.1).abs() < 1e-4);
        assert!((step_sensitivity(1.0, -1.0) - 0.95).abs() < 1e-4);
        assert!((step_sensitivity(0.1, -1.0) - 0.1).abs() < 1e-4);
        assert!((step_sensitivity(5.0, 1.0) - 5.0).abs() < 1e-4);
    }
}
