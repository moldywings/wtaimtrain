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
/// the screen, and it keeps itself upright (horizon level) except when
/// pointing almost straight up or down.
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
const LEVEL_RATE: f32 = 10.0;

impl MouseAim {
    pub fn rotation(&self) -> Quat {
        self.rot
    }

    pub fn reset(&mut self) {
        self.rot = Quat::IDENTITY;
    }

    /// Move the aim by a screen-space turn (rad): right and up.
    fn turn(&mut self, right: f32, up: f32) {
        // Side to side turns about the world's vertical, so the horizon
        // stays level however far up or down you look. Only when pointing
        // almost straight up or down does it turn about the view's own up,
        // so the circle can still move across the screen there.
        let f = self.rot * Vec3::NEG_Z;
        let own_up = self.rot * Vec3::Y;
        let world_up = if own_up.y >= 0.0 {
            Vec3::Y
        } else {
            Vec3::NEG_Y
        };
        let t = ((f.y.abs() - 0.9) / 0.09).clamp(0.0, 1.0);
        let axis = world_up
            .lerp(own_up, t * t * (3.0 - 2.0 * t))
            .normalize_or(Vec3::Y);
        self.rot = (Quat::from_axis_angle(axis, -right) * self.rot * Quat::from_rotation_x(up))
            .normalize();
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
        let k = (1.0 - f.y.abs()).sqrt() * (1.0 - (-LEVEL_RATE * dt).exp());
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
    /// Where free look is heading; the view eases toward it.
    look_goal: Vec2,
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
const LOOK_RETURN_RATE: f32 = 6.0;
/// How quickly free look catches up with the mouse (1/s).
const LOOK_FOLLOW_RATE: f32 = 10.0;

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
            look_goal: Vec2::ZERO,
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
    // Zoomed in, the aim slows down a bit (less than the view narrows, so
    // it doesn't feel sluggish), times the zoomed sensitivity setting.
    let narrowing =
        (current_fov(camera.zoom).to_radians() * 0.5).tan() / (NORMAL_FOV.to_radians() * 0.5).tan();
    let zoom = narrowing.sqrt() * (1.0 + (settings.zoom_sensitivity - 1.0) * camera.zoom);
    if camera.free_looking {
        let delta = motion.delta * settings.look_per_pixel() * zoom;
        camera.look_goal.x = (camera.look_goal.x - delta.x).clamp(-3.0, 3.0);
        camera.look_goal.y = (camera.look_goal.y - delta.y).clamp(-1.4, 1.4);
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

        // Free look eases toward where the mouse put it, and springs back
        // once the key is released.
        let k = if cam.free_looking {
            ease(LOOK_FOLLOW_RATE)
        } else {
            cam.look_goal = Vec2::ZERO;
            ease(LOOK_RETURN_RATE)
        };
        cam.look_yaw += (cam.look_goal.x - cam.look_yaw) * k;
        cam.look_pitch += (cam.look_goal.y - cam.look_pitch) * k;

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
    camera.look_goal = Vec2::ZERO;
}
