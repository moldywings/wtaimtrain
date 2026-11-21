//! Mouse aim: the mouse moves a free aim direction (the circle on screen),
//! the camera swings after it with a short lag, and the plane chases it
//! (see `flight`). Like War Thunder, a quick flick moves the circle off
//! center before the view catches up. Holding the free look key turns the
//! camera without moving the aim; the right mouse button toggles zoom.
//! Planes with turrets have a gunner view (F6, back with F3): the mouse
//! swings the gunsight round the plane and the keys fly it.

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
            .init_resource::<GunnerView>()
            .add_systems(Startup, spawn_camera)
            .add_systems(
                Update,
                (
                    (switch_view, update_mouse_aim)
                        .chain()
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

/// Gunner view: on, and where the gunsight looks (world yaw and pitch, rad).
#[derive(Resource, Default)]
pub struct GunnerView {
    pub active: bool,
    pub yaw: f32,
    pub pitch: f32,
}

/// Yaw and pitch of a direction (yaw 0 looks along -Z, + turns left).
fn yaw_pitch(dir: Vec3) -> (f32, f32) {
    let d = dir.normalize_or(Vec3::NEG_Z);
    ((-d.x).atan2(-d.z), d.y.clamp(-1.0, 1.0).asin())
}

fn from_yaw_pitch(yaw: f32, pitch: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Free look and the gunsight can look nearly straight up or down.
const MAX_LOOK_PITCH: f32 = 1.55;

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

// Each airframe has its own camera distance, height and tilt (see
// `models`): measured from War Thunder's chase view for each aircraft.

/// How quickly the view catches up with the aim circle (1/s). Higher is snappier.
const CAMERA_FOLLOW_RATE: f32 = 5.0;

/// Vertical field of view, degrees: about 61 normally, and the game's zoom
/// narrows it about 2.2 times.
const NORMAL_FOV: f32 = 61.0;
const ZOOM_FOV: f32 = 30.0;
/// How quickly zoom and free look settle (1/s).
const ZOOM_RATE: f32 = 10.0;
const LOOK_RETURN_RATE: f32 = 6.0;
/// How quickly free look catches up with the mouse (1/s).
const LOOK_FOLLOW_RATE: f32 = 10.0;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // HDR with bloom that only picks up over-bright things (the
        // tracers), so they glow without blurring the rest of the scene.
        bevy::camera::Hdr,
        bevy::post_process::bloom::Bloom {
            intensity: 0.2,
            prefilter: bevy::post_process::bloom::BloomPrefilter {
                threshold: 3.0,
                threshold_softness: 0.5,
            },
            composite_mode: bevy::post_process::bloom::BloomCompositeMode::Additive,
            ..bevy::post_process::bloom::Bloom::NATURAL
        },
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

/// F6 switches to gunner view (planes with a working turret), F3 back.
fn switch_view(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    mut gunner: ResMut<GunnerView>,
    camera: Single<&Transform, With<ChaseCamera>>,
    player: Single<Option<&crate::turrets::Turrets>, With<Player>>,
    mp: Res<crate::multiplayer::Mp>,
) {
    let has_turret = player.is_some_and(|t| t.any_alive()) && mp.alive();
    if settings.just_pressed(&keys, Action::GunnerView) && has_turret && !gunner.active {
        // Start looking where the camera looks now.
        let (yaw, pitch) = yaw_pitch(camera.forward().as_vec3());
        *gunner = GunnerView {
            active: true,
            yaw,
            pitch,
        };
    }
    // Testing aid: WTAT_GUNNER="yaw,pitch" (degrees) starts in gunner view.
    if let Ok(v) = std::env::var("WTAT_GUNNER")
        && has_turret
        && !gunner.active
    {
        let mut n = v.split(',').filter_map(|x| x.trim().parse::<f32>().ok());
        *gunner = GunnerView {
            active: true,
            yaw: n.next().unwrap_or(0.0).to_radians(),
            pitch: n.next().unwrap_or(0.0).to_radians(),
        };
    }
    if (settings.just_pressed(&keys, Action::PilotView) || !has_turret) && gunner.active {
        gunner.active = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn update_mouse_aim(
    time: Res<Time<Real>>,
    motion: Res<AccumulatedMouseMotion>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions>,
    settings: Res<Settings>,
    mut aim: ResMut<MouseAim>,
    mut camera: Single<&mut ChaseCamera>,
    mut gunner: ResMut<GunnerView>,
    player: Single<&Transform, With<Player>>,
) {
    camera.free_looking = settings.pressed(&keys, Action::FreeLook) && !gunner.active;
    // While free looking (or in gunner view, where the keys fly the plane)
    // the aim rides on the gun crosshair (the nose), so the plane holds its
    // course and the aim circle sits in the crosshair.
    if camera.free_looking || gunner.active {
        let nose = player.forward().as_vec3();
        if nose.y.abs() < 0.97 {
            let goal = Transform::default().looking_to(nose, Vec3::Y).rotation;
            let k = 1.0 - (-12.0 * time.delta_secs()).exp();
            aim.rot = aim.rot.slerp(goal, k).normalize();
        }
    }
    aim.level(time.delta_secs());
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    // Zoomed in, the aim slows down a bit (less than the view narrows, so
    // it doesn't feel sluggish), times the zoomed sensitivity setting.
    let narrowing =
        (current_fov(camera.zoom).to_radians() * 0.5).tan() / (NORMAL_FOV.to_radians() * 0.5).tan();
    let zoom = narrowing.sqrt() * (1.0 + (settings.zoom_sensitivity - 1.0) * camera.zoom);
    if gunner.active {
        let delta = motion.delta * settings.aim_per_pixel() * zoom;
        gunner.yaw = wrap(gunner.yaw - delta.x);
        gunner.pitch = (gunner.pitch - delta.y).clamp(-MAX_LOOK_PITCH, MAX_LOOK_PITCH);
    } else if camera.free_looking {
        // All the way round, and up or down to straight over or under the
        // plane (limited where the view itself already points).
        let delta = motion.delta * settings.look_per_pixel() * zoom;
        let (_, base_pitch) = yaw_pitch(camera.view * Vec3::NEG_Z);
        camera.look_goal.x -= delta.x;
        camera.look_goal.y = (camera.look_goal.y - delta.y)
            .clamp(-MAX_LOOK_PITCH - base_pitch, MAX_LOOK_PITCH - base_pitch);
    } else {
        let delta = motion.delta * settings.aim_per_pixel() * zoom;
        aim.turn(delta.x, -delta.y);
    }
}

fn current_fov(zoom: f32) -> f32 {
    NORMAL_FOV + (ZOOM_FOV - NORMAL_FOV) * zoom
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn follow_player(
    time: Res<Time<Real>>,
    state: Res<State<GameState>>,
    mouse: Res<ButtonInput<MouseButton>>,
    aim: Res<MouseAim>,
    gunner: Res<GunnerView>,
    player: Single<(&Transform, &crate::flight::Aircraft), (With<Player>, Without<ChaseCamera>)>,
    mp: Res<crate::multiplayer::Mp>,
    remotes: Query<
        (
            &Transform,
            &crate::flight::Aircraft,
            &crate::multiplayer::RemotePlane,
        ),
        Without<ChaseCamera>,
    >,
    camera: Single<(&mut Transform, &mut ChaseCamera, &mut Projection)>,
) {
    // Out of a multiplayer round: ride along behind the player you watch.
    let watched = mp
        .spectate
        .and_then(|id| remotes.iter().find(|(_, _, r)| r.id == id))
        .map(|(tf, a, _)| (*tf, a.card.model_kind()));
    let (followed, kind) = watched.unwrap_or((*player.0, player.1.card.model_kind()));
    let watched = watched.map(|w| w.0);
    let gunning = gunner.active && watched.is_none();
    let spec = kind.spec().camera;
    let (mut tf, mut cam, mut projection) = camera.into_inner();
    // Real time so the view keeps settling smoothly in the pause menu.
    let dt = time.delta_secs();
    let ease = |rate: f32| 1.0 - (-rate * dt).exp();

    if *state.get() == GameState::Playing {
        let goal = match watched {
            // Level with the horizon, looking where they fly.
            Some(w) => {
                let f = w.forward().as_vec3();
                Transform::default().looking_to(f, Vec3::Y).rotation
            }
            None => aim.rotation(),
        };
        // Free looking (and in gunner view) the view holds still, level,
        // wherever it was pointing, like the game: keyboard flying turns the
        // plane under the camera instead of swinging the camera with it.
        if cam.free_looking || gunning {
            let (yaw, pitch) = yaw_pitch(cam.view * Vec3::NEG_Z);
            cam.view = from_yaw_pitch(yaw, pitch);
        } else {
            cam.view = cam.view.slerp(goal, ease(CAMERA_FOLLOW_RATE)).normalize();
        }

        // Free look eases toward where the mouse put it, and springs back
        // once the key is released (the short way round).
        let k = if cam.free_looking {
            ease(LOOK_FOLLOW_RATE)
        } else {
            cam.look_goal = Vec2::ZERO;
            cam.look_yaw = wrap(cam.look_yaw);
            ease(LOOK_RETURN_RATE)
        };
        cam.look_yaw += (cam.look_goal.x - cam.look_yaw) * k;
        cam.look_pitch += (cam.look_goal.y - cam.look_pitch) * k;

        // (Watching someone, the mouse buttons pick who.)
        if mouse.just_pressed(MouseButton::Right) && watched.is_none() {
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
    let mut offset = spec.offset.lerp(spec.zoom_offset, cam.zoom);
    let mut tilt = Quat::from_rotation_x(-spec.tilt_deg.to_radians());
    if let Ok(look) = std::env::var("WTAT_LOOK") {
        let v: Vec<f32> = look
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        cam.look_yaw = v.first().copied().unwrap_or(0.0).to_radians();
        cam.look_pitch = v.get(1).copied().unwrap_or(0.0).to_radians();
        if let Some(d) = v.get(2) {
            offset = Vec3::new(0.0, 0.0, *d);
            tilt = Quat::IDENTITY;
        }
    }
    let rot = if gunning {
        // Gunner view: a little closer and lower, the gunsight in the middle.
        offset = spec.gunner_offset() * (1.0 - 0.25 * cam.zoom);
        tilt = Quat::IDENTITY;
        from_yaw_pitch(gunner.yaw, gunner.pitch)
    } else {
        // Free look turns about the vertical and tilts up and down, so the
        // horizon stays level wherever you look.
        let (yaw, pitch) = yaw_pitch(cam.view * Vec3::NEG_Z);
        from_yaw_pitch(
            yaw + cam.look_yaw,
            (pitch + cam.look_pitch).clamp(-MAX_LOOK_PITCH, MAX_LOOK_PITCH),
        )
    };
    tf.translation = followed.translation + rot * offset;
    tf.rotation = rot * tilt;
}

/// Snap the view straight to the aim direction (used on reset).
pub fn snap_view(camera: &mut ChaseCamera, aim: &MouseAim) {
    camera.view = aim.rotation();
    camera.look_yaw = 0.0;
    camera.look_pitch = 0.0;
    camera.look_goal = Vec2::ZERO;
}
