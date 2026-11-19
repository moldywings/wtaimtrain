//! Guns: gun presets (ballistics from muzzle velocity, projectile mass and
//! fire rate), where they're mounted on each aircraft preset, the gun models
//! on the planes, firing, bullet flight and hits on the target.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use serde::{Deserialize, Serialize};

use crate::GameState;
use crate::flight::{Aircraft, Player};
use crate::flight_model::air_density;
use crate::settings::Settings;
use crate::sound::{self, Sounds};
use crate::target::{PlayerHit, ResetScenario, ScenarioState, Target, TargetHit};

/// Path of the gun model, relative to `assets/`.
pub const GUN_MODEL: &str = "models/gun.glb";

// ---------------------------------------------------------------------------
// Gun presets
// ---------------------------------------------------------------------------

/// A gun preset. Ballistics come from these numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GunCard {
    pub name: String,
    /// m/s.
    pub muzzle_velocity: f32,
    /// Projectile mass, kg.
    pub projectile_mass: f32,
    /// Rounds per minute.
    pub fire_rate: f32,
    /// Optional: bore in mm (sets drag and the size of the gun model).
    /// Estimated from the projectile mass when blank.
    pub caliber: Option<f32>,
    /// Optional: dispersion, mils (1 mil = 1 m at 1 km): the width of the
    /// circle the rounds land in. The editor shows it as meters at 500 m.
    pub spread: Option<f32>,
    /// Color of the tracer.
    pub tracer: TracerColor,
    /// Optional: hits from this gun that destroy an aircraft. Estimated
    /// from the caliber when blank.
    pub hits_to_kill: Option<f32>,
}

/// Tracer colors to pick from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TracerColor {
    Red,
    #[default]
    Orange,
    Yellow,
    Green,
    White,
    Blue,
    Purple,
}

impl TracerColor {
    pub const ALL: [TracerColor; 7] = [
        TracerColor::Red,
        TracerColor::Orange,
        TracerColor::Yellow,
        TracerColor::Green,
        TracerColor::White,
        TracerColor::Blue,
        TracerColor::Purple,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TracerColor::Red => "Red",
            TracerColor::Orange => "Orange",
            TracerColor::Yellow => "Yellow",
            TracerColor::Green => "Green",
            TracerColor::White => "White",
            TracerColor::Blue => "Blue",
            TracerColor::Purple => "Purple",
        }
    }

    /// Display color (for menus).
    pub fn color(self) -> Color {
        let c = self.rgb();
        Color::srgb(c.x, c.y, c.z)
    }

    fn rgb(self) -> Vec3 {
        match self {
            TracerColor::Red => Vec3::new(1.0, 0.22, 0.12),
            TracerColor::Orange => Vec3::new(1.0, 0.55, 0.15),
            TracerColor::Yellow => Vec3::new(1.0, 0.92, 0.3),
            TracerColor::Green => Vec3::new(0.35, 1.0, 0.35),
            TracerColor::White => Vec3::new(1.0, 0.97, 0.9),
            TracerColor::Blue => Vec3::new(0.35, 0.6, 1.0),
            TracerColor::Purple => Vec3::new(0.85, 0.4, 1.0),
        }
    }
}

impl Default for GunCard {
    fn default() -> Self {
        GunCard::shvak()
    }
}

/// Name of the built-in gun preset, which can't be changed or deleted.
pub const BUILTIN_GUN: &str = "ShVAK 20 mm";

/// Spread used when the preset leaves it blank, mils.
pub const DEFAULT_SPREAD: f32 = 2.0;
/// One mil of spread is half a meter wide at 500 m.
pub const MILS_TO_M_AT_500: f32 = 0.5;

impl GunCard {
    /// The Soviet 20 mm ShVAK cannon.
    pub fn shvak() -> Self {
        GunCard {
            name: BUILTIN_GUN.into(),
            muzzle_velocity: 800.0,
            projectile_mass: 0.1,
            fire_rate: 800.0,
            caliber: Some(20.0),
            spread: None,
            tracer: TracerColor::Orange,
            hits_to_kill: None,
        }
    }

    /// The Soviet 12.7 mm UBS machine gun (the Yak-3's other guns).
    pub fn ubs() -> Self {
        GunCard {
            name: "UBS 12.7 mm".into(),
            muzzle_velocity: 860.0,
            projectile_mass: 0.048,
            fire_rate: 1050.0,
            caliber: Some(12.7),
            spread: None,
            tracer: TracerColor::Red,
            hits_to_kill: None,
        }
    }

    /// Bore in mm, estimated from mass when blank (projectile mass grows
    /// with the cube of the caliber; 20 mm is about 100 g).
    pub fn caliber_mm(&self) -> f32 {
        self.caliber
            .unwrap_or_else(|| 20.0 * (self.projectile_mass.max(0.001) / 0.1).cbrt())
    }

    pub fn spread_mils(&self) -> f32 {
        self.spread.unwrap_or(DEFAULT_SPREAD)
    }

    /// Width of the spread circle at 500 m, m.
    pub fn spread_at_500(&self) -> f32 {
        self.spread_mils() * MILS_TO_M_AT_500
    }

    /// Hits needed to destroy an aircraft. Left blank, it comes from the
    /// caliber: about 6 for 20 mm, 16 for 12.7 mm, 50 for 7.62 mm and 2
    /// for 37 mm.
    pub fn hits_to_destroy(&self) -> u32 {
        let hits = self
            .hits_to_kill
            .unwrap_or_else(|| 6.0 * (20.0 / self.caliber_mm().max(3.0)).powf(2.2));
        (hits.round() as u32).max(1)
    }

    /// Drag factor: deceleration = factor * air density * speed².
    pub fn drag_factor(&self) -> f32 {
        const CD: f32 = 0.30;
        let d = self.caliber_mm() / 1000.0;
        let area = std::f32::consts::PI * d * d / 4.0;
        0.5 * CD * area / self.projectile_mass.max(0.001)
    }

    /// Seconds between shots.
    pub fn interval(&self) -> f32 {
        60.0 / self.fire_rate.max(1.0)
    }
}

/// Where a round is after flying `range` metres straight ahead at sea
/// level: time of flight, speed left, and drop below the line of fire.
pub struct Ballistics {
    pub time: f32,
    pub speed: f32,
    pub drop: f32,
}

pub fn ballistics_at(gun: &GunCard, range: f32) -> Ballistics {
    let mut v = Vec3::NEG_Z * gun.muzzle_velocity;
    let mut p = Vec3::ZERO;
    let mut t = 0.0;
    let h = 1.0 / 960.0;
    while -p.z < range && t < 20.0 {
        v += (Vec3::NEG_Y * 9.81 - v * v.length() * air_density(0.0) * gun.drag_factor()) * h;
        p += v * h;
        t += h;
    }
    Ballistics {
        time: t,
        speed: v.length(),
        drop: -p.y,
    }
}

/// Which way to point a gun so its rounds meet a target flying straight
/// on: the shooter's own speed, gravity and drag (at the shooter's height)
/// all count. `None` if the target is out of reach.
pub fn lead_direction(
    gun: &GunCard,
    shooter: Vec3,
    shooter_vel: Vec3,
    target: Vec3,
    target_vel: Vec3,
) -> Option<Vec3> {
    const H: f32 = 1.0 / 240.0;
    let rho = air_density(shooter.y.max(0.0));
    let drag = gun.drag_factor();
    let mut aim_at = target;
    let mut dir = (aim_at - shooter).normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    for _ in 0..5 {
        let mut v = shooter_vel + dir * gun.muzzle_velocity;
        let mut p = shooter;
        let mut t = 0.0;
        // Fly the round until it's as far out as the target is then.
        loop {
            let tp = target + target_vel * t;
            let line = tp - shooter;
            let reach = line.length();
            if (p - shooter).dot(line) >= reach * reach {
                aim_at += tp - p;
                break;
            }
            if t > 6.0 || (p - shooter).length() > BULLET_RANGE {
                return None;
            }
            v += (Vec3::NEG_Y * 9.81 - v * v.length() * rho * drag) * H;
            p += v * H;
            t += H;
        }
        dir = (aim_at - shooter).normalize_or(dir);
    }
    Some(dir)
}

pub fn is_builtin_gun(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case(BUILTIN_GUN)
}

// ---------------------------------------------------------------------------
// Loadouts
// ---------------------------------------------------------------------------

/// Where guns go on the airframe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// One gun firing through the propeller spinner.
    NoseCenter,
    /// Up to six around the spinner gun, in a hexagon.
    NoseRing,
    /// Up to four under the fuselage.
    Belly,
    /// Up to four in each wing (the count is per wing).
    Wings,
}

impl Slot {
    pub const ALL: [Slot; 4] = [Slot::NoseCenter, Slot::NoseRing, Slot::Belly, Slot::Wings];

    pub fn max(self) -> u32 {
        match self {
            Slot::NoseCenter => 1,
            Slot::NoseRing => 6,
            Slot::Belly => 4,
            Slot::Wings => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Slot::NoseCenter => "Nose, in the spinner",
            Slot::NoseRing => "Nose, around it",
            Slot::Belly => "Belly",
            Slot::Wings => "Wings (per wing)",
        }
    }
}

/// A group of identical guns in one place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mount {
    /// Gun preset name.
    pub gun: String,
    pub count: u32,
}

impl Mount {
    fn new(gun: &str, count: u32) -> Mount {
        Mount {
            gun: gun.into(),
            count,
        }
    }
}

/// The guns on an aircraft preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Loadout {
    pub nose_center: Mount,
    pub nose_ring: Mount,
    pub belly: Mount,
    pub wings: Mount,
    /// Range in meters where wing and belly guns are angled to meet; 0
    /// means off (they fire parallel, straight ahead).
    pub convergence: f32,
}

impl Default for Loadout {
    /// The Yak-3: a ShVAK in the spinner and two UBS above it.
    fn default() -> Self {
        Loadout {
            nose_center: Mount::new(BUILTIN_GUN, 1),
            nose_ring: Mount::new("UBS 12.7 mm", 2),
            belly: Mount::new(BUILTIN_GUN, 0),
            wings: Mount::new("UBS 12.7 mm", 0),
            convergence: CONVERGENCE,
        }
    }
}

impl Loadout {
    pub fn mount(&self, slot: Slot) -> &Mount {
        match slot {
            Slot::NoseCenter => &self.nose_center,
            Slot::NoseRing => &self.nose_ring,
            Slot::Belly => &self.belly,
            Slot::Wings => &self.wings,
        }
    }

    pub fn mount_mut(&mut self, slot: Slot) -> &mut Mount {
        match slot {
            Slot::NoseCenter => &mut self.nose_center,
            Slot::NoseRing => &mut self.nose_ring,
            Slot::Belly => &mut self.belly,
            Slot::Wings => &mut self.wings,
        }
    }

    /// Every gun: its slot, gun preset name, and muzzle position in model
    /// space (the airplane model's own axes, nose toward +Z).
    pub fn guns(&self) -> Vec<(Slot, String, Vec3)> {
        let mut out = vec![];
        for slot in Slot::ALL {
            let m = self.mount(slot);
            for p in muzzle_positions(slot, m.count.min(slot.max())) {
                out.push((slot, m.gun.clone(), p));
            }
        }
        out
    }

    pub fn total_guns(&self) -> u32 {
        Slot::ALL
            .iter()
            .map(|s| {
                let n = self.mount(*s).count.min(s.max());
                if *s == Slot::Wings { n * 2 } else { n }
            })
            .sum()
    }
}

// Airframe reference points in model space, measured from airplane.glb.
const SPINNER: Vec3 = Vec3::new(0.0, 0.50, 2.46);
/// How far the spinner gun's muzzle pokes out of the spinner.
const SPINNER_MUZZLE_Z: f32 = 2.53;
/// The ring guns' muzzles sit at the front of the cowling.
const RING_MUZZLE_Z: f32 = 2.30;
const RING_RADIUS: f32 = 0.17;
const BELLY_Y: f32 = -0.04;
const BELLY_MUZZLE_Z: f32 = 1.2;
const BELLY_SPACING: f32 = 0.17;
/// Spanwise stations for wing guns, from the root outward.
const WING_STATIONS: [f32; 4] = [0.85, 1.15, 1.45, 1.75];

/// Muzzle positions for `count` guns in a slot, in model space.
pub fn muzzle_positions(slot: Slot, count: u32) -> Vec<Vec3> {
    match slot {
        Slot::NoseCenter => (0..count.min(1))
            .map(|_| Vec3::new(SPINNER.x, SPINNER.y, SPINNER_MUZZLE_Z))
            .collect(),
        Slot::NoseRing => {
            // Hexagon corners, in degrees from "right", picked so every count
            // is symmetric: two sit above the spinner like on a Yak-3.
            let angles: &[f32] = match count {
                0 => &[],
                1 => &[90.0],
                2 => &[30.0, 150.0],
                3 => &[90.0, 210.0, 330.0],
                4 => &[30.0, 150.0, 210.0, 330.0],
                5 => &[30.0, 90.0, 150.0, 210.0, 330.0],
                _ => &[30.0, 90.0, 150.0, 210.0, 270.0, 330.0],
            };
            angles
                .iter()
                .map(|a| {
                    let r = a.to_radians();
                    Vec3::new(
                        SPINNER.x + RING_RADIUS * r.cos(),
                        SPINNER.y + RING_RADIUS * r.sin(),
                        RING_MUZZLE_Z,
                    )
                })
                .collect()
        }
        Slot::Belly => {
            let n = count.min(4) as f32;
            (0..count.min(4))
                .map(|i| {
                    let x = (i as f32 - (n - 1.0) / 2.0) * BELLY_SPACING;
                    Vec3::new(x, BELLY_Y, BELLY_MUZZLE_Z)
                })
                .collect()
        }
        Slot::Wings => {
            let mut out = vec![];
            for &x in WING_STATIONS.iter().take(count.min(4) as usize) {
                // The wing sweeps back and has dihedral toward the tip.
                let leading_edge = 0.88 - (x - 0.48) * 0.49 / 2.76;
                let y = 0.19 + (x - 0.48) * 0.09;
                for side in [-1.0, 1.0] {
                    out.push(Vec3::new(side * x, y, leading_edge + 0.2));
                }
            }
            out
        }
    }
}

/// Size of a gun model for a caliber, relative to the model as drawn (20 mm).
fn gun_model_scale(caliber_mm: f32) -> f32 {
    (caliber_mm / 20.0).clamp(0.5, 2.0)
}

/// The gun model's muzzle is at +0.505 on its Z axis.
const GUN_MODEL_MUZZLE: f32 = 0.505;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitStats>()
            .add_systems(Startup, (setup_assets, spawn_hit_ui))
            .add_systems(
                Update,
                (
                    record_target_pose.before(crate::target::fly_target),
                    (sync_gun_models, sync_player_guns),
                    (fire_guns, fly_bullets)
                        .chain()
                        .after(crate::flight::fly_player)
                        .after(crate::target::fly_target)
                        .after(sync_player_guns)
                        .run_if(in_state(GameState::Playing)),
                    fade_sparks.run_if(in_state(GameState::Playing)),
                    clear_rounds,
                    update_hit_ui,
                ),
            );
    }
}

/// Marks the airplane model under a plane, which the gun models attach to.
#[derive(Component)]
pub struct PlaneModel;

#[derive(Component)]
struct GunModel;

/// What the gun models on a plane currently show, to spot changes.
#[derive(Component, PartialEq)]
struct ShownGuns(Vec<(Vec3, f32)>);

#[derive(Resource)]
struct WeaponAssets {
    tracer_mesh: Handle<Mesh>,
    /// A flat card that always faces the camera, for the glow.
    glow_mesh: Handle<Mesh>,
    /// Per color: the bright core and the soft glow around it.
    tracer_materials: HashMap<TracerColor, (Handle<StandardMaterial>, Handle<StandardMaterial>)>,
    spark_mesh: Handle<Mesh>,
    spark_material: Handle<StandardMaterial>,
    gun_scene: Handle<WorldAsset>,
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
) {
    let glow_texture = images.add(glow_image());
    commands.insert_resource(WeaponAssets {
        // A round streak, lying along -Z.
        tracer_mesh: meshes.add(
            Mesh::from(Cylinder::new(TRACER_WIDTH * 0.5, 1.0))
                .rotated_by(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        ),
        glow_mesh: meshes.add(Rectangle::new(1.0, 1.0)),
        // A hot, nearly white core inside a soft additive glow of the
        // tracer's color.
        tracer_materials: TracerColor::ALL
            .iter()
            .map(|c| {
                let rgb = c.rgb();
                let core = rgb.lerp(Vec3::ONE, 0.45);
                (
                    *c,
                    (
                        materials.add(StandardMaterial {
                            base_color: Color::srgb(core.x, core.y, core.z),
                            unlit: true,
                            ..default()
                        }),
                        materials.add(StandardMaterial {
                            base_color: Color::srgba(rgb.x, rgb.y, rgb.z, TRACER_GLOW),
                            base_color_texture: Some(glow_texture.clone()),
                            unlit: true,
                            double_sided: true,
                            cull_mode: None,
                            alpha_mode: AlphaMode::Add,
                            ..default()
                        }),
                    ),
                )
            })
            .collect(),
        spark_mesh: meshes.add(Sphere::new(0.35)),
        spark_material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.9, 0.5),
            emissive: LinearRgba::rgb(8.0, 6.0, 1.5),
            unlit: true,
            ..default()
        }),
        gun_scene: assets.load(GltfAssetLabel::Scene(0).from_asset(GUN_MODEL)),
    });
}

/// Put gun models on each plane where its preset's loadout says.
#[allow(clippy::type_complexity)]
fn sync_gun_models(
    mut commands: Commands,
    settings: Res<Settings>,
    weapon_assets: Res<WeaponAssets>,
    planes: Query<(&Aircraft, &Children)>,
    models: Query<(Entity, Option<&ShownGuns>, Option<&Children>), With<PlaneModel>>,
    gun_models: Query<(), With<GunModel>>,
) {
    for (aircraft, children) in &planes {
        let Some((model, shown, model_children)) = children.iter().find_map(|c| models.get(c).ok())
        else {
            continue;
        };
        let wanted = ShownGuns(
            aircraft
                .card
                .weapons
                .guns()
                .into_iter()
                .map(|(_, gun, p)| (p, settings.gun(&gun).caliber_mm()))
                .collect(),
        );
        if shown == Some(&wanted) {
            continue;
        }
        for c in model_children.into_iter().flatten() {
            if gun_models.contains(*c) {
                commands.entity(*c).despawn();
            }
        }
        commands.entity(model).with_children(|m| {
            for (muzzle, caliber) in &wanted.0 {
                let s = gun_model_scale(*caliber);
                m.spawn((
                    GunModel,
                    WorldAssetRoot(weapon_assets.gun_scene.clone()),
                    Transform::from_translation(*muzzle - Vec3::Z * GUN_MODEL_MUZZLE * s)
                        .with_scale(Vec3::splat(s)),
                ));
            }
        });
        commands.entity(model).insert(wanted);
    }
}

/// One gun on a plane.
struct GunState {
    slot: Slot,
    /// Muzzle in the plane's local frame (model correction applied).
    muzzle: Vec3,
    card: GunCard,
    /// Time until the next shot, s (negative = overdue).
    cooldown: f32,
}

#[derive(Component, Default)]
struct Guns {
    /// When each gun preset's sound last played (real seconds).
    last_sound: HashMap<String, f32>,
    guns: Vec<GunState>,
    /// Where wing and belly guns meet, m.
    convergence: f32,
    /// The loadout and gun cards this was built from.
    built_from: Option<(Loadout, Vec<GunCard>)>,
}

/// Give every plane (yours and the target) the guns its preset carries.
fn sync_player_guns(
    mut commands: Commands,
    settings: Res<Settings>,
    mut planes: Query<(Entity, &Aircraft, Option<&mut Guns>)>,
) {
    for (entity, aircraft, guns) in &mut planes {
        sync_guns(&mut commands, &settings, entity, aircraft, guns);
    }
}

fn sync_guns(
    commands: &mut Commands,
    settings: &Settings,
    entity: Entity,
    aircraft: &Aircraft,
    guns: Option<Mut<Guns>>,
) {
    let loadout = aircraft.card.weapons.clone();
    let key = (loadout.clone(), settings.guns.clone());
    if guns
        .as_ref()
        .is_some_and(|g| g.built_from.as_ref() == Some(&key))
    {
        return;
    }
    let correction = crate::model_correction();
    let built = Guns {
        last_sound: HashMap::new(),
        convergence: if loadout.convergence > 0.0 {
            loadout.convergence.clamp(CONVERGENCE_MIN, CONVERGENCE_MAX)
        } else {
            0.0
        },
        guns: loadout
            .guns()
            .into_iter()
            .map(|(slot, gun, p)| GunState {
                slot,
                muzzle: correction.transform_point(p),
                card: settings.gun(&gun).clone(),
                cooldown: 0.0,
            })
            .collect(),
        built_from: Some(key),
    };
    match guns {
        Some(mut g) => *g = built,
        None => {
            commands.entity(entity).insert(built);
        }
    }
}

/// Default range where wing and belly guns meet; nose guns fire straight
/// ahead.
pub const CONVERGENCE: f32 = 400.0;
/// Limits and step for the convergence setting, m.
pub const CONVERGENCE_MIN: f32 = 100.0;
pub const CONVERGENCE_MAX: f32 = 1500.0;
pub const CONVERGENCE_STEP: f32 = 50.0;

#[derive(Component)]
struct Bullet {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    /// Distance flown, m.
    traveled: f32,
    size: f32,
    drag: f32,
    /// Share of a plane's health one hit takes.
    damage: f32,
    /// Fired by the target at you.
    enemy: bool,
}

#[derive(Component)]
struct Spark {
    age: f32,
}

/// Shots and hits since the game started.
#[derive(Resource, Default)]
pub struct HitStats {
    pub shots: u32,
    pub hits: u32,
    /// `Time<Real>` seconds of the latest hit.
    pub last_hit: f32,
}

#[derive(Default)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        // xorshift32
        let mut x = if self.0 == 0 { 0x9E37_79B9 } else { self.0 };
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn trigger_held(mouse: &ButtonInput<MouseButton>, cursor: &CursorOptions) -> bool {
    // Testing aid: WTAT_FIRE=1 holds the trigger.
    if std::env::var("WTAT_FIRE").is_ok() {
        return true;
    }
    mouse.pressed(MouseButton::Left) && cursor.grab_mode != CursorGrabMode::None
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn fire_guns(
    mut commands: Commands,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut sounds: ResMut<Sounds>,
    mut sources: ResMut<Assets<AudioSource>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions>,
    weapon_assets: Res<WeaponAssets>,
    mut stats: ResMut<HitStats>,
    mut rng: Local<Rng>,
    camera: Single<&GlobalTransform, With<Camera3d>>,
    settings: Res<Settings>,
    planes: Query<(
        &Transform,
        &Aircraft,
        &mut Guns,
        Has<Player>,
        Option<&Target>,
    )>,
) {
    let dt = time.delta_secs();
    let player_trigger = trigger_held(&mouse, &cursor);
    let eye = camera.translation();
    for (tf, aircraft, mut guns, is_player, target) in planes {
        let firing = if is_player {
            player_trigger
        } else {
            settings.enemy_fires && target.is_some_and(|t| t.firing && !t.destroyed())
        };
        // The enemy's guns sound fainter the farther away they are.
        let loudness = if is_player {
            1.0
        } else {
            0.8 / (1.0 + tf.translation.distance(eye) / 150.0)
        };
        fire_plane(
            &mut commands,
            dt,
            real.elapsed_secs(),
            &mut sounds,
            &mut sources,
            &weapon_assets,
            &mut stats,
            &mut rng,
            eye,
            tf,
            aircraft,
            &mut guns,
            firing,
            !is_player,
            loudness,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn fire_plane(
    commands: &mut Commands,
    dt: f32,
    now: f32,
    sounds: &mut Sounds,
    sources: &mut Assets<AudioSource>,
    weapon_assets: &WeaponAssets,
    stats: &mut HitStats,
    rng: &mut Rng,
    eye: Vec3,
    tf: &Transform,
    aircraft: &Aircraft,
    guns: &mut Guns,
    firing: bool,
    enemy: bool,
    loudness: f32,
) {
    let forward = tf.forward().as_vec3();
    let converging = guns.convergence > 0.0;
    let converge_at = tf.translation + forward * guns.convergence;
    let up = tf.up().as_vec3();
    let right = tf.right().as_vec3();
    // Guns of the same kind share one sound per frame (louder for more of
    // them), so a full battery doesn't stack dozens of sounds.
    let mut fired: Vec<(GunCard, u32)> = vec![];
    for gun in &mut guns.guns {
        if !firing {
            // Ready to fire again once the gun has cycled.
            gun.cooldown = (gun.cooldown - dt).max(0.0);
            continue;
        }
        gun.cooldown -= dt;
        while gun.cooldown <= 0.0 {
            let muzzle = tf.transform_point(gun.muzzle);
            let aim = match gun.slot {
                Slot::NoseCenter | Slot::NoseRing => forward,
                Slot::Belly | Slot::Wings if converging => {
                    (converge_at - muzzle).normalize_or(forward)
                }
                Slot::Belly | Slot::Wings => forward,
            };
            // Rounds land evenly over a circle `spread` wide.
            let radius = gun.card.spread_mils() * 0.001 * 0.5;
            let angle = rng.next() * std::f32::consts::PI;
            let r = radius * (rng.next() * 0.5 + 0.5).sqrt();
            let dir = (aim + (right * angle.cos() + up * angle.sin()) * r).normalize();
            let velocity = aircraft.body.velocity + dir * gun.card.muzzle_velocity;
            // A shot due partway through the frame has already flown a bit.
            let early = -gun.cooldown;
            let position = muzzle + velocity * early;
            let size = tracer_size(gun.card.caliber_mm());
            let (core_tf, glow_tf) = tracer_shape(position, velocity, eye, size);
            let (core, glow) = weapon_assets
                .tracer_materials
                .get(&gun.card.tracer)
                .cloned()
                .unwrap_or_default();
            commands.spawn((
                Bullet {
                    position,
                    velocity,
                    age: early,
                    traveled: 0.0,
                    drag: gun.card.drag_factor(),
                    damage: 1.0 / gun.card.hits_to_destroy() as f32,
                    size,
                    enemy,
                },
                Transform::from_translation(position),
                Visibility::default(),
                children![
                    (
                        TracerCore,
                        Mesh3d(weapon_assets.tracer_mesh.clone()),
                        MeshMaterial3d(core),
                        core_tf,
                    ),
                    (
                        TracerGlow,
                        Mesh3d(weapon_assets.glow_mesh.clone()),
                        MeshMaterial3d(glow),
                        glow_tf,
                    )
                ],
            ));
            if !enemy {
                stats.shots += 1;
            }
            gun.cooldown += gun.card.interval();
            match fired.iter_mut().find(|(c, _)| *c == gun.card) {
                Some((_, n)) => *n += 1,
                None => fired.push((gun.card.clone(), 1)),
            }
        }
    }
    for (card, count) in fired {
        let key = card.name.clone();
        let last = guns.last_sound.get(&key).copied().unwrap_or(f32::MIN);
        // Every shot gets its sound, up to 25 a second (faster guns blend
        // together). Shots land on whole frames, so the gap between two can
        // be most of a frame shorter than the gun's interval: only skip a
        // sound that comes far too soon, or slow guns drop shots at random.
        if now - last < (card.interval() * 0.5).max(0.04) {
            continue;
        }
        guns.last_sound.insert(key, now);
        let sound = sounds.gunshot(sources, card.caliber_mm(), card.muzzle_velocity);
        let loud = (card.caliber_mm() / 20.0).clamp(0.4, 1.6).sqrt();
        let volume = 0.22 * loud * (count as f32).powf(0.35) * loudness;
        let speed = 1.0 + rng.next() * 0.015;
        sound::play(commands, sound, volume, speed);
    }
}

/// Tracer thickness for a caliber, relative to a 20 mm round.
fn tracer_size(caliber_mm: f32) -> f32 {
    (caliber_mm / 20.0).clamp(0.3, 3.0)
}

#[derive(Component)]
struct TracerCore;

#[derive(Component)]
struct TracerGlow;

/// How a tracer is drawn, relative to the round's own position: a thin
/// round streak trailing it, true to caliber, and a soft glow just wider
/// than the streak on a card that always faces the camera. Far off, the
/// streak is kept to about one pixel wide so it doesn't vanish; its length
/// never changes.
fn tracer_shape(position: Vec3, velocity: Vec3, camera: Vec3, size: f32) -> (Transform, Transform) {
    let dist = position.distance(camera);
    let width = size.max(dist * MIN_TRACER_ANGLE / TRACER_WIDTH);
    let len = (velocity.length() * 0.003).clamp(0.6, 3.0);
    let dir = velocity.normalize_or(Vec3::NEG_Z);
    let center = -dir * len * 0.5;
    let core = Transform::from_translation(center)
        .looking_to(dir, Vec3::Y)
        .with_scale(Vec3::new(width, width, len));
    // The glow card faces the camera square on, stretched along the
    // streak as it appears on screen.
    let face = (camera - (position + center)).normalize_or(Vec3::Z);
    let along = dir - face * dir.dot(face);
    let shown = along.length();
    let up = if shown > 1e-4 {
        along / shown
    } else {
        face.any_orthonormal_vector()
    };
    let side = up.cross(face);
    let glow_w = width * TRACER_WIDTH * GLOW_WIDTH;
    let glow = Transform {
        translation: center,
        rotation: Quat::from_mat3(&Mat3::from_cols(side, up, face)),
        scale: Vec3::new(glow_w, len * shown + glow_w, 1.0),
    };
    (core, glow)
}

/// The glow's texture: a soft round spot (white, with the shape in alpha),
/// brightest in the middle and fading smoothly to nothing at the edge.
fn glow_image() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const N: usize = 64;
    let mut data = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let u = (x as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / N as f32 * 2.0 - 1.0;
            let d2 = u * u + v * v;
            // A bright, nearly solid middle that softens toward the edge.
            let a = (2.2 * (-d2 * 2.5).exp() * (1.0 - d2).max(0.0)).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Tracer thickness for a 20 mm round, m.
const TRACER_WIDTH: f32 = 0.02;
/// Far off, a streak is kept at least this wide on screen, rad (about a
/// pixel at normal zoom).
const MIN_TRACER_ANGLE: f32 = 0.0012;
/// The glow is this many times wider than the streak.
const GLOW_WIDTH: f32 = 2.0;
/// Strength of the glow around a tracer (0 to 1).
const TRACER_GLOW: f32 = 1.0;

/// The target's pose at the start of the frame, so bullets can be tested
/// against where it actually was during the frame.
#[derive(Component)]
struct PrevPose(Transform);

#[allow(clippy::type_complexity)]
fn record_target_pose(
    mut commands: Commands,
    targets: Query<(Entity, &Transform, Option<&mut PrevPose>), Or<(With<Target>, With<Player>)>>,
) {
    for (e, tf, prev) in targets {
        match prev {
            Some(mut p) => p.0 = *tf,
            None => {
                commands.entity(e).insert(PrevPose(*tf));
            }
        }
    }
}

/// Hit boxes in model space (min, max), from airplane.glb.
const HIT_BOXES: [(Vec3, Vec3); 4] = [
    // Fuselage
    (Vec3::new(-0.34, 0.0, -2.7), Vec3::new(0.34, 1.12, 2.5)),
    // Tail
    (Vec3::new(-0.1, 0.38, -3.92), Vec3::new(0.1, 1.64, -2.7)),
    // Wings
    (Vec3::new(-3.5, 0.0, -1.3), Vec3::new(3.5, 0.5, 1.0)),
    // Tailplane
    (Vec3::new(-1.26, 0.70, -3.7), Vec3::new(1.26, 0.83, -2.74)),
];

/// Where the segment a→b first enters the box, as a fraction of the way.
fn segment_hits_box(a: Vec3, b: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if a[i] < min[i] || a[i] > max[i] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d[i];
        let (mut ta, mut tb) = ((min[i] - a[i]) * inv, (max[i] - a[i]) * inv);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// The world → model space transform for a plane at `pose`.
fn to_model(pose: &Transform) -> Mat4 {
    (pose.to_matrix() * crate::model_correction().to_matrix()).inverse()
}

fn lerp_pose(a: &Transform, b: &Transform, t: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation: a.rotation.slerp(b.rotation, t),
        scale: Vec3::ONE,
    }
}

/// Rounds are removed after flying this far: 2.5 miles.
pub const BULLET_RANGE: f32 = 2.5 * 1609.344;
/// And in any case after this long, s.
const BULLET_LIFETIME: f32 = 15.0;
const BULLET_SUBSTEPS: usize = 4;

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn fly_bullets(
    mut commands: Commands,
    time: Res<Time>,
    real: Res<Time<Real>>,
    weapon_assets: Res<WeaponAssets>,
    mut stats: ResMut<HitStats>,
    sounds: Res<Sounds>,
    mut last_hit_sound: Local<f32>,
    mut bullets: Query<(Entity, &mut Bullet, &mut Transform, &Children), Without<PrevPose>>,
    mut cores: Query<
        &mut Transform,
        (
            With<TracerCore>,
            Without<TracerGlow>,
            Without<Bullet>,
            Without<PrevPose>,
        ),
    >,
    mut glows: Query<
        &mut Transform,
        (
            With<TracerGlow>,
            Without<TracerCore>,
            Without<Bullet>,
            Without<PrevPose>,
        ),
    >,
    planes: Query<(Entity, &Transform, &PrevPose, Option<&Target>, Has<Player>)>,
    mut hit_messages: MessageWriter<TargetHit>,
    mut player_hits: MessageWriter<PlayerHit>,
    scenario: Res<ScenarioState>,
    camera: Single<&GlobalTransform, With<Camera3d>>,
) {
    let eye = camera.translation();
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let h = dt / BULLET_SUBSTEPS as f32;
    // Planes that can be hit: the target (by your rounds) and you (by its).
    let poses: Vec<(Entity, Transform, Transform, bool)> = planes
        .iter()
        .filter(|(_, _, _, t, is_player)| {
            if *is_player {
                !scenario.player_down()
            } else {
                t.is_some_and(|t| !t.destroyed())
            }
        })
        .map(|(e, tf, p, _, is_player)| (e, p.0, *tf, is_player))
        .collect();
    'bullets: for (entity, mut bullet, mut tf, children) in &mut bullets {
        let mut pos = bullet.position;
        for i in 0..BULLET_SUBSTEPS {
            let start = pos;
            let rho = air_density(pos.y.max(0.0));
            let v = bullet.velocity;
            let accel = Vec3::NEG_Y * 9.81 - v * v.length() * rho * bullet.drag;
            bullet.velocity += accel * h;
            pos += bullet.velocity * h;
            bullet.traveled += (pos - start).length();

            for (target, prev, now, is_player) in &poses {
                // Your rounds hit the target; its rounds hit you.
                if bullet.enemy != *is_player {
                    continue;
                }
                let pose_a = lerp_pose(prev, now, i as f32 / BULLET_SUBSTEPS as f32);
                let pose_b = lerp_pose(prev, now, (i + 1) as f32 / BULLET_SUBSTEPS as f32);
                let a = to_model(&pose_a).transform_point3(start);
                let b = to_model(&pose_b).transform_point3(pos);
                let hit = HIT_BOXES
                    .iter()
                    .filter_map(|(min, max)| segment_hits_box(a, b, *min, *max))
                    .reduce(f32::min);
                if let Some(t) = hit {
                    if bullet.enemy {
                        player_hits.write(PlayerHit {
                            damage: bullet.damage,
                        });
                        // A deeper thud when you're the one hit.
                        let now = real.elapsed_secs();
                        if now - *last_hit_sound > 0.065 {
                            *last_hit_sound = now;
                            sound::play(&mut commands, sounds.hit(), 0.6, 0.62);
                        }
                        commands.entity(entity).despawn();
                        continue 'bullets;
                    }
                    stats.hits += 1;
                    hit_messages.write(TargetHit {
                        damage: bullet.damage,
                    });
                    stats.last_hit = real.elapsed_secs();
                    // A hit sound, up to about 15 a second.
                    if stats.last_hit - *last_hit_sound > 0.065 {
                        *last_hit_sound = stats.last_hit;
                        let speed = 0.92 + 0.16 * ((stats.hits * 7919) % 100) as f32 / 100.0;
                        sound::play(&mut commands, sounds.hit(), 0.5, speed);
                    }
                    // Spark where it struck, riding along with the target.
                    let local = crate::model_correction().transform_point(a.lerp(b, t));
                    commands.entity(*target).with_children(|p| {
                        p.spawn((
                            Spark { age: 0.0 },
                            Transform::from_translation(local),
                            Mesh3d(weapon_assets.spark_mesh.clone()),
                            MeshMaterial3d(weapon_assets.spark_material.clone()),
                        ));
                    });
                    commands.entity(entity).despawn();
                    continue 'bullets;
                }
            }
        }
        bullet.position = pos;
        bullet.age += dt;
        if bullet.traveled > BULLET_RANGE || bullet.age > BULLET_LIFETIME || pos.y < 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        tf.translation = pos;
        let (core, glow) = tracer_shape(pos, bullet.velocity, eye, bullet.size);
        for c in children {
            if let Ok(mut t) = cores.get_mut(*c) {
                *t = core;
            } else if let Ok(mut t) = glows.get_mut(*c) {
                *t = glow;
            }
        }
    }
}

/// A scenario restart clears every round in the air.
fn clear_rounds(
    mut commands: Commands,
    mut resets: MessageReader<ResetScenario>,
    bullets: Query<Entity, With<Bullet>>,
) {
    if resets.read().count() == 0 {
        return;
    }
    for e in &bullets {
        commands.entity(e).despawn();
    }
}

const SPARK_TIME: f32 = 0.25;

fn fade_sparks(
    mut commands: Commands,
    time: Res<Time>,
    mut sparks: Query<(Entity, &mut Spark, &mut Transform)>,
) {
    for (e, mut spark, mut tf) in &mut sparks {
        spark.age += time.delta_secs();
        if spark.age > SPARK_TIME {
            commands.entity(e).despawn();
        } else {
            tf.scale = Vec3::splat(1.0 - spark.age / SPARK_TIME * 0.7);
        }
    }
}

#[derive(Component)]
struct HitMarker;

#[derive(Component)]
struct HitCounter;

fn spawn_hit_ui(mut commands: Commands) {
    commands.spawn((
        Text::new("HIT"),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.85, 0.3)),
        TextShadow {
            offset: Vec2::splat(1.5),
            color: Color::srgba(0.0, 0.0, 0.0, 0.7),
        },
        Node {
            position_type: PositionType::Absolute,
            top: percent(36),
            left: percent(50),
            margin: UiRect::left(px(-20)),
            ..default()
        },
        Visibility::Hidden,
        HitMarker,
    ));
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextShadow {
            offset: Vec2::splat(1.5),
            color: Color::srgba(0.0, 0.0, 0.0, 0.7),
        },
        Node {
            position_type: PositionType::Absolute,
            top: px(10),
            right: px(12),
            ..default()
        },
        HitCounter,
    ));
}

fn update_hit_ui(
    real: Res<Time<Real>>,
    stats: Res<HitStats>,
    settings: Res<Settings>,
    scenario: Res<ScenarioState>,
    mut marker: Single<&mut Visibility, With<HitMarker>>,
    mut counter: Single<&mut Text, With<HitCounter>>,
) {
    let show = stats.hits > 0 && real.elapsed_secs() - stats.last_hit < 0.35;
    let vis = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if **marker != vis {
        **marker = vis;
    }
    if stats.is_changed() {
        let accuracy = if stats.shots > 0 {
            100.0 * stats.hits as f32 / stats.shots as f32
        } else {
            0.0
        };
        counter.0 = format!(
            "Hits {} / {} shots ({accuracy:.0}%)",
            stats.hits, stats.shots
        );
    }
    if settings.enemy_fires {
        let line = format!(
            "\nYour damage {:.0}%",
            (scenario.player_damage * 100.0).min(100.0)
        );
        if !counter.0.ends_with(&line) {
            let base = counter.0.split('\n').next().unwrap_or("").to_string();
            counter.0 = base + &line;
        }
    } else if counter.0.contains('\n') {
        counter.0 = counter.0.split('\n').next().unwrap_or("").to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lead_points_ahead_of_a_crossing_target_and_above_for_drop() {
        let gun = GunCard::shvak();
        // Still target 600 m ahead: aim a little high.
        let d = lead_direction(
            &gun,
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -600.0),
            Vec3::ZERO,
        )
        .unwrap();
        assert!(d.y > 0.0 && d.y < 0.02, "{d}");
        // Crossing left to right at 120 m/s: aim to the right of it.
        let d = lead_direction(
            &gun,
            Vec3::ZERO,
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -400.0),
            Vec3::new(120.0, 0.0, 0.0),
        )
        .unwrap();
        let lead_deg = d.x.atan2(-d.z).to_degrees();
        println!("lead {lead_deg:.2} deg");
        assert!(lead_deg > 6.0 && lead_deg < 12.0);
    }

    #[test]
    fn mount_counts_and_symmetry() {
        for n in 0..=6 {
            let p = muzzle_positions(Slot::NoseRing, n);
            assert_eq!(p.len(), n as usize);
            // Left-right symmetric: the x positions sum to zero.
            let sx: f32 = p.iter().map(|v| v.x).sum();
            assert!(sx.abs() < 1e-4, "ring of {n} is lopsided");
        }
        assert_eq!(muzzle_positions(Slot::NoseCenter, 3).len(), 1);
        assert_eq!(muzzle_positions(Slot::Belly, 4).len(), 4);
        assert_eq!(muzzle_positions(Slot::Wings, 4).len(), 8);
        let yak = Loadout::default();
        assert_eq!(yak.total_guns(), 3);
    }

    #[test]
    fn shvak_ballistics_are_plausible() {
        // Fly a ShVAK round 1 km at sea level and check drop and speed.
        let b = ballistics_at(&GunCard::shvak(), 1000.0);
        println!(
            "ShVAK at 1 km: {:.2} s, {:.0} m/s, drop {:.1} m",
            b.time, b.speed, b.drop
        );
        assert!(b.time > 1.3 && b.time < 2.2);
        assert!(b.speed > 300.0 && b.speed < 650.0);
        assert!(b.drop > 6.0 && b.drop < 25.0);
        // A blank caliber is estimated from the mass.
        let est = GunCard {
            caliber: None,
            ..GunCard::ubs()
        };
        assert!((est.caliber_mm() - 15.7).abs() < 0.5);
    }

    fn hits_plane_at(pose: &Transform, a: Vec3, b: Vec3) -> bool {
        let m = to_model(pose);
        let (a, b) = (m.transform_point3(a), m.transform_point3(b));
        HIT_BOXES
            .iter()
            .any(|(min, max)| segment_hits_box(a, b, *min, *max).is_some())
    }

    #[test]
    fn rounds_hit_the_airframe_not_the_air_beside_it() {
        let pose = Transform::from_xyz(0.0, 1000.0, -300.0);
        let shot = |x: f32, y: f32| {
            hits_plane_at(
                &pose,
                Vec3::new(x, 1000.0 + y, -280.0),
                Vec3::new(x, 1000.0 + y, -320.0),
            )
        };
        assert!(shot(0.0, 0.8), "through the fuselage");
        assert!(shot(4.0, 0.4), "through a wing (scaled span 9.3 m)");
        assert!(!shot(5.5, 0.4), "past the wingtip");
        assert!(!shot(0.0, 3.0), "over the top");
        // A plane turned side-on presents its fuselage length instead.
        let side = pose.with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        assert!(hits_plane_at(
            &side,
            Vec3::new(4.0, 1000.8, -280.0),
            Vec3::new(4.0, 1000.8, -320.0)
        ));
    }

    #[test]
    fn segments_hit_boxes() {
        let (min, max) = (Vec3::splat(-1.0), Vec3::splat(1.0));
        let t = segment_hits_box(
            Vec3::new(0.0, 0.0, -3.0),
            Vec3::new(0.0, 0.0, 3.0),
            min,
            max,
        );
        assert!((t.unwrap() - 1.0 / 3.0).abs() < 1e-5);
        assert!(
            segment_hits_box(
                Vec3::new(2.0, 0.0, -3.0),
                Vec3::new(2.0, 0.0, 3.0),
                min,
                max
            )
            .is_none()
        );
        assert!(
            segment_hits_box(
                Vec3::new(0.0, 0.0, -3.0),
                Vec3::new(0.0, 0.0, -2.0),
                min,
                max
            )
            .is_none()
        );
    }
}
