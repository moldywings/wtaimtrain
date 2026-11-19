//! Player plane. A simple "instructor" turns the plane toward the mouse aim
//! point, the way War Thunder's mouse aim does: roll toward the aim point,
//! pull, then level the wings when the nose is on target.

use bevy::prelude::*;

use crate::camera::MouseAim;

pub struct FlightPlugin;

impl Plugin for FlightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player)
            .add_systems(Update, (fly_player, reset_player));
    }
}

#[derive(Component)]
pub struct Player {
    /// Current airspeed, m/s.
    pub speed: f32,
    /// Throttle, 0..=1.
    pub throttle: f32,
}

/// Handling limits for the player plane. Rough fighter-like numbers.
const MAX_PITCH_RATE: f32 = 1.1; // rad/s, nose up
const MAX_PUSH_RATE: f32 = 0.4; // rad/s, nose down
const MAX_YAW_RATE: f32 = 0.25; // rad/s
const MAX_ROLL_RATE: f32 = 3.5; // rad/s
const MIN_SPEED: f32 = 60.0;
const MAX_LEVEL_SPEED: f32 = 170.0; // m/s at full throttle, level flight
const START_ALTITUDE: f32 = 1_500.0;

fn start_transform() -> Transform {
    Transform::from_xyz(0.0, START_ALTITUDE, 0.0)
}

fn spawn_player(mut commands: Commands, assets: Res<AssetServer>) {
    commands
        .spawn((
            start_transform(),
            Visibility::default(),
            Player {
                speed: 140.0,
                throttle: 0.8,
            },
        ))
        .with_children(|plane| {
            plane.spawn((
                WorldAssetRoot(
                    assets.load(GltfAssetLabel::Scene(0).from_asset(crate::AIRPLANE_MODEL)),
                ),
                crate::model_correction(),
            ));
        });
}

pub fn fly_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    aim: Res<MouseAim>,
    player: Single<(&mut Transform, &mut Player)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let (mut tf, mut plane) = player.into_inner();

    // Throttle: W / S (or Shift / Ctrl).
    let mut throttle_input = 0.0;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ShiftLeft]) {
        throttle_input += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ControlLeft]) {
        throttle_input -= 1.0;
    }
    plane.throttle = (plane.throttle + throttle_input * 0.5 * dt).clamp(0.0, 1.0);

    // Aim direction in the plane's local frame (forward = -Z, up = +Y).
    let local = tf.rotation.inverse() * aim.direction();
    let off_nose = local.angle_between(Vec3::NEG_Z);

    // Pitch toward the aim point, measured in the plane's pitch plane.
    let pitch_err = local.y.atan2(-local.z);
    let pitch_rate = (pitch_err * 4.0).clamp(-MAX_PUSH_RATE, MAX_PITCH_RATE);

    // A little rudder for fine aiming.
    let yaw_err = local.x.atan2(-local.z);
    let yaw_rate = (-yaw_err * 3.0).clamp(-MAX_YAW_RATE, MAX_YAW_RATE);

    // Roll: far off the nose, bank so the aim point sits "above" the canopy.
    // Close to the nose, roll the wings level instead.
    // Aim point well below the nose means rolling inverted and pulling.
    let bank_to_target = local.x.atan2(local.y);
    let right = tf.rotation * Vec3::X;
    let bank_to_level = right.y.clamp(-1.0, 1.0).asin();
    let w = smoothstep(0.03, 0.15, off_nose);
    let roll_err = w * bank_to_target + (1.0 - w) * bank_to_level;
    let roll_rate = (-roll_err * 5.0).clamp(-MAX_ROLL_RATE, MAX_ROLL_RATE);

    // Pull less while still rolling into the turn, like a real pilot.
    let pull_scale = 1.0 - (w * bank_to_target.abs() / 1.5).clamp(0.0, 0.8);

    let delta = Quat::from_rotation_x(pitch_rate * pull_scale * dt)
        * Quat::from_rotation_y(yaw_rate * dt)
        * Quat::from_rotation_z(roll_rate * dt);
    tf.rotation = (tf.rotation * delta).normalize();

    // Energy: gravity along the flight path plus thrust pulling toward the
    // throttle's level speed. Climbing bleeds speed, diving adds it.
    let forward = tf.rotation * Vec3::NEG_Z;
    let target_speed = MAX_LEVEL_SPEED * (0.35 + 0.65 * plane.throttle);
    let accel = -9.81 * forward.y + (target_speed - plane.speed) * 0.15;
    plane.speed = (plane.speed + accel * dt).max(MIN_SPEED);

    tf.translation += forward * plane.speed * dt;

    // No crashes yet: bounce off a floor.
    if tf.translation.y < 50.0 {
        tf.translation.y = 50.0;
    }
}

/// R resets the player to the start position.
fn reset_player(
    keys: Res<ButtonInput<KeyCode>>,
    mut aim: ResMut<MouseAim>,
    player: Single<(&mut Transform, &mut Player)>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        let (mut tf, mut plane) = player.into_inner();
        *tf = start_transform();
        plane.speed = 140.0;
        aim.yaw = 0.0;
        aim.pitch = 0.0;
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
