//! Mouse aim: the mouse moves a free aim direction (the circle on screen),
//! the camera swings after it with a short lag, and the plane chases it
//! (see `flight`). Like War Thunder, a quick flick moves the circle off
//! center before the view catches up. Holding the free look key turns the
//! camera without moving the aim; the right mouse button toggles zoom.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

use crate::GameState;
use crate::flight::Player;
use crate::settings::{Action, Settings};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MouseAim>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    update_mouse_aim
                        .before(crate::flight::fly_player)
                        .run_if(in_state(GameState::Playing)),
                    follow_player.after(crate::flight::fly_player),
                ),
            );
    }
}

/// Where the player wants to point, in world space: a full orientation, so
/// the aim can go straight up and over the top. The mouse moves it across
/// the screen, and it slowly rolls back level whenever it isn't pointing
/// steeply up or down.
#[derive(Resource)]
pub struct MouseAim {
    pub rot: Quat,
}

impl Default for MouseAim {
    fn default() -> Self {
        MouseAim {
            rot: Quat::IDENTITY,
        }
    }
}

/// How quickly the aim (and the view) rolls back level, 1/s.
const LEVEL_RATE: f32 = 1.5;

impl MouseAim {
    pub fn rotation(&self) -> Quat {
        self.rot
    }

    pub fn reset(&mut self) {
        self.rot = Quat::IDENTITY;
    }

    /// Move the aim by a screen-space turn (rad): right and up.
    fn turn(&mut self, right: f32, up: f32) {
        self.rot =
            (self.rot * Quat::from_rotation_y(-right) * Quat::from_rotation_x(up)).normalize();
    }

    /// Roll part of the way back toward level.
    fn level(&mut self, dt: f32) {
        let f = self.rot * Vec3::NEG_Z;
        let up = self.rot * Vec3::Y;
        let level_up = (Vec3::Y - f * f.y).normalize_or_zero();
        if level_up == Vec3::ZERO {
            return;
        }
        let err = f.dot(up.cross(level_up)).atan2(up.dot(level_up));
        // Hardly at all when pointing straight up or down.
        let k = (1.0 - f.y.abs()).powi(2) * (1.0 - (-LEVEL_RATE * dt).exp());
        self.rot = (Quat::from_axis_angle(f, err * k) * self.rot).normalize();
    }

    pub fn direction(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }
}

#[derive(Component)]
pub struct ChaseCamera {
    /// The view direction, which trails the aim direction.
    view: Quat,
    /// Free look offset from the view, radians.
    look_yaw: f32,
    look_pitch: f32,
    free_looking: bool,
    /// Whether zoom is toggled on.
    zoomed: bool,
    /// Current zoom, 0 = normal, 1 = fully zoomed.
    zoom: f32,
}

/// Camera offset behind and above the player, in view space.
const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 3.0, 20.0);

/// How quickly the view catches up with the aim circle (1/s). Higher is snappier.
const CAMERA_FOLLOW_RATE: f32 = 5.0;

const NORMAL_FOV: f32 = 60.0;
const ZOOM_FOV: f32 = 25.0;
/// How quickly zoom and free look settle (1/s).
const ZOOM_RATE: f32 = 10.0;
const LOOK_RETURN_RATE: f32 = 8.0;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: NORMAL_FOV.to_radians(),
            far: 30_000.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 800.0, 20.0),
        DistanceFog {
            color: Color::srgb(0.62, 0.75, 0.88),
            falloff: FogFalloff::Linear {
                start: 2_000.0,
                end: 16_000.0,
            },
            ..default()
        },
        ChaseCamera {
            view: Quat::IDENTITY,
            look_yaw: 0.0,
            look_pitch: 0.0,
            free_looking: false,
            zoomed: false,
            zoom: 0.0,
        },
    ));
}

fn update_mouse_aim(
    time: Res<Time<Real>>,
    motion: Res<AccumulatedMouseMotion>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions>,
    settings: Res<Settings>,
    mut aim: ResMut<MouseAim>,
    mut camera: Single<&mut ChaseCamera>,
) {
    camera.free_looking = settings.pressed(&keys, Action::FreeLook);
    aim.level(time.delta_secs());
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    // Zoomed in, the same hand movement moves the circle the same distance
    // across the screen as it does unzoomed.
    let zoom =
        (current_fov(camera.zoom).to_radians() * 0.5).tan() / (NORMAL_FOV.to_radians() * 0.5).tan();
    if camera.free_looking {
        let delta = motion.delta * settings.look_per_pixel() * zoom;
        camera.look_yaw = (camera.look_yaw - delta.x).clamp(-3.0, 3.0);
        camera.look_pitch = (camera.look_pitch - delta.y).clamp(-1.4, 1.4);
    } else {
        let delta = motion.delta * settings.aim_per_pixel() * zoom;
        aim.turn(delta.x, -delta.y);
    }
}

fn current_fov(zoom: f32) -> f32 {
    NORMAL_FOV + (ZOOM_FOV - NORMAL_FOV) * zoom
}

fn follow_player(
    time: Res<Time<Real>>,
    state: Res<State<GameState>>,
    mouse: Res<ButtonInput<MouseButton>>,
    aim: Res<MouseAim>,
    player: Single<&Transform, (With<Player>, Without<ChaseCamera>)>,
    camera: Single<(&mut Transform, &mut ChaseCamera, &mut Projection)>,
) {
    let (mut tf, mut cam, mut projection) = camera.into_inner();
    // Real time so the view keeps settling smoothly in the pause menu.
    let dt = time.delta_secs();
    let ease = |rate: f32| 1.0 - (-rate * dt).exp();

    if *state.get() == GameState::Playing {
        cam.view = cam
            .view
            .slerp(aim.rotation(), ease(CAMERA_FOLLOW_RATE))
            .normalize();

        // Free look springs back once the key is released.
        if !cam.free_looking {
            let k = ease(LOOK_RETURN_RATE);
            cam.look_yaw -= cam.look_yaw * k;
            cam.look_pitch -= cam.look_pitch * k;
        }

        if mouse.just_pressed(MouseButton::Right) {
            cam.zoomed = !cam.zoomed;
        }
        let zoom_target = if cam.zoomed { 1.0 } else { 0.0 };
        cam.zoom += (zoom_target - cam.zoom) * ease(ZOOM_RATE);
    }

    if let Projection::Perspective(p) = projection.as_mut() {
        p.fov = current_fov(cam.zoom).to_radians();
    }

    // Testing aid: WTAT_LOOK="yaw,pitch,distance" (degrees, metres) holds
    // a fixed view of the player's plane for screenshots.
    let mut offset = CAMERA_OFFSET;
    if let Ok(look) = std::env::var("WTAT_LOOK") {
        let v: Vec<f32> = look
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        cam.look_yaw = v.first().copied().unwrap_or(0.0).to_radians();
        cam.look_pitch = v.get(1).copied().unwrap_or(0.0).to_radians();
        if let Some(d) = v.get(2) {
            offset = Vec3::new(0.0, 0.0, *d);
        }
    }
    let look = Quat::from_euler(EulerRot::YXZ, cam.look_yaw, cam.look_pitch, 0.0);
    let rot = cam.view * look;
    tf.translation = player.translation + rot * offset;
    tf.rotation = rot;
}

/// Snap the view straight to the aim direction (used on reset).
pub fn snap_view(camera: &mut ChaseCamera, aim: &MouseAim) {
    camera.view = aim.rotation();
    camera.look_yaw = 0.0;
    camera.look_pitch = 0.0;
}
