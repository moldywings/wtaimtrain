//! Target aircraft. It flies its own preset's flight model, steered by the
//! same instructor as the player: for now a steady turn at a set altitude.
//! Scenario modes with varied speeds and approaches come later.

use bevy::prelude::*;

use crate::GameState;
use crate::flight::{Aircraft, START_ALTITUDE};
use crate::flight_model::{self, Body};
use crate::settings::Settings;

pub struct TargetPlugin;

impl Plugin for TargetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_target)
            .add_systems(Update, fly_target.run_if(in_state(GameState::Playing)));
    }
}

#[derive(Component)]
pub struct Target {
    /// Where the autopilot is pointing, radians.
    heading: f32,
    /// Turn rate, rad/s (positive turns left).
    turn_rate: f32,
    altitude: f32,
    throttle: f32,
}

impl Default for Target {
    fn default() -> Self {
        Target {
            heading: 0.0,
            turn_rate: 8f32.to_radians(),
            altitude: START_ALTITUDE + 100.0,
            throttle: 0.85,
        }
    }
}

/// Starts 1.2 km ahead of the player's start point, flying the same way.
const START_AHEAD: f32 = 1_200.0;

impl Target {
    pub fn reset(&mut self, aircraft: &mut Aircraft) {
        *self = Target::default();
        aircraft.body = Body::new(
            Vec3::new(0.0, self.altitude, -START_AHEAD),
            Quat::IDENTITY,
            115.0,
        );
    }
}

fn spawn_target(mut commands: Commands, assets: Res<AssetServer>, settings: Res<Settings>) {
    let mut target = Target::default();
    let mut aircraft = Aircraft::new(
        settings.target_card(),
        Body::new(Vec3::ZERO, Quat::IDENTITY, 0.0),
    );
    target.reset(&mut aircraft);
    commands
        .spawn((
            Transform::from_translation(aircraft.body.position),
            Visibility::default(),
            target,
            aircraft,
        ))
        .with_children(|plane| {
            plane.spawn((
                WorldAssetRoot(
                    assets.load(GltfAssetLabel::Scene(0).from_asset(crate::AIRPLANE_MODEL)),
                ),
                crate::model_correction(),
            ));
        });
}

fn fly_target(time: Res<Time>, mut targets: Query<(&mut Transform, &mut Aircraft, &mut Target)>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut tf, mut aircraft, mut target) in &mut targets {
        target.heading += target.turn_rate * dt;
        let climb = ((target.altitude - aircraft.body.position.y) * 0.003).clamp(-0.25, 0.25);
        let aim = Quat::from_euler(EulerRot::YXZ, target.heading, climb, 0.0) * Vec3::NEG_Z;

        let Aircraft {
            model,
            body,
            controls,
            ..
        } = &mut *aircraft;
        *controls = flight_model::instructor(model, body, aim);
        controls.throttle = target.throttle;
        flight_model::step(model, body, controls, dt);

        if body.position.y < 3.0 {
            target.reset(&mut aircraft);
        }
        tf.translation = aircraft.body.position;
        tf.rotation = aircraft.body.rotation;
    }
}
