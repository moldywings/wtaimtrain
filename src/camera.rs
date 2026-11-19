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

/// Where the player wants to point, in world space.
#[derive(Resource, Default)]
pub struct MouseAim {
    pub yaw: f32,
    pub pitch: f32,
}

impl MouseAim {
    pub fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
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
    motion: Res<AccumulatedMouseMotion>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions>,
    settings: Res<Settings>,
    mut aim: ResMut<MouseAim>,
    mut camera: Single<&mut ChaseCamera>,
) {
    camera.free_looking = settings.pressed(&keys, Action::FreeLook);
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    // Same sensitivity zoomed or not.
    let delta = motion.delta * settings.aim_per_pixel();
    if camera.free_looking {
        camera.look_yaw = (camera.look_yaw - delta.x).clamp(-3.0, 3.0);
        camera.look_pitch = (camera.look_pitch - delta.y).clamp(-1.4, 1.4);
    } else {
        aim.yaw -= delta.x;
        // Stop just short of straight up/down so the camera never flips.
        aim.pitch = (aim.pitch - delta.y).clamp(-1.53, 1.53);
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

    let look = Quat::from_euler(EulerRot::YXZ, cam.look_yaw, cam.look_pitch, 0.0);
    let rot = cam.view * look;
    tf.translation = player.translation + rot * CAMERA_OFFSET;
    tf.rotation = rot;
}

/// Snap the view straight to the aim direction (used on reset).
pub fn snap_view(camera: &mut ChaseCamera, aim: &MouseAim) {
    camera.view = aim.rotation();
    camera.look_yaw = 0.0;
    camera.look_pitch = 0.0;
}
