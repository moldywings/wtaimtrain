//! The target aircraft and the practice scenarios. The target flies its own
//! preset's flight model, steered by the same instructor as the player.
//! Each scenario places both planes and gives the target a way to fly.
//! The scenario resets when the target is destroyed (after 2 s), when
//! either plane crashes, when the run is over, or when you press R.

use bevy::prelude::*;

use crate::GameState;
use crate::camera::{ChaseCamera, MouseAim, snap_view};
use crate::flight::{Aircraft, Player, START_ALTITUDE, start_body};
use crate::flight_model::{self, Body};
use crate::settings::{Ranges, Scenario, Settings, Span};
use crate::sound::{self, Sounds};
use crate::weapons::HitStats;

pub struct TargetPlugin;

impl Plugin for TargetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ResetScenario>()
            .add_message::<TargetHit>()
            .add_message::<PlayerHit>()
            .init_resource::<ScenarioState>()
            .add_systems(Startup, (spawn_target, spawn_notice))
            .add_systems(
                Update,
                (
                    restart_on_change,
                    reset_scenario,
                    fly_target.run_if(in_state(GameState::Playing)),
                )
                    .chain()
                    .after(crate::flight::fly_player)
                    .run_if(crate::multiplayer::offline),
            )
            .add_systems(
                Update,
                (
                    take_hits,
                    take_player_hits,
                    end_of_run,
                    fade_explosions,
                    update_notice,
                )
                    .chain()
                    .after(fly_target)
                    .run_if(in_state(GameState::Playing))
                    .run_if(crate::multiplayer::offline),
            );
    }
}

/// Start the scenario over: place both planes again.
#[derive(Message, Default)]
pub struct ResetScenario;

/// A round hit the target, taking away this fraction of its health.
#[derive(Message)]
pub struct TargetHit {
    pub damage: f32,
}

/// One of the target's rounds hit you, taking away this fraction of your
/// health.
#[derive(Message)]
pub struct PlayerHit {
    pub damage: f32,
}

/// What the target is doing.
#[derive(Clone, Copy, Debug)]
enum Behavior {
    /// Straight along a direction.
    Straight { dir: Vec3 },
    /// Keep a heading, weaving and dodging. `intensity` 0 flies straight.
    Weave {
        dir: Vec3,
        altitude: f32,
        intensity: f32,
    },
    /// Pull straight up, dodging and rolling.
    Climb { intensity: f32 },
    /// Fly at the player and try to hit them.
    Ram,
    /// Get on the player's tail and shoot them down.
    Attack,
    /// A hard banked turn: `turn` is the aim's yaw offset (rad, + right),
    /// `climb` the flight path angle to hold (rad). Turns through `left`
    /// more radians of heading, then rolls out and flies straight.
    Bank {
        turn: f32,
        climb: f32,
        left: f32,
        last: Vec3,
    },
    /// Fly along unaware (or watching), then dive away and run once
    /// alerted. `style`: 0 jinks, 1 rolls, 2 corkscrews.
    Evade {
        aware: bool,
        alerted: bool,
        style: u8,
        intensity: f32,
        dir: Vec3,
        phase: f32,
    },
}

#[derive(Component)]
pub struct Target {
    behavior: Behavior,
    /// Hold this airspeed, m/s (full throttle when `None`).
    hold_speed: Option<f32>,
    /// Current dodge: yaw and pitch offsets from the base direction, rad.
    dodge: Vec2,
    /// Current extra roll input while dodging.
    roll: f32,
    /// Seconds until the next dodge.
    next_dodge: f32,
    /// Fraction of the target destroyed (1 = gone).
    damage: f32,
    /// Its guns are on target and it's shooting (when enemy fire is on).
    pub firing: bool,
    /// Seconds left of breaking away after a close pass.
    break_off: f32,
}

impl Target {
    fn new(behavior: Behavior) -> Self {
        Target {
            behavior,
            hold_speed: None,
            dodge: Vec2::ZERO,
            roll: 0.0,
            next_dodge: 0.0,
            damage: 0.0,
            firing: false,
            break_off: 0.0,
        }
    }

    pub fn destroyed(&self) -> bool {
        self.damage >= 1.0
    }
}

/// The scenario being flown now and how it's going.
#[derive(Resource)]
pub struct ScenarioState {
    /// The scenario in play (for "all in one", the one picked this time).
    pub current: Scenario,
    /// Seconds since the reset.
    pub time: f32,
    /// Seconds since the target was destroyed (or you were shot down).
    destroyed_for: Option<f32>,
    /// Fraction of your plane destroyed by the target's fire (1 = down).
    pub player_damage: f32,
    /// Your shot count when the run started (to tell when you open fire).
    shots_at_start: u32,
    rng: Rng,
    /// What to show in the middle of the screen, and for how long.
    notice: Option<(String, Color, f32)>,
}

impl Default for ScenarioState {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ d.as_secs() as u32)
            .unwrap_or(1);
        // Testing aid: WTAT_SEED fixes the random runs.
        let seed = std::env::var("WTAT_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(seed);
        ScenarioState {
            current: Scenario::Chase,
            time: 0.0,
            destroyed_for: None,
            player_damage: 0.0,
            shots_at_start: 0,
            rng: Rng(seed | 1),
            notice: None,
        }
    }
}

impl ScenarioState {
    pub fn player_down(&self) -> bool {
        self.player_damage >= 1.0
    }
}

/// Small xorshift random numbers.
struct Rng(u32);

impl Rng {
    /// 0..1
    fn unit(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x as f32 / u32::MAX as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// -1..1
    fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.unit() * items.len() as f32) as usize).min(items.len() - 1)]
    }
}

/// How hard the target dodges, picked at random each run: sometimes not at
/// all, sometimes a lot.
const INTENSITIES: [f32; 4] = [0.0, 0.35, 0.65, 1.0];

/// Seconds the "destroyed" notice shows before the scenario resets.
const RESET_AFTER_KILL: f32 = 2.0;

/// A run ends when the target gets this far away, m.
const LOST_RANGE: f32 = 4_000.0;

/// Planes closer than this have collided, m.
const COLLISION: f32 = 9.0;

fn heading(dir: Vec3) -> Quat {
    Transform::default().looking_to(dir, Vec3::Y).rotation
}

fn spawn_target(
    mut commands: Commands,
    assets: Res<AssetServer>,
    settings: Res<Settings>,
    mut reset: MessageWriter<ResetScenario>,
) {
    let aircraft = Aircraft::new(
        settings.target_card(),
        Body::new(
            Vec3::new(0.0, START_ALTITUDE, -1_000.0),
            Quat::IDENTITY,
            100.0,
        ),
    );
    commands
        .spawn((
            Transform::from_translation(aircraft.body.position),
            Visibility::default(),
            Target::new(Behavior::Straight { dir: Vec3::NEG_Z }),
            aircraft,
        ))
        .with_children(|plane| {
            plane.spawn((
                WorldAssetRoot(
                    assets.load(GltfAssetLabel::Scene(0).from_asset(crate::AIRPLANE_MODEL)),
                ),
                crate::model_correction(),
                crate::weapons::PlaneModel,
            ));
        });
    reset.write(ResetScenario);
}

/// Start over when the scenario or its settings change in the menu.
fn restart_on_change(
    settings: Res<Settings>,
    mut last: Local<Option<(Scenario, Ranges, bool)>>,
    mut reset: MessageWriter<ResetScenario>,
) {
    if !settings.is_changed() {
        return;
    }
    let now = (settings.scenario, settings.ranges, settings.enemy_fires);
    if last.is_some_and(|l| l != now) {
        reset.write(ResetScenario);
    }
    *last = Some(now);
}

/// Testing aid: `WTAT_SCENARIO=headon|chase|stall|dogfight` overrides the
/// saved scenario.
fn scenario_override() -> Option<Scenario> {
    match std::env::var("WTAT_SCENARIO").ok()?.as_str() {
        "headon" => Some(Scenario::HeadOn),
        "chase" => Some(Scenario::Chase),
        "stall" => Some(Scenario::Stall),
        "dogfight" => Some(Scenario::Dogfight),
        "banking" => Some(Scenario::Banking),
        "diving" => Some(Scenario::Diving),
        _ => None,
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn reset_scenario(
    mut messages: MessageReader<ResetScenario>,
    settings: Res<Settings>,
    mut state: ResMut<ScenarioState>,
    mut aim: ResMut<MouseAim>,
    mut camera: Single<&mut ChaseCamera>,
    player: Single<(&mut Aircraft, &mut Transform), (With<Player>, Without<Target>)>,
    targets: Query<(&mut Aircraft, &mut Target, &mut Transform, &mut Visibility), Without<Player>>,
    shots: Res<HitStats>,
) {
    if messages.read().count() == 0 {
        return;
    }
    let (mut player, mut player_tf) = player.into_inner();
    let fraction = state.rng.range(
        crate::flight::START_SPEED_MIN,
        crate::flight::START_SPEED_MAX,
    );
    player.body = start_body(&player.model, fraction);
    player.controls.throttle = 1.0;
    *player_tf = Transform::from_translation(player.body.position);
    aim.reset();
    snap_view(&mut camera, &aim);

    let mut scenario = scenario_override().unwrap_or(settings.scenario);
    if scenario == Scenario::AllInOne {
        scenario = state.rng.pick(&[
            Scenario::HeadOn,
            Scenario::Chase,
            Scenario::Stall,
            Scenario::Dogfight,
            Scenario::Banking,
            Scenario::Diving,
        ]);
    }
    state.current = scenario;
    state.time = 0.0;
    state.destroyed_for = None;
    state.player_damage = 0.0;
    state.shots_at_start = shots.shots;

    let ranges = settings.ranges;
    // Diving: you start this far above the target.
    let dive_height = if scenario == Scenario::Diving {
        pick(&mut state.rng, ranges.diving_height)
    } else {
        0.0
    };
    player.body.position.y += dive_height;
    player_tf.translation = player.body.position;
    let p = player.body.position;
    let player_speed = player.body.speed();
    for (mut aircraft, mut target, mut tf, mut vis) in targets {
        *vis = Visibility::Inherited;
        let rng = &mut state.rng;
        let model = &aircraft.model;
        let (target_new, body) = if std::env::var("WTAT_EASY_TARGET").is_ok() {
            // Testing aid: 250 m dead ahead, flying straight.
            let speed = player_speed;
            let mut t = Target::new(Behavior::Straight { dir: Vec3::NEG_Z });
            t.hold_speed = Some(speed);
            (
                t,
                Body::new(p + Vec3::new(0.0, 0.0, -250.0), Quat::IDENTITY, speed),
            )
        } else {
            match scenario {
                Scenario::HeadOn => {
                    let dist = pick(rng, ranges.head_on);
                    let alt = rng.range(30.0, 250.0) * rng.signed().signum();
                    let pos = p + Vec3::new(rng.signed() * 150.0, alt, -dist);
                    let speed = model.top_speed(pos.y) * rng.range(0.55, 1.0);
                    // Fly at a point near where you'll be when you meet.
                    let meet = dist / (speed + player_speed);
                    let aim_at = p
                        + Vec3::NEG_Z * player_speed * meet
                        + Vec3::new(rng.signed() * 80.0, rng.signed() * 60.0, 0.0);
                    let dir = (aim_at - pos).normalize();
                    let mut t = Target::new(Behavior::Straight { dir });
                    t.hold_speed = Some(speed);
                    (t, Body::new(pos, heading(dir), speed))
                }
                Scenario::Chase => {
                    let dist = pick(rng, ranges.chase);
                    let pos = p + Vec3::new(
                        rng.signed() * 0.08 * dist,
                        rng.signed() * 0.04 * dist,
                        -dist,
                    );
                    // A bit slower than you, so you can close in.
                    let speed =
                        (player_speed * rng.range(0.85, 0.95)).min(model.top_speed(pos.y) * 0.95);
                    let mut t = Target::new(Behavior::Weave {
                        dir: Vec3::NEG_Z,
                        altitude: pos.y,
                        intensity: rng.pick(&INTENSITIES),
                    });
                    t.hold_speed = Some(speed);
                    (t, Body::new(pos, Quat::IDENTITY, speed))
                }
                Scenario::Stall => {
                    let pos = p + Vec3::new(
                        rng.signed() * 30.0,
                        rng.range(40.0, 120.0),
                        -pick(rng, ranges.stall),
                    );
                    let t = Target::new(Behavior::Climb {
                        intensity: rng.pick(&INTENSITIES),
                    });
                    (t, Body::new(pos, Quat::IDENTITY, player_speed * 0.95))
                }
                Scenario::Dogfight => {
                    // Mostly ahead of you, in view; now and then anywhere,
                    // including behind.
                    let ahead = rng.unit() < 0.85;
                    let bearing = if ahead {
                        rng.signed() * 35f32.to_radians()
                    } else {
                        rng.range(0.0, std::f32::consts::TAU)
                    };
                    let dist = pick(rng, ranges.dogfight);
                    let pos = p + Vec3::new(
                        bearing.sin() * dist,
                        rng.signed() * 250.0,
                        -bearing.cos() * dist,
                    );
                    // Any heading at all.
                    let course = rng.range(0.0, std::f32::consts::TAU);
                    let dir = Vec3::new(course.sin(), 0.0, -course.cos());
                    let speed = model.top_speed(pos.y) * rng.range(0.45, 1.0);
                    // With enemy fire on it tries to shoot you down;
                    // otherwise it tries to ram you.
                    let behavior = if settings.enemy_fires {
                        Behavior::Attack
                    } else {
                        Behavior::Ram
                    };
                    (Target::new(behavior), Body::new(pos, heading(dir), speed))
                }
                Scenario::Banking => {
                    let dist = pick(rng, ranges.banking);
                    // Which side of the screen it starts on (-1 left, +1 right).
                    let side = rng.signed().signum();
                    // Path: low to high, straight across, or high to low.
                    let path = rng.pick(&[-1.0f32, 0.0, 1.0]);
                    let (rise, climb) = if path == 0.0 {
                        (rng.signed() * 0.03, rng.signed() * 3f32.to_radians())
                    } else {
                        (
                            -path * rng.range(0.05, 0.18),
                            path * rng.range(8.0, 22.0).to_radians(),
                        )
                    };
                    let pos =
                        p + Vec3::new(side * rng.range(0.08, 0.35) * dist, rise * dist, -dist);
                    // Mostly turns across your nose, sometimes away.
                    let toward = rng.unit() < 0.8;
                    let sign = if toward { -side } else { side };
                    // Starts pointing a little away from the turn.
                    let course = -sign * rng.range(-10.0, 35.0).to_radians();
                    let dir = Vec3::new(course.sin(), 0.0, -course.cos());
                    // About your speed, so it stays ahead while you follow.
                    let speed =
                        (player_speed * rng.range(0.95, 1.1)).min(model.top_speed(pos.y) * 0.97);
                    let mut t = Target::new(Behavior::Bank {
                        turn: sign * rng.range(0.4, 0.8),
                        climb,
                        left: rng.range(150.0, 300.0).to_radians(),
                        last: dir,
                    });
                    t.hold_speed = Some(speed);
                    (t, Body::new(pos, heading(dir), speed))
                }
                Scenario::Diving => {
                    let dist = pick(rng, ranges.diving);
                    let bearing = rng.signed() * 25f32.to_radians();
                    let pos =
                        p + Vec3::new(bearing.sin() * dist, -dive_height, -bearing.cos() * dist);
                    // Any heading; it cruises along until it notices you.
                    let course = rng.range(0.0, std::f32::consts::TAU);
                    let dir = Vec3::new(course.sin(), 0.0, -course.cos());
                    let speed = model.top_speed(pos.y) * rng.range(0.6, 0.85);
                    let mut t = Target::new(Behavior::Evade {
                        aware: rng.unit() < 0.5,
                        alerted: false,
                        style: rng.pick(&[0u8, 1, 2]),
                        intensity: rng.pick(&INTENSITIES),
                        dir,
                        phase: 0.0,
                    });
                    t.hold_speed = Some(speed);
                    (t, Body::new(pos, heading(dir), speed))
                }
                Scenario::AllInOne => unreachable!("picked above"),
            }
        };
        *target = target_new;
        aircraft.body = body;
        aircraft.controls.throttle = 1.0;
        tf.translation = aircraft.body.position;
        tf.rotation = aircraft.body.rotation;
    }
}

/// A random value in a range.
fn pick(rng: &mut Rng, span: Span) -> f32 {
    rng.range(span.min.min(span.max), span.max.max(span.min))
}

/// Rotate `dir` by a yaw and pitch offset (rad).
fn offset_dir(dir: Vec3, yaw: f32, pitch: f32) -> Vec3 {
    let right = dir.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(dir).normalize_or(Vec3::Y);
    (Quat::from_axis_angle(up, -yaw) * Quat::from_axis_angle(right, pitch) * dir).normalize()
}

pub fn fly_target(
    time: Res<Time>,
    settings: Res<Settings>,
    mut state: ResMut<ScenarioState>,
    player: Single<&Aircraft, (With<Player>, Without<Target>)>,
    mut targets: Query<(&mut Transform, &mut Aircraft, &mut Target), Without<Player>>,
    shots: Res<HitStats>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    state.time += dt;
    let player = &player.body;
    for (mut tf, mut aircraft, mut target) in &mut targets {
        if target.destroyed() {
            continue;
        }
        target.next_dodge -= dt;
        target.break_off = (target.break_off - dt).max(0.0);
        let new_dodge = target.next_dodge <= 0.0;
        let pos = aircraft.body.position;
        let rel = player.position - pos;
        let range = rel.length();
        // Where its biggest gun has to point to hit you.
        let gun = aircraft
            .card
            .weapons
            .guns()
            .iter()
            .map(|(_, g, _)| settings.gun(g))
            .max_by(|a, b| a.caliber_mm().total_cmp(&b.caliber_mm()))
            .cloned();
        let lead = gun.as_ref().and_then(|g| {
            crate::weapons::lead_direction(
                g,
                pos,
                aircraft.body.velocity,
                player.position,
                player.velocity,
            )
        });
        let aim = match target.behavior {
            Behavior::Straight { dir } => dir,
            Behavior::Weave {
                dir,
                altitude,
                intensity,
            } => {
                if new_dodge {
                    let rng = &mut state.rng;
                    target.dodge = Vec2::new(
                        rng.signed() * intensity * 40f32.to_radians(),
                        rng.signed() * intensity * 18f32.to_radians(),
                    );
                    target.next_dodge = rng.range(0.8, 3.0) * (1.3 - intensity * 0.6);
                }
                // Drift back to the starting height between dodges.
                let hold = ((altitude - pos.y) * 0.003).clamp(-0.25, 0.25);
                offset_dir(dir, target.dodge.x, target.dodge.y + hold)
            }
            Behavior::Climb { intensity } => {
                if new_dodge {
                    let rng = &mut state.rng;
                    target.dodge = Vec2::new(
                        rng.signed() * intensity * 25f32.to_radians(),
                        rng.signed() * intensity * 15f32.to_radians(),
                    );
                    target.roll = rng.signed() * intensity * 0.7;
                    target.next_dodge = rng.range(0.6, 2.0);
                }
                // Straight up, leaning a little toward the dodge.
                let up = Vec3::new(target.dodge.x.sin(), 1.0, target.dodge.y.sin());
                up.normalize()
            }
            Behavior::Ram => {
                // Lead the player: aim where they'll be on arrival.
                let rel = player.position - pos;
                let closing = aircraft.body.speed().max(60.0);
                let t = (rel.length() / closing).min(6.0);
                let at = player.position + player.velocity * t * 0.8;
                (at - pos).normalize_or(Vec3::NEG_Z)
            }
            Behavior::Attack => {
                // Too close and closing: break away, then come back round.
                let closing = (player.velocity - aircraft.body.velocity).dot(rel) < 0.0;
                if range < 150.0 && closing && target.break_off <= 0.0 {
                    target.break_off = 1.8;
                }
                if target.break_off > 0.0 {
                    // Break off to the side and up, so it can turn back in.
                    let away = (-rel).with_y(0.0).normalize_or(Vec3::X);
                    let side = away.cross(Vec3::Y);
                    (away + side * 0.8 + Vec3::Y * 0.4).normalize()
                } else {
                    lead.unwrap_or_else(|| rel.normalize_or(Vec3::NEG_Z))
                }
            }
            Behavior::Evade {
                aware,
                alerted,
                style,
                intensity,
                dir,
                phase,
            } => {
                let mut alerted = alerted;
                if !alerted {
                    // Alerted by your fire, or (if it's watching) by you
                    // diving at it.
                    let fired = shots.shots > state.shots_at_start;
                    let to_it = (pos - player.position).normalize_or_zero();
                    let pv = player.velocity.normalize_or_zero();
                    let diving_at = pv.y < -0.17 && pv.dot(to_it) > 0.85;
                    alerted = fired || (aware && (diving_at || range < 500.0));
                    if alerted {
                        // Full throttle from here on.
                        target.hold_speed = None;
                        target.next_dodge = 0.0;
                        target.roll = 0.0;
                    }
                }
                let phase = if alerted { phase + dt } else { phase };
                target.behavior = Behavior::Evade {
                    aware,
                    alerted,
                    style,
                    intensity,
                    dir,
                    phase,
                };
                if !alerted {
                    dir
                } else {
                    // Dive away from you and run, leveling off near the ground.
                    let away = (pos - player.position).with_y(0.0).normalize_or(dir);
                    let dive = if pos.y > 700.0 {
                        -(0.3 + 0.3 * intensity)
                    } else if pos.y > 350.0 {
                        -0.1
                    } else {
                        0.08
                    };
                    let (yaw, pitch) = match style {
                        // Corkscrew: the nose circles round the escape line.
                        2 => {
                            let a = (0.15 + 0.35 * intensity) * 0.5;
                            let w = 1.4 + intensity;
                            (a * (phase * w).sin(), a * (phase * w).cos())
                        }
                        // Jinks (and rolls, which also jink a little).
                        _ => {
                            if new_dodge {
                                let rng = &mut state.rng;
                                let k = if style == 1 { 0.4 } else { 1.0 };
                                target.dodge = Vec2::new(
                                    rng.signed() * intensity * 35f32.to_radians() * k,
                                    rng.signed() * intensity * 15f32.to_radians() * k,
                                );
                                if style == 1 {
                                    target.roll = rng.signed().signum() * (0.4 + 0.6 * intensity);
                                }
                                target.next_dodge = rng.range(0.8, 2.5) * (1.3 - intensity * 0.6);
                            }
                            (target.dodge.x, target.dodge.y)
                        }
                    };
                    offset_dir(away, yaw, dive + pitch)
                }
            }
            Behavior::Bank {
                turn,
                climb,
                left,
                last,
            } => {
                // Count how far it has turned since the last frame.
                let flat = aircraft.body.velocity.with_y(0.0).normalize_or(last);
                let left = (left - last.angle_between(flat)).max(0.0);
                target.behavior = Behavior::Bank {
                    turn,
                    climb,
                    left,
                    last: flat,
                };
                // Pull out of a dive well above the ground.
                let climb = if pos.y < 300.0 {
                    climb.max(5f32.to_radians())
                } else {
                    climb
                };
                if left > 0.0 {
                    offset_dir(flat, turn, climb)
                } else {
                    offset_dir(flat, 0.0, 0.0)
                }
            }
        };

        // Shoot when its guns are roughly on you and you're in range: like
        // a real pilot, it opens fire a little before the solution is perfect.
        let forward = aircraft.body.rotation * Vec3::NEG_Z;
        let cone = (12.0 / range.max(1.0)).atan().max(2.5f32.to_radians());
        target.firing = settings.enemy_fires
            && !state.player_down()
            && range < 900.0
            && lead.is_some_and(|d| d.angle_between(forward) < cone);

        let hold_speed = target.hold_speed;
        let roll = target.roll;
        let extra_roll = match target.behavior {
            Behavior::Climb { .. } => true,
            Behavior::Evade { alerted, style, .. } => alerted && style == 1,
            _ => false,
        };
        let Aircraft {
            model,
            body,
            controls,
            ..
        } = &mut *aircraft;
        *controls = flight_model::instructor(model, body, aim, dt);
        if extra_roll {
            controls.roll = (controls.roll + roll).clamp(-1.0, 1.0);
        }
        controls.throttle = match hold_speed {
            Some(v) => (0.6 + (v - body.speed()) * 0.05).clamp(0.0, 1.0),
            None => 1.0,
        };
        flight_model::step(model, body, controls, dt);
        tf.translation = body.position;
        tf.rotation = body.rotation;
    }
}

/// Count the target's hits on you; enough and you're shot down.
fn take_player_hits(mut hits: MessageReader<PlayerHit>, mut state: ResMut<ScenarioState>) {
    let damage: f32 = hits.read().map(|h| h.damage).sum();
    if damage <= 0.0 || state.player_down() {
        return;
    }
    state.player_damage += damage;
    if state.player_down() && state.destroyed_for.is_none() {
        state.destroyed_for = Some(0.0);
        state.notice = Some((
            "YOU WERE SHOT DOWN".into(),
            Color::srgb(1.0, 0.3, 0.25),
            RESET_AFTER_KILL,
        ));
    }
}

/// Count hits; when the target has taken enough, it's destroyed.
#[allow(clippy::too_many_arguments)]
fn take_hits(
    mut commands: Commands,
    mut hits: MessageReader<TargetHit>,
    mut state: ResMut<ScenarioState>,
    sounds: Res<Sounds>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    targets: Query<(&mut Target, &Transform, &mut Visibility)>,
    mut sphere: Local<Option<Handle<Mesh>>>,
) {
    let damage: f32 = hits.read().map(|h| h.damage).sum();
    if damage <= 0.0 {
        return;
    }
    for (mut target, tf, mut vis) in targets {
        if target.destroyed() {
            continue;
        }
        target.damage += damage;
        if target.destroyed() {
            *vis = Visibility::Hidden;
            state.destroyed_for = Some(0.0);
            state.notice = Some((
                "TARGET DESTROYED".into(),
                Color::srgb(1.0, 0.82, 0.3),
                RESET_AFTER_KILL,
            ));
            sound::play(&mut commands, sounds.kill(), 0.3, 1.0);
            commands.spawn((
                Explosion { age: 0.0 },
                Transform::from_translation(tf.translation),
                Mesh3d(
                    sphere
                        .get_or_insert_with(|| meshes.add(Sphere::new(1.0)))
                        .clone(),
                ),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 0.8, 0.35, 0.95),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                })),
            ));
        }
    }
}

#[derive(Component)]
struct Explosion {
    age: f32,
}

const EXPLOSION_TIME: f32 = 0.9;

fn fade_explosions(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(
        Entity,
        &mut Explosion,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    for (e, mut ex, mut tf, mat) in &mut q {
        ex.age += time.delta_secs();
        let k = ex.age / EXPLOSION_TIME;
        if k >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        tf.scale = Vec3::splat(3.0 + 14.0 * k.sqrt());
        if let Some(mut m) = materials.get_mut(&mat.0) {
            m.base_color = Color::srgba(
                1.0,
                0.8 - 0.5 * k,
                0.35 - 0.3 * k,
                0.95 * (1.0 - k).powf(1.5),
            );
        }
    }
}

/// Reset after a kill, a crash, a collision, or when the run is over.
fn end_of_run(
    time: Res<Time>,
    mut state: ResMut<ScenarioState>,
    player: Single<&Aircraft, (With<Player>, Without<Target>)>,
    targets: Query<(&Aircraft, &Target), Without<Player>>,
    mut reset: MessageWriter<ResetScenario>,
) {
    let dt = time.delta_secs();
    if let Some(t) = state.destroyed_for.as_mut() {
        *t += dt;
        if *t >= RESET_AFTER_KILL {
            reset.write(ResetScenario);
        }
        return;
    }
    let p = &player.body;
    for (aircraft, target) in &targets {
        if target.destroyed() {
            continue;
        }
        let pos = aircraft.body.position;
        let rel = pos - p.position;
        let range = rel.length();
        let mut end = None;
        if range < COLLISION {
            end = Some(("RAMMED", Color::srgb(1.0, 0.3, 0.25)));
        } else if pos.y < 3.0 {
            end = Some(("TARGET CRASHED", Color::srgb(0.9, 0.9, 0.9)));
        } else if range > LOST_RANGE {
            end = Some(("TARGET LOST", Color::srgb(0.9, 0.9, 0.9)));
        } else {
            let behind = rel.dot(p.velocity) < 0.0;
            let over = match state.current {
                // Once you've passed each other.
                Scenario::HeadOn => behind && range > 600.0,
                // Once the target has stalled and fallen away.
                Scenario::Stall => state.time > 25.0,
                // Once it has finished its turn and flown on a while.
                Scenario::Banking => state.time > 22.0,
                Scenario::Diving => state.time > 45.0,
                _ => false,
            };
            if over {
                end = Some(("MISSED", Color::srgb(0.9, 0.9, 0.9)));
            }
        }
        if let Some((text, color)) = end {
            state.notice = Some((text.into(), color, 1.5));
            reset.write(ResetScenario);
        }
    }
}

#[derive(Component)]
struct Notice;

fn spawn_notice(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(34.0),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow {
            offset: Vec2::splat(2.0),
            color: Color::srgba(0.0, 0.0, 0.0, 0.75),
        },
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: percent(24),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Visibility::Hidden,
        Notice,
    ));
}

fn update_notice(
    time: Res<Time>,
    mut state: ResMut<ScenarioState>,
    notice: Single<(&mut Text, &mut TextColor, &mut Visibility), With<Notice>>,
) {
    let (mut text, mut color, mut vis) = notice.into_inner();
    let Some((msg, c, left)) = state.notice.as_mut() else {
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
        return;
    };
    *left -= time.delta_secs();
    if *left <= 0.0 {
        state.notice = None;
        *vis = Visibility::Hidden;
        return;
    }
    if text.0 != *msg {
        text.0 = msg.clone();
    }
    // Fade out over the last half second.
    color.0 = c.with_alpha((*left / 0.5).min(1.0));
    *vis = Visibility::Inherited;
}
