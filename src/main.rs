//! War Thunder style aim trainer (Air Realistic Battles, mouse aim).
//!
//! Step 1 prototype: fly a plane with mouse aim, a chase camera, an aim
//! circle and nose marker, a reference world, a target aircraft, and a
//! pause menu with settings.

// No console window behind the game in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod camera;
mod flight;
mod flight_model;
mod hud;
mod menu;
mod multiplayer;
mod names;
mod settings;
mod sound;
mod target;
mod weapons;
mod world;

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

/// Path of the shared aircraft model, relative to `assets/`.
pub const AIRPLANE_MODEL: &str = "models/airplane.glb";

/// The model is about 0.75x a real Yak-3 (8.5 m long, 9.2 m span).
/// Scale it up so ranges and lead angles look right.
pub const MODEL_SCALE: f32 = 1.33;

/// The model's nose points +Z; Bevy treats -Z as forward.
/// Height in the model of the fuselage's center line, which the plane
/// rolls about (the model's origin is at the bottom of the fuselage).
pub const MODEL_ROLL_AXIS_Y: f32 = 0.6;

pub fn model_correction() -> Transform {
    Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI))
        .with_scale(Vec3::splat(MODEL_SCALE))
        .with_translation(Vec3::new(0.0, -MODEL_ROLL_AXIS_Y * MODEL_SCALE, 0.0))
}

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameState {
    #[default]
    Playing,
    Paused,
}

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.53, 0.70, 0.88)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "WT Aim Trainer".into(),
                ..default()
            }),
            ..default()
        }))
        .init_state::<GameState>()
        .add_plugins((
            settings::SettingsPlugin,
            world::WorldPlugin,
            flight::FlightPlugin,
            camera::CameraPlugin,
            target::TargetPlugin,
            hud::HudPlugin,
            menu::MenuPlugin,
            weapons::WeaponsPlugin,
            sound::SoundPlugin,
            multiplayer::MultiplayerPlugin,
        ))
        .add_systems(OnEnter(GameState::Playing), (resume_time, capture_cursor))
        .add_systems(OnEnter(GameState::Paused), release_cursor)
        .add_systems(
            Update,
            (
                click_to_capture.run_if(in_state(GameState::Playing)),
                pause_time,
                debug_screenshot,
            ),
        )
        .run();
}

fn capture_cursor(mut cursor: Single<&mut CursorOptions>) {
    // Automated screenshot runs drive the window with synthetic input; a
    // pointer grab would swallow it.
    if std::env::var("WTAT_SCREENSHOT").is_ok() {
        return;
    }
    cursor.visible = false;
    cursor.grab_mode = CursorGrabMode::Locked;
}

fn release_cursor(mut cursor: Single<&mut CursorOptions>) {
    cursor.visible = true;
    cursor.grab_mode = CursorGrabMode::None;
}

/// If the mouse got released while flying (alt-tab, etc.), a click takes it back.
fn click_to_capture(cursor: Single<&mut CursorOptions>, mouse: Res<ButtonInput<MouseButton>>) {
    if mouse.just_pressed(MouseButton::Left) {
        capture_cursor(cursor);
    }
}

/// The menu pauses the game, except in multiplayer, where the world goes on
/// for everyone else.
fn pause_time(
    mut time: ResMut<Time<Virtual>>,
    state: Res<State<GameState>>,
    mp: Res<multiplayer::Mp>,
) {
    let pause = *state.get() == GameState::Paused && !mp.online();
    if pause != time.is_paused() {
        if pause {
            time.pause();
        } else {
            time.unpause();
        }
    }
}

fn resume_time(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

/// Testing aid: with `WTAT_SCREENSHOT=out.png`, save a screenshot after a few
/// seconds (`WTAT_SHOT_AT`, default 6) and quit. `WTAT_PAUSE=1` opens the
/// pause menu first. Does nothing
/// otherwise.
fn debug_screenshot(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut taken: Local<bool>,
    mut paused: Local<bool>,
    mut next_state: ResMut<NextState<GameState>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(path) = std::env::var("WTAT_SCREENSHOT") else {
        return;
    };
    let t = time.elapsed_secs();
    let shot_at: f32 = std::env::var("WTAT_SHOT_AT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6.0);
    if !*paused && t > 4.0 && std::env::var("WTAT_PAUSE").is_ok() {
        next_state.set(GameState::Paused);
        *paused = true;
    }
    if !*taken && t > shot_at {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
        *taken = true;
    }
    if t > shot_at + 3.0 {
        exit.write(AppExit::Success);
    }
}
