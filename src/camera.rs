//! Mouse aim: the mouse steers a free aim direction, the camera looks along
//! it from behind the player, and the plane chases it (see `flight`).

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

use crate::flight::Player;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MouseAim>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    update_mouse_aim.before(crate::flight::fly_player),
                    follow_player.after(crate::flight::fly_player),
                ),
            );
    }
}

/// Where the player wants to point, in world space.
#[derive(Resource)]
pub struct MouseAim {
    pub yaw: f32,
    pub pitch: f32,
    /// Radians of aim movement per pixel of mouse movement.
    pub sensitivity: f32,
}

impl Default for MouseAim {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            sensitivity: 0.0015,
        }
    }
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
pub struct ChaseCamera;

/// Camera offset behind and above the player, in aim space.
const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 3.0, 20.0);

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 60f32.to_radians(),
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
        ChaseCamera,
    ));
}

fn update_mouse_aim(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    mut aim: ResMut<MouseAim>,
) {
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    let delta = motion.delta;
    aim.yaw -= delta.x * aim.sensitivity;
    // Stop just short of straight up/down so the camera never flips.
    aim.pitch = (aim.pitch - delta.y * aim.sensitivity).clamp(-1.53, 1.53);
}

fn follow_player(
    aim: Res<MouseAim>,
    player: Single<&Transform, (With<Player>, Without<ChaseCamera>)>,
    mut camera: Single<&mut Transform, With<ChaseCamera>>,
) {
    let rot = aim.rotation();
    camera.translation = player.translation + rot * CAMERA_OFFSET;
    camera.rotation = rot;
}
