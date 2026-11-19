//! On-screen markers: the mouse aim circle, where the nose points, a box
//! around the target, and a flight data readout.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

use crate::camera::{ChaseCamera, MouseAim};
use crate::flight::Player;
use crate::target::Target;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(PostUpdate, update_hud.after(TransformSystems::Propagate));
    }
}

#[derive(Component)]
struct AimCircle;

#[derive(Component)]
struct NoseMarker;

#[derive(Component)]
struct TargetBox;

#[derive(Component)]
struct TargetLabel;

#[derive(Component)]
struct FlightReadout;

#[derive(Component)]
struct HelpText;

const AIM_SIZE: f32 = 26.0;
const NOSE_SIZE: f32 = 10.0;
const BOX_SIZE: f32 = 34.0;

fn marker(size: f32, border: f32, color: Color, round: bool) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            width: px(size),
            height: px(size),
            border: UiRect::all(px(border)),
            border_radius: if round {
                BorderRadius::MAX
            } else {
                BorderRadius::ZERO
            },
            ..default()
        },
        BorderColor::all(color),
        Visibility::Hidden,
    )
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((marker(AIM_SIZE, 2.0, Color::WHITE, true), AimCircle));
    commands.spawn((
        marker(NOSE_SIZE, 2.0, Color::srgb(0.3, 1.0, 0.4), false),
        NoseMarker,
    ));
    commands.spawn((
        marker(BOX_SIZE, 1.5, Color::srgb(1.0, 0.35, 0.3), false),
        TargetBox,
    ));
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.35, 0.3)),
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        Visibility::Hidden,
        TargetLabel,
    ));
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: px(10),
            left: px(12),
            ..default()
        },
        FlightReadout,
    ));
    commands.spawn((
        Text::new("Click to capture the mouse    Esc: release    W/S: throttle    R: reset"),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(12),
            ..default()
        },
        HelpText,
    ));
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_hud(
    aim: Res<MouseAim>,
    cursor: Single<&CursorOptions>,
    camera: Single<(&Camera, &GlobalTransform), With<ChaseCamera>>,
    player: Single<(&GlobalTransform, &Player)>,
    target: Single<&GlobalTransform, With<Target>>,
    mut markers: ParamSet<(
        Single<(&mut Node, &mut Visibility), With<AimCircle>>,
        Single<(&mut Node, &mut Visibility), With<NoseMarker>>,
        Single<(&mut Node, &mut Visibility), With<TargetBox>>,
        Single<(&mut Node, &mut Visibility, &mut Text), With<TargetLabel>>,
    )>,
    mut readout: Single<&mut Text, (With<FlightReadout>, Without<TargetLabel>)>,
    mut help: Single<
        &mut Visibility,
        (
            With<HelpText>,
            Without<AimCircle>,
            Without<NoseMarker>,
            Without<TargetBox>,
            Without<TargetLabel>,
        ),
    >,
) {
    let (cam, cam_tf) = *camera;
    let (player_tf, plane) = *player;
    let project = |world: Vec3| -> Option<Vec2> {
        // Behind the camera means not on screen.
        if (world - cam_tf.translation()).dot(cam_tf.forward().as_vec3()) <= 0.0 {
            return None;
        }
        cam.world_to_viewport(cam_tf, world).ok()
    };

    // Mouse aim circle: a point far along the aim direction.
    let aim_point = cam_tf.translation() + aim.direction() * 2_000.0;
    place(markers.p0().into_inner(), project(aim_point), AIM_SIZE);

    // Nose marker: where the plane is actually pointing (and where guns go).
    let nose_point = player_tf.translation() + player_tf.forward().as_vec3() * 2_000.0;
    place(markers.p1().into_inner(), project(nose_point), NOSE_SIZE);

    // Target box and range.
    let target_pos = target.translation();
    let range = player_tf.translation().distance(target_pos);
    let target_screen = project(target_pos);
    place(markers.p2().into_inner(), target_screen, BOX_SIZE);
    {
        let (mut node, mut vis, mut text) = markers.p3().into_inner();
        match target_screen {
            Some(p) => {
                node.left = px(p.x - BOX_SIZE / 2.0);
                node.top = px(p.y + BOX_SIZE / 2.0 + 2.0);
                text.0 = format!("{:.0} m", range);
                *vis = Visibility::Visible;
            }
            None => *vis = Visibility::Hidden,
        }
    }

    readout.0 = format!(
        "IAS {:>4.0} km/h\nALT {:>5.0} m\nTHR {:>4.0} %\nTGT {:>5.0} m",
        plane.speed * 3.6,
        player_tf.translation().y,
        plane.throttle * 100.0,
        range,
    );

    **help = if cursor.grab_mode == CursorGrabMode::None {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

fn place((mut node, mut vis): (Mut<Node>, Mut<Visibility>), screen: Option<Vec2>, size: f32) {
    match screen {
        Some(p) => {
            node.left = px(p.x - size / 2.0);
            node.top = px(p.y - size / 2.0);
            *vis = Visibility::Visible;
        }
        None => *vis = Visibility::Hidden,
    }
}
