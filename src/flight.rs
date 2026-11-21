//! Player plane, flown by the physics in `flight_model`. Mouse aim drives
//! an instructor (like War Thunder's) that works the stick for you; holding
//! a pitch, roll or yaw key takes over that axis until it's released.

use std::collections::HashMap;

use bevy::gltf::GltfMaterialName;
use bevy::prelude::*;

use crate::GameState;
use crate::camera::MouseAim;
use crate::flight_model::{self, AircraftCard, Body, Controls, FlightModel};
use crate::multiplayer::{Mp, local_flies, offline};
use crate::settings::{Action, PlaneColor, Settings};
use crate::target::{ResetScenario, Target};

pub struct FlightPlugin;

impl Plugin for FlightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player)
            .add_systems(
                Update,
                (
                    apply_presets,
                    fly_player.run_if(local_flies),
                    reset.run_if(in_state(GameState::Playing)).run_if(offline),
                )
                    .chain(),
            )
            .add_systems(Update, (find_paintwork, paint_planes).chain());
    }
}

/// An aircraft in the world: its flight model, card and moving state.
#[derive(Component)]
pub struct Aircraft {
    pub card: AircraftCard,
    pub model: FlightModel,
    pub body: Body,
    pub controls: Controls,
}

impl Aircraft {
    pub fn new(card: &AircraftCard, body: Body) -> Self {
        Aircraft {
            card: card.clone(),
            model: FlightModel::from_card(card),
            body,
            controls: Controls {
                throttle: 1.0,
                ..default()
            },
        }
    }

    pub fn set_card(&mut self, card: &AircraftCard) {
        if self.card != *card {
            self.card = card.clone();
            self.model = FlightModel::from_card(card);
        }
    }

    pub fn is_stalled(&self) -> bool {
        self.model.is_stalled(self.body.alpha)
    }
}

/// The paint on a plane (not the cockpit). Planes without one keep the
/// model's own red.
#[derive(Component, Clone, Copy, PartialEq)]
pub struct PaintColor(pub PlaneColor);

/// A painted part of a plane's model.
#[derive(Component)]
struct Paintwork {
    plane: Entity,
    original: Handle<StandardMaterial>,
    shown: PlaneColor,
}

/// The model's paint material.
const PAINT_MATERIAL: &str = "Red";

/// Find the painted parts of newly loaded plane models.
fn find_paintwork(
    mut commands: Commands,
    parts: Query<
        (Entity, &GltfMaterialName, &MeshMaterial3d<StandardMaterial>),
        Added<GltfMaterialName>,
    >,
    parents: Query<&ChildOf>,
    painted: Query<(), With<PaintColor>>,
) {
    for (entity, name, material) in &parts {
        if name.0 != PAINT_MATERIAL {
            continue;
        }
        let Some(plane) = parents
            .iter_ancestors(entity)
            .find(|a| painted.contains(*a))
        else {
            continue;
        };
        commands.entity(entity).insert(Paintwork {
            plane,
            original: material.0.clone(),
            shown: PlaneColor::Red,
        });
    }
}

/// Repaint planes whose color changed.
fn paint_planes(
    mut parts: Query<(&mut Paintwork, &mut MeshMaterial3d<StandardMaterial>)>,
    planes: Query<&PaintColor>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<HashMap<PlaneColor, Handle<StandardMaterial>>>,
) {
    for (mut part, mut material) in &mut parts {
        let Ok(paint) = planes.get(part.plane) else {
            continue;
        };
        if part.shown == paint.0 {
            continue;
        }
        let handle = if paint.0 == PlaneColor::Red {
            part.original.clone()
        } else if let Some(h) = cache.get(&paint.0) {
            h.clone()
        } else {
            let Some(base) = materials.get(&part.original).cloned() else {
                continue;
            };
            let h = materials.add(StandardMaterial {
                base_color: paint.0.color(),
                ..base
            });
            cache.insert(paint.0, h.clone());
            h
        };
        material.0 = handle;
        part.shown = paint.0;
    }
}

#[derive(Component, Default)]
pub struct Player {
    /// Keyboard pitch/roll/yaw, eased toward -1..=1 while keys are held.
    keys: Vec3,
}

pub const START_ALTITUDE: f32 = 1_500.0;
/// Start between these fractions of the preset's top speed at the start
/// altitude.
pub const START_SPEED_MIN: f32 = 0.5;
pub const START_SPEED_MAX: f32 = 0.6;
/// How quickly keyboard inputs reach full deflection (per second).
const KEY_RAMP: f32 = 4.0;

/// A body at the start spot flying at `fraction` of top speed.
pub fn start_body(model: &FlightModel, fraction: f32) -> Body {
    Body::new(
        Vec3::new(0.0, START_ALTITUDE, 0.0),
        Quat::IDENTITY,
        fraction * model.top_speed(START_ALTITUDE),
    )
}

fn spawn_player(mut commands: Commands, settings: Res<Settings>) {
    let mut aircraft = Aircraft::new(
        settings.player_card(),
        Body::new(Vec3::ZERO, Quat::IDENTITY, 0.0),
    );
    aircraft.body = start_body(&aircraft.model, 0.55);
    commands.spawn((
        Transform::from_translation(aircraft.body.position),
        Visibility::default(),
        Player::default(),
        PaintColor(settings.plane_color),
        aircraft,
    ));
}

/// Rebuild flight models when presets change in the menu.
fn apply_presets(
    settings: Res<Settings>,
    mp: Res<Mp>,
    mut player: Query<&mut Aircraft, (With<Player>, Without<Target>)>,
    mut targets: Query<&mut Aircraft, (With<Target>, Without<Player>)>,
) {
    if !settings.is_changed() {
        return;
    }
    // Online, the multiplayer code picks your preset (it may be the host's).
    if !mp.online() {
        for mut a in &mut player {
            a.set_card(settings.player_card());
        }
    }
    for mut a in &mut targets {
        a.set_card(settings.target_card());
    }
}

#[allow(clippy::too_many_arguments)]
pub fn fly_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    aim: Res<MouseAim>,
    mut reset: MessageWriter<ResetScenario>,
    mp: Res<Mp>,
    state: Res<State<GameState>>,
    player: Single<(&mut Transform, &mut Aircraft, &mut Player)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let (mut tf, mut aircraft, mut player) = player.into_inner();
    // In the menu (only online does the plane keep flying) the keys are
    // for the menu.
    let playing = *state.get() == GameState::Playing;
    let axis = |plus: Action, minus: Action| -> f32 {
        if !playing {
            return 0.0;
        }
        settings.pressed(&keys, plus) as i32 as f32 - settings.pressed(&keys, minus) as i32 as f32
    };

    let throttle_input = axis(Action::ThrottleUp, Action::ThrottleDown);
    let mut controls = aircraft.controls;
    controls.throttle =
        (controls.throttle + throttle_input * 0.5 * dt).clamp(0.0, aircraft.model.max_throttle());

    // Keys ease in and out so a tap isn't a full-deflection jolt.
    let wanted = Vec3::new(
        axis(Action::PitchUp, Action::PitchDown),
        axis(Action::RollRight, Action::RollLeft),
        axis(Action::YawRight, Action::YawLeft),
    );
    let step = KEY_RAMP * dt;
    let current = player.keys;
    player.keys = current + (wanted - current).clamp(Vec3::splat(-step), Vec3::splat(step));

    // Instructor for whatever the keys aren't doing; keys blend in on top.
    let auto = {
        let Aircraft { model, body, .. } = &mut *aircraft;
        flight_model::instructor(model, body, aim.direction(), dt)
    };
    // The pitch keys take over every axis from the instructor (so it
    // doesn't yaw or roll against you while you pull). Roll and yaw keys
    // only take over their own axis.
    let pitching = player.keys.x.abs();
    let blend = |auto: f32, key: f32| auto * (1.0 - pitching.max(key.abs())) + key;
    controls.pitch = blend(auto.pitch, player.keys.x);
    controls.roll = blend(auto.roll, player.keys.y);
    controls.yaw = blend(auto.yaw, player.keys.z);
    // Like the game's instructor, keyboard pitch stops just short of a
    // stall.
    if pitching > 0.0 {
        let (pull, push) = flight_model::safe_pitch_stick(&aircraft.model, &aircraft.body);
        controls.pitch = controls.pitch.clamp(-push, pull);
    }
    aircraft.controls = controls;

    let Aircraft { model, body, .. } = &mut *aircraft;
    flight_model::step(model, body, &controls, dt);

    // No crash damage yet: hitting the ground just restarts you.
    // (Online, the multiplayer code deals with it.)
    if body.position.y < 3.0 && !mp.online() {
        reset.write(ResetScenario);
    }

    tf.translation = body.position;
    tf.rotation = body.rotation;
}

/// The reset key puts the player back at the start and the target ahead.
/// R starts the scenario over.
fn reset(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    mut reset: MessageWriter<ResetScenario>,
) {
    if settings.just_pressed(&keys, Action::Reset) {
        reset.write(ResetScenario);
    }
}
