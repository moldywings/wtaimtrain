//! Gun turrets: up to two on top, two underneath, one in the nose and one
//! in the tail. Each carries up to four guns of one type and turns at its
//! traverse speed within its limits. Every turret on a plane aims at the
//! same point, 400 m out from the plane's center: where the gunner's
//! sight points in gunner view, or at the enemy for AI gunners.

use bevy::prelude::*;
use bevy::window::CursorOptions;
use serde::{Deserialize, Serialize};

use crate::flight::{Aircraft, Player};
use crate::models::ModelSpec;
use crate::multiplayer::{Bot, Mp, RemotePlane};
use crate::settings::Settings;
use crate::target::{ScenarioState, Target};
use crate::weapons::{GunBook, GunCard, lead_direction};

/// All turrets converge this far from the plane's center of gravity, m.
pub const CONVERGENCE: f32 = 400.0;
/// Default traverse speed, deg/s.
pub const TRAVERSE: f32 = 60.0;
/// Most guns in one turret.
pub const MAX_GUNS: u32 = 4;
/// AI gunners open fire inside this range, m.
const AI_FIRE_RANGE: f32 = 800.0;
/// How far off an AI gunner's aim wanders, degrees.
const AI_AIM_ERROR: f32 = 2.2;
/// AI gunners fire in bursts: this long a cycle, s, firing for this share.
const AI_BURST_CYCLE: f32 = 2.5;
const AI_BURST_SHARE: f32 = 0.5;
/// And start tracking inside this range, m.
const AI_TRACK_RANGE: f32 = 1_400.0;

/// Where on the airframe a turret sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TurretPlace {
    #[default]
    Top,
    Bottom,
    Front,
    Rear,
}

impl TurretPlace {
    pub const ALL: [TurretPlace; 4] = [
        TurretPlace::Top,
        TurretPlace::Bottom,
        TurretPlace::Front,
        TurretPlace::Rear,
    ];

    /// Most turrets of this kind on one aircraft.
    pub fn max(self) -> usize {
        match self {
            TurretPlace::Top | TurretPlace::Bottom => 2,
            TurretPlace::Front | TurretPlace::Rear => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TurretPlace::Top => "Top",
            TurretPlace::Bottom => "Bottom",
            TurretPlace::Front => "Front",
            TurretPlace::Rear => "Rear",
        }
    }

    /// Top and bottom turrets slide along the fuselage and can face
    /// either way.
    pub fn slides(self) -> bool {
        matches!(self, TurretPlace::Top | TurretPlace::Bottom)
    }
}

/// A turret on an aircraft preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Turret {
    pub place: TurretPlace,
    /// Gun preset name.
    pub gun: String,
    /// 1 to 4 guns.
    pub count: u32,
    /// Top and bottom: how far along the fuselage, 0 = tail end, 1 = nose end.
    pub position: f32,
    /// Top and bottom: faces backward when not in use (instead of forward).
    pub faces_back: bool,
    /// Traverse limits from where it faces, degrees: left and right, and
    /// down and up (for a bottom turret, "up" is away from the fuselage).
    pub yaw_left: f32,
    pub yaw_right: f32,
    pub pitch_down: f32,
    pub pitch_up: f32,
    /// Traverse speed, deg/s.
    pub speed: f32,
}

impl Default for Turret {
    fn default() -> Self {
        Turret::new(TurretPlace::Top)
    }
}

impl Turret {
    pub fn new(place: TurretPlace) -> Turret {
        let (yaw, down, up, position) = match place {
            TurretPlace::Top => (180.0, 5.0, 80.0, 0.55),
            TurretPlace::Bottom => (180.0, 5.0, 80.0, 0.4),
            TurretPlace::Front | TurretPlace::Rear => (60.0, 45.0, 45.0, 0.5),
        };
        Turret {
            place,
            gun: "UBS 12.7 mm".into(),
            count: 2,
            position,
            faces_back: false,
            yaw_left: yaw,
            yaw_right: yaw,
            pitch_down: down,
            pitch_up: up,
            speed: TRAVERSE,
        }
    }

    /// Turns all the way round.
    pub fn free_yaw(&self) -> bool {
        self.yaw_left + self.yaw_right >= 359.9
    }

    pub fn guns(&self) -> u32 {
        self.count.clamp(1, MAX_GUNS)
    }

    /// Where it sits in the plane's own frame (meters, forward -Z), and the
    /// way it faces when not in use (its "up" points away from the
    /// fuselage).
    pub fn mount(&self, spec: &ModelSpec) -> (Vec3, Quat) {
        let z =
            spec.turret_z.0 + (spec.turret_z.1 - spec.turret_z.0) * self.position.clamp(0.0, 1.0);
        let model = match self.place {
            TurretPlace::Top => Vec3::new(0.0, spec.top_at(z), z),
            TurretPlace::Bottom => Vec3::new(0.0, spec.bottom_at(z), z),
            TurretPlace::Front => spec.front,
            TurretPlace::Rear => spec.rear,
        };
        let back = match self.place {
            TurretPlace::Top | TurretPlace::Bottom => self.faces_back,
            TurretPlace::Front => false,
            TurretPlace::Rear => true,
        };
        let yaw = if back {
            Quat::from_rotation_y(std::f32::consts::PI)
        } else {
            Quat::IDENTITY
        };
        let flip = if self.place == TurretPlace::Bottom {
            Quat::from_rotation_z(std::f32::consts::PI)
        } else {
            Quat::IDENTITY
        };
        (spec.correction().transform_point(model), yaw * flip)
    }

    /// Half the size of the turret, m (its hit box).
    pub fn half_size(&self, spec: &ModelSpec) -> f32 {
        let model = match self.place {
            TurretPlace::Top | TurretPlace::Bottom => 0.76,
            TurretPlace::Front | TurretPlace::Rear => 0.57,
        };
        model * spec.turret_scale
    }

    /// Where each gun's model sits on the turret's cradle (forward -Z):
    /// one in the middle, two or three in a row, four in a rectangle.
    pub fn gun_spots(&self, spec: &ModelSpec) -> Vec<Vec3> {
        let half = self.half_size(spec);
        let s = half * 0.55;
        let z = -half * 0.7;
        let spots: &[(f32, f32)] = match self.guns() {
            1 => &[(0.0, 0.0)],
            2 => &[(-0.5, 0.0), (0.5, 0.0)],
            3 => &[(-1.0, 0.0), (0.0, 0.0), (1.0, 0.0)],
            _ => &[(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)],
        };
        spots
            .iter()
            .map(|(x, y)| Vec3::new(x * s, y * s, z))
            .collect()
    }
}

/// The gun model's muzzle is at +0.505 on its Z axis.
const GUN_MUZZLE: f32 = 0.505;

/// One turret on a plane, as it is now.
#[derive(Clone, Debug)]
pub struct TurretState {
    pub config: Turret,
    /// Pivot in the plane's frame, and the way it faces at rest.
    pub mount: Vec3,
    pub base: Quat,
    pub half: f32,
    /// Where its guns' muzzles are on the cradle.
    pub muzzles: Vec<Vec3>,
    /// Current traverse from the rest position, rad (yaw + right, pitch + up).
    pub yaw: f32,
    pub pitch: f32,
    /// Not pushing against a traverse limit, so it may fire.
    pub can_fire: bool,
    /// Its guns fire this frame.
    pub fire: bool,
    pub lost: bool,
    /// Its gun preset.
    pub card: GunCard,
}

impl TurretState {
    /// The turret body's rotation in the plane's frame (yaw only).
    pub fn body_rotation(&self) -> Quat {
        self.base * Quat::from_rotation_y(-self.yaw)
    }

    /// The guns' rotation in the plane's frame.
    pub fn rotation(&self) -> Quat {
        self.body_rotation() * Quat::from_rotation_x(self.pitch)
    }

    /// Which way the guns point, in the plane's frame.
    pub fn direction(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    /// Gun `i`'s muzzle in the plane's frame.
    pub fn muzzle(&self, i: usize) -> Vec3 {
        self.mount + self.rotation() * self.muzzles.get(i).copied().unwrap_or(Vec3::ZERO)
    }

    /// Clamp a wanted traverse to the limits.
    fn clamp(&self, yaw: f32, pitch: f32) -> (f32, f32) {
        let c = &self.config;
        let yaw = if c.free_yaw() {
            yaw
        } else {
            yaw.clamp(-c.yaw_left.to_radians(), c.yaw_right.to_radians())
        };
        let pitch = pitch.clamp(-c.pitch_down.to_radians(), c.pitch_up.to_radians());
        (yaw, pitch)
    }

    /// The traverse that points the guns along `dir` (plane frame).
    fn angles_for(&self, dir: Vec3) -> (f32, f32) {
        let d = self.base.inverse() * dir;
        let yaw = d.x.atan2(-d.z);
        let pitch = d.y.atan2(Vec2::new(d.x, d.z).length());
        (yaw, pitch)
    }

    /// Turn toward pointing along `dir` (plane frame), or back to rest.
    /// Returns how far off the wanted direction the guns still are, rad.
    pub fn track(&mut self, dir: Option<Vec3>, dt: f32) -> f32 {
        let (want_yaw, want_pitch) = dir.map(|d| self.angles_for(d)).unwrap_or((0.0, 0.0));
        let (yaw, pitch) = self.clamp(want_yaw, want_pitch);
        let step = self.config.speed.max(1.0).to_radians() * dt;
        let mut dy = yaw - self.yaw;
        if self.config.free_yaw() {
            dy = wrap(dy);
        }
        self.yaw += dy.clamp(-step, step);
        if self.config.free_yaw() {
            self.yaw = wrap(self.yaw);
        }
        self.pitch += (pitch - self.pitch).clamp(-step, step);
        let outside = wrap(want_yaw - yaw).abs() > 0.005 || (want_pitch - pitch).abs() > 0.005;
        let at_limit = wrap(yaw - self.yaw).abs() < 0.005 && (pitch - self.pitch).abs() < 0.005;
        self.can_fire = !(outside && at_limit);
        match dir {
            Some(d) => self.direction().angle_between(d.normalize_or(Vec3::NEG_Z)),
            None => 0.0,
        }
    }
}

fn wrap(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

/// A plane's turrets.
#[derive(Component, Default)]
pub struct Turrets {
    pub list: Vec<TurretState>,
    /// Knocked out turrets (bit per turret), set by whoever keeps score.
    pub lost: u8,
    /// What this was built from.
    built_from: Option<(crate::models::ModelKind, Vec<Turret>, Vec<GunCard>)>,
}

impl Turrets {
    /// Any turret still working.
    pub fn any_alive(&self) -> bool {
        self.list.iter().any(|t| !t.lost)
    }

    pub fn any_firing(&self) -> bool {
        self.list.iter().any(|t| t.fire)
    }

    /// A copy of where the turrets are and which are lost.
    pub fn snapshot(&self) -> Turrets {
        Turrets {
            list: self.list.clone(),
            lost: self.lost,
            built_from: None,
        }
    }
}

/// Where the player's gunners aim, worked out from the gunner sight.
#[derive(Resource, Default)]
pub struct GunnerAim {
    /// World point the turrets converge on (in gunner view).
    pub point: Option<Vec3>,
    /// The trigger is pulled in gunner view.
    pub firing: bool,
}

pub struct TurretsPlugin;

impl Plugin for TurretsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GunnerAim>()
            .add_systems(Startup, setup_assets)
            .add_systems(
                Update,
                (sync_turrets, aim_turrets, show_turrets)
                    .chain()
                    .after(crate::camera::follow_player)
                    .after(crate::target::fly_target)
                    .before(crate::weapons::Firing),
            );
    }
}

#[derive(Resource)]
struct TurretAssets {
    dome: Handle<Mesh>,
    ball: Handle<Mesh>,
    gun: Handle<Mesh>,
    black: Handle<StandardMaterial>,
}

fn setup_assets(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let path = "models/turrets.glb";
    commands.insert_resource(TurretAssets {
        dome: assets.load(
            GltfAssetLabel::Primitive {
                mesh: 0,
                primitive: 0,
            }
            .from_asset(path),
        ),
        ball: assets.load(
            GltfAssetLabel::Primitive {
                mesh: 1,
                primitive: 0,
            }
            .from_asset(path),
        ),
        gun: assets.load(
            GltfAssetLabel::Primitive {
                mesh: 0,
                primitive: 0,
            }
            .from_asset(crate::weapons::GUN_MODEL),
        ),
        black: materials.add(StandardMaterial {
            base_color: Color::srgb(0.06, 0.06, 0.065),
            perceptual_roughness: 0.6,
            metallic: 0.2,
            ..default()
        }),
    });
}

/// Give every plane the turrets its preset carries.
fn sync_turrets(
    mut commands: Commands,
    settings: Res<Settings>,
    mut planes: Query<(Entity, &Aircraft, Option<&GunBook>, Option<&mut Turrets>)>,
) {
    for (entity, aircraft, book, turrets) in &mut planes {
        let kind = aircraft.card.model_kind();
        let configs = &aircraft.card.weapons.turrets;
        let cards: Vec<GunCard> = configs
            .iter()
            .map(|t| GunBook::gun(book, &settings, &t.gun).clone())
            .collect();
        if turrets.as_ref().is_some_and(|t| {
            t.built_from
                .as_ref()
                .is_some_and(|(k, c, g)| *k == kind && c == configs && *g == cards)
        }) {
            continue;
        }
        let spec = kind.spec();
        let list = configs
            .iter()
            .zip(&cards)
            .map(|(c, card)| {
                let (mount, base) = c.mount(spec);
                let gs = crate::weapons::gun_model_scale(card.caliber_mm());
                TurretState {
                    config: c.clone(),
                    mount,
                    base,
                    half: c.half_size(spec),
                    muzzles: c
                        .gun_spots(spec)
                        .into_iter()
                        .map(|p| p - Vec3::Z * GUN_MUZZLE * gs)
                        .collect(),
                    yaw: 0.0,
                    pitch: 0.0,
                    can_fire: true,
                    fire: false,
                    lost: false,
                    card: card.clone(),
                }
            })
            .collect();
        let built = Turrets {
            list,
            lost: turrets.as_ref().map(|t| t.lost).unwrap_or(0),
            built_from: Some((kind, configs.clone(), cards)),
        };
        match turrets {
            Some(mut t) => *t = built,
            None => {
                commands.entity(entity).insert(built);
            }
        }
    }
}

/// A plane that might be shot at, for AI gunners.
#[derive(Clone, Copy)]
struct Mark {
    entity: Entity,
    position: Vec3,
    velocity: Vec3,
    /// Multiplayer id (the host is 0), or none offline.
    id: Option<u64>,
    player: bool,
}

/// Point every turret: at the gunner's sight, at the enemy for AI gunners,
/// or back to rest.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn aim_turrets(
    time: Res<Time>,
    settings: Res<Settings>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions>,
    state: Res<State<crate::GameState>>,
    gunner: Res<crate::camera::GunnerView>,
    camera: Single<&Transform, (With<crate::camera::ChaseCamera>, Without<Aircraft>)>,
    mp: Res<Mp>,
    scenario: Res<ScenarioState>,
    mut aim: ResMut<GunnerAim>,
    mut planes: Query<(
        Entity,
        &Transform,
        &Aircraft,
        &mut Turrets,
        Has<Player>,
        Option<&Target>,
        Option<&RemotePlane>,
        Has<Bot>,
    )>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    // Who could be shot at.
    let marks: Vec<Mark> = planes
        .iter()
        .filter_map(|(e, tf, a, _, is_player, target, remote, _)| {
            let alive = if is_player {
                if mp.online() {
                    mp.alive()
                } else {
                    !scenario.player_down()
                }
            } else if let Some(r) = remote {
                r.alive
            } else {
                !mp.online() && target.is_some_and(|t| !t.destroyed())
            };
            alive.then_some(Mark {
                entity: e,
                position: tf.translation,
                velocity: a.body.velocity,
                id: if is_player {
                    Some(mp.my_id)
                } else {
                    remote.map(|r| r.id)
                },
                player: is_player,
            })
        })
        .collect();

    // The gunner sight: the point on the sight line 400 m from the plane.
    let player_pos = marks.iter().find(|m| m.player).map(|m| m.position);
    aim.point = None;
    aim.firing = false;
    if gunner.active
        && let Some(cg) = player_pos
    {
        let c = camera.translation;
        let v = camera.forward().as_vec3();
        let rel = c - cg;
        let b = v.dot(rel);
        let disc = b * b - (rel.length_squared() - CONVERGENCE * CONVERGENCE);
        let t = -b + disc.max(0.0).sqrt();
        aim.point = Some(c + v * t.max(1.0));
        aim.firing = *state.get() == crate::GameState::Playing
            && mp.guns_free()
            && crate::weapons::trigger_now(&mouse, &cursor, settings.single_fire);
    }

    for (entity, tf, aircraft, mut turrets, is_player, target, remote, is_bot) in &mut planes {
        if turrets.list.is_empty() {
            continue;
        }
        let lost = turrets.lost;
        for (i, t) in turrets.list.iter_mut().enumerate() {
            t.lost = lost & (1 << i) != 0;
        }
        let to_local = |p: Vec3| tf.rotation.inverse() * (p - tf.translation);
        if is_player {
            let dir_to = |t: &TurretState| aim.point.map(|p| to_local(p) - t.mount);
            for t in turrets.list.iter_mut() {
                let d = dir_to(t);
                t.track(d, dt);
                t.fire = aim.firing && t.can_fire && !t.lost;
            }
            continue;
        }
        if let Some(r) = remote.filter(|_| !is_bot || !mp.is_host()) {
            // Another player's (or, joined, an AI target's) gunners: aim
            // where they said.
            for t in turrets.list.iter_mut() {
                let d = r.turret_aim.map(|p| to_local(p) - t.mount);
                t.track(d, dt);
                t.fire = r.alive && r.turret_firing && t.can_fire && !t.lost;
            }
            continue;
        }
        // AI gunners: the practice target, or AI targets on the host.
        let flying = if is_bot {
            remote.is_some_and(|r| r.alive) && mp.guns_free_for_bots()
        } else {
            !mp.online() && target.is_some_and(|t| !t.destroyed()) && !scenario.player_down()
        };
        let my_id = remote.map(|r| r.id);
        let foes: Vec<Mark> = marks
            .iter()
            .filter(|m| m.entity != entity)
            .filter(|m| match my_id {
                Some(id) => m.id.is_some_and(|other| mp.enemies(id, other)),
                None => m.player,
            })
            .copied()
            .collect();
        let now = time.elapsed_secs();
        let seed = entity.index_u32() as f32 * 0.618;
        for (k, t) in turrets.list.iter_mut().enumerate() {
            let pivot = tf.transform_point(t.mount);
            // A human-ish gunner: the aim wanders around the lead point,
            // and fire comes in bursts.
            let phase = seed + k as f32 * 1.7;
            let err = AI_AIM_ERROR.to_radians();
            let wobble = Quat::from_euler(
                EulerRot::YXZ,
                err * ((now * 0.9 + phase).sin() + 0.5 * (now * 2.3 + phase * 3.0).sin()) / 1.5,
                err * ((now * 0.7 + phase * 2.0).cos() + 0.5 * (now * 1.9 + phase).sin()) / 1.5,
                0.0,
            );
            let bursting =
                ((now + phase * AI_BURST_CYCLE) / AI_BURST_CYCLE).fract() < AI_BURST_SHARE;
            let foe = foes
                .iter()
                .filter(|m| m.position.distance(pivot) < AI_TRACK_RANGE)
                .min_by(|a, b| {
                    a.position
                        .distance(pivot)
                        .total_cmp(&b.position.distance(pivot))
                });
            let dir = foe.filter(|_| flying && !t.lost).and_then(|m| {
                lead_direction(
                    &t.card,
                    pivot,
                    aircraft.body.velocity,
                    m.position,
                    m.velocity,
                )
                .map(|d| tf.rotation.inverse() * (wobble * d))
            });
            let off = t.track(dir, dt);
            let range = foe.map(|m| m.position.distance(pivot)).unwrap_or(f32::MAX);
            t.fire = dir.is_some()
                && bursting
                && t.can_fire
                && off < 1.5f32.to_radians()
                && range < AI_FIRE_RANGE;
        }
    }
}

/// The turret models on a plane.
#[derive(Component)]
struct TurretModels {
    shown: Vec<Turret>,
    kind: crate::models::ModelKind,
    bodies: Vec<Entity>,
    cradles: Vec<Entity>,
    lost: u8,
}

#[derive(Component)]
struct TurretPart;

/// Draw the turrets where they point, and knock out lost ones.
#[allow(clippy::type_complexity)]
fn show_turrets(
    mut commands: Commands,
    turret_assets: Res<TurretAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut planes: Query<(
        Entity,
        &GlobalTransform,
        &Aircraft,
        &Turrets,
        Option<&mut TurretModels>,
    )>,
    mut parts: Query<(&mut Transform, &mut Visibility), With<TurretPart>>,
) {
    for (plane, gtf, aircraft, turrets, models) in &mut planes {
        let kind = aircraft.card.model_kind();
        let configs: Vec<Turret> = turrets.list.iter().map(|t| t.config.clone()).collect();
        let rebuild = models
            .as_ref()
            .is_none_or(|m| m.shown != configs || m.kind != kind);
        if rebuild {
            if let Some(m) = &models {
                for e in &m.bodies {
                    commands.entity(*e).despawn();
                }
            }
            let spec = kind.spec();
            let mut bodies = vec![];
            let mut cradles = vec![];
            for t in &turrets.list {
                let mesh = if t.config.place.slides() {
                    turret_assets.dome.clone()
                } else {
                    turret_assets.ball.clone()
                };
                let gs = crate::weapons::gun_model_scale(t.card.caliber_mm());
                let body = commands
                    .spawn((
                        TurretPart,
                        Transform::from_translation(t.mount).with_rotation(t.body_rotation()),
                        Visibility::default(),
                        ChildOf(plane),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(turret_assets.black.clone()),
                            Transform::from_scale(Vec3::splat(spec.turret_scale)),
                        ));
                    })
                    .id();
                let cradle = commands
                    .spawn((
                        TurretPart,
                        Transform::from_rotation(Quat::from_rotation_x(t.pitch)),
                        Visibility::default(),
                        ChildOf(body),
                    ))
                    .with_children(|c| {
                        for spot in t.config.gun_spots(spec) {
                            c.spawn((
                                Mesh3d(turret_assets.gun.clone()),
                                MeshMaterial3d(turret_assets.black.clone()),
                                Transform::from_translation(spot)
                                    .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
                                    .with_scale(Vec3::splat(gs)),
                            ));
                        }
                    })
                    .id();
                bodies.push(body);
                cradles.push(cradle);
            }
            commands.entity(plane).insert(TurretModels {
                shown: configs,
                kind,
                bodies,
                cradles,
                lost: 0,
            });
            continue;
        }
        let Some(mut models) = models else {
            continue;
        };
        for (i, t) in turrets.list.iter().enumerate() {
            if let (Some(b), Some(c)) = (models.bodies.get(i), models.cradles.get(i)) {
                if let Ok((mut tf, mut vis)) = parts.get_mut(*b) {
                    tf.rotation = t.body_rotation();
                    let want = if t.lost {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                    if *vis != want {
                        *vis = want;
                    }
                }
                if let Ok((mut tf, _)) = parts.get_mut(*c) {
                    tf.rotation = Quat::from_rotation_x(t.pitch);
                }
            }
        }
        // A turret just knocked out: a small burst where it was.
        let newly = turrets.lost & !models.lost;
        if newly != 0 {
            for (i, t) in turrets.list.iter().enumerate() {
                if newly & (1 << i) != 0 {
                    crate::target::spawn_explosion(
                        &mut commands,
                        &mut meshes,
                        &mut materials,
                        gtf.transform_point(t.mount),
                        0.25,
                    );
                }
            }
        }
        if models.lost != turrets.lost {
            models.lost = turrets.lost;
        }
    }
}

/// Where an AI should shoot at a plane to bring it down: the middle of a
/// fighter, or a bomber's cockpit or whichever working turret faces it best.
pub fn aim_point(
    card: &crate::flight_model::AircraftCard,
    tf: &Transform,
    turrets: Option<&Turrets>,
    from: Vec3,
) -> Vec3 {
    if card.damage_model() == crate::damage::DamageModel::Fighter {
        return tf.translation;
    }
    let spec = card.model_kind().spec();
    let toward = (from - tf.translation).normalize_or(Vec3::Y);
    let cockpit = spec
        .correction()
        .transform_point((spec.cockpit.0 + spec.cockpit.1) * 0.5);
    // The canopy shows from above and ahead.
    let cockpit_face = Vec3::new(0.0, 1.0, -0.6).normalize();
    let mut best = (
        (tf.rotation * cockpit_face).dot(toward),
        tf.transform_point(cockpit),
    );
    for t in turrets.into_iter().flat_map(|t| t.list.iter()) {
        if t.lost {
            continue;
        }
        let face = if t.config.place.slides() {
            t.base * Vec3::Y
        } else {
            t.base * Vec3::NEG_Z
        };
        let score = (tf.rotation * face).dot(toward);
        if score > best.0 {
            best = (score, tf.transform_point(t.mount));
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ModelKind;

    fn state(c: Turret) -> TurretState {
        let spec = ModelKind::Bomber.spec();
        let (mount, base) = c.mount(spec);
        TurretState {
            half: c.half_size(spec),
            muzzles: c.gun_spots(spec),
            config: c,
            mount,
            base,
            yaw: 0.0,
            pitch: 0.0,
            can_fire: true,
            fire: false,
            lost: false,
            card: GunCard::ubs(),
        }
    }

    #[test]
    fn turrets_sit_on_the_fuselage_and_face_out() {
        let spec = ModelKind::Bomber.spec();
        let top = Turret::new(TurretPlace::Top);
        let (p, base) = top.mount(spec);
        assert!(p.y > 1.0, "top turret on top: {p}");
        assert!((base * Vec3::Y).y > 0.99);
        let bottom = Turret::new(TurretPlace::Bottom);
        let (p, base) = bottom.mount(spec);
        assert!(p.y < -1.0, "bottom turret underneath: {p}");
        assert!((base * Vec3::Y).y < -0.99, "its up points down");
        assert!((base * Vec3::NEG_Z).z < -0.99, "and it faces forward");
        let (p, base) = Turret::new(TurretPlace::Rear).mount(spec);
        assert!(p.z > 12.0, "rear turret at the tail (+Z is back): {p}");
        assert!((base * Vec3::NEG_Z).z > 0.99, "facing back");
        let (p, _) = Turret::new(TurretPlace::Front).mount(spec);
        assert!(p.z < -12.0, "front turret at the nose: {p}");
    }

    #[test]
    fn turrets_traverse_at_their_speed_and_stop_at_limits() {
        let mut t = state(Turret::new(TurretPlace::Front));
        // Straight right is 90 degrees, past the 60 degree limit.
        let right = Vec3::X;
        let mut fired_while_turning = false;
        for _ in 0..30 {
            t.track(Some(right), 1.0 / 60.0);
            fired_while_turning |= t.can_fire;
        }
        // Half a second at 60 deg/s: 30 degrees, still short of the limit,
        // so it may fire though the sight is outside its reach.
        assert!(
            (t.yaw.to_degrees() - 30.0).abs() < 1.0,
            "{}",
            t.yaw.to_degrees()
        );
        assert!(fired_while_turning && t.can_fire);
        for _ in 0..120 {
            t.track(Some(right), 1.0 / 60.0);
        }
        assert!((t.yaw.to_degrees() - 60.0).abs() < 0.1);
        assert!(!t.can_fire, "pushing against the limit: no fire");
        // Back inside its reach it fires again.
        let inside = Vec3::new(0.5, 0.0, -1.0).normalize();
        for _ in 0..120 {
            t.track(Some(inside), 1.0 / 60.0);
        }
        assert!(t.can_fire);
        assert!(t.direction().angle_between(inside) < 0.01);
    }

    #[test]
    fn a_full_circle_turret_takes_the_short_way_round() {
        let mut t = state(Turret::new(TurretPlace::Top));
        // Just left of straight back: should turn left, not right.
        let back_left = Vec3::new(-0.1, 0.0, 1.0).normalize();
        t.track(Some(back_left), 0.1);
        assert!(t.yaw < 0.0, "turned the short way: {}", t.yaw);
        for _ in 0..300 {
            t.track(Some(back_left), 1.0 / 60.0);
        }
        assert!(t.direction().angle_between(back_left) < 0.01);
        assert!(t.can_fire);
    }

    #[test]
    fn gun_layouts() {
        let spec = ModelKind::Bomber.spec();
        for n in 1..=4 {
            let mut c = Turret::new(TurretPlace::Top);
            c.count = n;
            let spots = c.gun_spots(spec);
            assert_eq!(spots.len(), n as usize);
            let sum: Vec3 = spots.iter().copied().sum();
            assert!(sum.x.abs() < 1e-4 && sum.y.abs() < 1e-4, "centered");
        }
    }
}
