//! War Thunder style aim trainer (Air Realistic Battles, mouse aim).
//!
//! Step 1 prototype: fly a plane with mouse aim, a chase camera, an aim
//! circle and nose marker, a reference world, and a target aircraft.

mod camera;
mod flight;
mod hud;
mod target;
mod world;

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

/// Path of the shared aircraft model, relative to `assets/`.
pub const AIRPLANE_MODEL: &str = "models/airplane.glb";

/// The model is about 0.75x a real Yak-3 (8.5 m long, 9.2 m span).
/// Scale it up so ranges and lead angles look right.
pub const MODEL_SCALE: f32 = 1.33;

/// The model's nose points +Z; Bevy treats -Z as forward.
pub fn model_correction() -> Transform {
    Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI))
        .with_scale(Vec3::splat(MODEL_SCALE))
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
        .add_plugins((
            world::WorldPlugin,
            flight::FlightPlugin,
            camera::CameraPlugin,
            target::TargetPlugin,
            hud::HudPlugin,
        ))
        .add_systems(Update, (grab_cursor, debug_screenshot))
        .run();
}

/// Click to capture the mouse, Escape to release it.
fn grab_cursor(
    mut cursor: Single<&mut CursorOptions>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

/// Testing aid: with `WTAT_SCREENSHOT=out.png`, save a screenshot after a few
/// seconds of flight and quit. Does nothing otherwise.
fn debug_screenshot(
    mut commands: Commands,
    time: Res<Time>,
    mut taken: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(path) = std::env::var("WTAT_SCREENSHOT") else {
        return;
    };
    let t = time.elapsed_secs();
    if !*taken && t > 6.0 {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
        *taken = true;
    }
    if t > 9.0 {
        exit.write(AppExit::Success);
    }
}
