//! Target aircraft. For now it flies a steady banked circle ahead of the
//! player; scenario modes with varied speeds and approaches come in step 3.

use bevy::prelude::*;

pub struct TargetPlugin;

impl Plugin for TargetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_target)
            .add_systems(Update, fly_target);
    }
}

#[derive(Component)]
pub struct Target {
    pub center: Vec3,
    pub radius: f32,
    pub speed: f32,
    /// Current angle around the circle, radians.
    pub angle: f32,
}

impl Target {
    /// Position and velocity at the current angle (counter-clockwise from above).
    fn state(&self) -> (Vec3, Vec3) {
        let (s, c) = self.angle.sin_cos();
        let pos = self.center + Vec3::new(c, 0.0, -s) * self.radius;
        let vel = Vec3::new(-s, 0.0, -c) * self.speed;
        (pos, vel)
    }
}

fn spawn_target(mut commands: Commands, assets: Res<AssetServer>) {
    let target = Target {
        center: Vec3::new(0.0, 1_600.0, -2_500.0),
        radius: 900.0,
        speed: 130.0,
        angle: 0.0,
    };
    let (pos, _) = target.state();
    commands
        .spawn((
            Transform::from_translation(pos),
            Visibility::default(),
            target,
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

fn fly_target(time: Res<Time>, mut targets: Query<(&mut Transform, &mut Target)>) {
    for (mut tf, mut target) in &mut targets {
        target.angle += target.speed / target.radius * time.delta_secs();
        let (pos, vel) = target.state();

        // Coordinated turn: bank angle from speed and turn radius.
        let bank = (target.speed * target.speed / (9.81 * target.radius)).atan();
        let inward = (target.center - pos).with_y(0.0).normalize();
        let up = (Vec3::Y * bank.cos() + inward * bank.sin()).normalize();

        tf.translation = pos;
        tf.look_to(vel, up);
    }
}
