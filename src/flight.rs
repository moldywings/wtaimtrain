//! Player plane, flown by the physics in `flight_model`. Mouse aim drives
//! an instructor (like War Thunder's) that works the stick for you; holding
//! a pitch, roll or yaw key takes over that axis until it's released.

use bevy::prelude::*;

use crate::GameState;
use crate::camera::{ChaseCamera, MouseAim, snap_view};
use crate::flight_model::{self, AircraftCard, Body, Controls, FlightModel};
use crate::settings::{Action, Settings};
use crate::target::Target;

pub struct FlightPlugin;

impl Plugin for FlightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player).add_systems(
            Update,
            (
                apply_presets,
                (fly_player, reset).run_if(in_state(GameState::Playing)),
            )
                .chain(),
        );
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
                throttle: 0.8,
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

#[derive(Component, Default)]
pub struct Player {
    /// Keyboard pitch/roll/yaw, eased toward -1..=1 while keys are held.
    keys: Vec3,
}

pub const START_ALTITUDE: f32 = 1_500.0;
/// Start at this fraction of the preset's top speed at the start altitude.
pub const START_SPEED_FRACTION: f32 = 0.85;
/// How quickly keyboard inputs reach full deflection (per second).
const KEY_RAMP: f32 = 4.0;

fn start_body(model: &FlightModel) -> Body {
    Body::new(
        Vec3::new(0.0, START_ALTITUDE, 0.0),
        Quat::IDENTITY,
        START_SPEED_FRACTION * model.top_speed(START_ALTITUDE),
    )
}

fn spawn_player(mut commands: Commands, assets: Res<AssetServer>, settings: Res<Settings>) {
    let mut aircraft = Aircraft::new(
        settings.player_card(),
        Body::new(Vec3::ZERO, Quat::IDENTITY, 0.0),
    );
    aircraft.body = start_body(&aircraft.model);
    commands
        .spawn((
            Transform::from_translation(aircraft.body.position),
            Visibility::default(),
            Player::default(),
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
}

/// Rebuild flight models when presets change in the menu.
fn apply_presets(
    settings: Res<Settings>,
    mut player: Query<&mut Aircraft, (With<Player>, Without<Target>)>,
    mut targets: Query<&mut Aircraft, (With<Target>, Without<Player>)>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut a in &mut player {
        a.set_card(settings.player_card());
    }
    for mut a in &mut targets {
        a.set_card(settings.target_card());
    }
}

pub fn fly_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    aim: Res<MouseAim>,
    player: Single<(&mut Transform, &mut Aircraft, &mut Player)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let (mut tf, mut aircraft, mut player) = player.into_inner();
    let axis = |plus: Action, minus: Action| -> f32 {
        settings.pressed(&keys, plus) as i32 as f32 - settings.pressed(&keys, minus) as i32 as f32
    };

    let throttle_input = axis(Action::ThrottleUp, Action::ThrottleDown);
    let mut controls = aircraft.controls;
    controls.throttle = (controls.throttle + throttle_input * 0.5 * dt).clamp(0.0, 1.0);

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
    let blend = |auto: f32, key: f32| auto * (1.0 - key.abs()) + key;
    controls.pitch = blend(auto.pitch, player.keys.x);
    controls.roll = blend(auto.roll, player.keys.y);
    controls.yaw = blend(auto.yaw, player.keys.z);
    aircraft.controls = controls;

    let Aircraft { model, body, .. } = &mut *aircraft;
    flight_model::step(model, body, &controls, dt);

    // No crash damage yet: hitting the ground just restarts you.
    if body.position.y < 3.0 {
        *body = start_body(model);
    }

    tf.translation = body.position;
    tf.rotation = body.rotation;
}

/// The reset key puts the player back at the start and the target ahead.
#[allow(clippy::type_complexity)]
fn reset(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    mut aim: ResMut<MouseAim>,
    mut camera: Single<&mut ChaseCamera>,
    mut player: Single<&mut Aircraft, (With<Player>, Without<Target>)>,
    mut targets: Query<(&mut Aircraft, &mut Target), Without<Player>>,
) {
    if !settings.just_pressed(&keys, Action::Reset) {
        return;
    }
    player.body = start_body(&player.model);
    player.controls.throttle = 0.8;
    aim.yaw = 0.0;
    aim.pitch = 0.0;
    snap_view(&mut camera, &aim);
    for (mut aircraft, mut target) in &mut targets {
        target.reset(&mut aircraft);
    }
}
