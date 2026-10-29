//! Joueur à la première personne : caméra, déplacement, souris, curseur verrouillé.

use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{settings::Settings, touch::TouchCtl, world::Walkable, GameState};

pub const EYE: f32 = 1.7;

#[derive(Component)]
pub struct Player {
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Component)]
pub struct PlayerCam;

/// Ce que fait le joueur en ce moment.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    #[default]
    Explore,
    /// Utilise l'app `WinId` : clavier/souris sont redirigés vers elle.
    Using(u32),
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mode>()
            .add_systems(Startup, spawn_player)
            .add_systems(OnEnter(GameState::Playing), lock_cursor)
            .add_systems(OnExit(GameState::Playing), unlock_cursor)
            .add_systems(
                Update,
                (look, walk)
                    .chain()
                    .after(crate::touch::touch_input)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(Update, update_cursor_policy.after(crate::touch::touch_input).run_if(in_state(GameState::Playing)))
            .add_systems(Update, menu_orbit.run_if(not(in_state(GameState::Playing))))
            .add_systems(Update, apply_camera_settings);
    }
}

pub fn spawn_player(mut commands: Commands, settings: Res<Settings>) {
    commands
        .spawn((
            Player { yaw: 0.0, pitch: 0.0 },
            Transform::from_xyz(0.0, 0.0, 4.0),
            Visibility::default(),
        ))
        .with_children(|p| {
            p.spawn((
                PlayerCam,
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection {
                    fov: settings.fov.to_radians(),
                    near: 0.05,
                    ..default()
                }),
                Transform::from_xyz(0.0, EYE, 0.0),
                DistanceFog {
                    color: Color::srgb(0.012, 0.014, 0.02),
                    falloff: FogFalloff::from_visibility(settings.fog_visibility),
                    ..default()
                },
            ));
        });
}

fn apply_camera_settings(
    settings: Res<Settings>,
    mut cam: Query<(&mut Projection, &mut DistanceFog), With<PlayerCam>>,
) {
    if !settings.is_changed() {
        return;
    }
    for (mut proj, mut fog) in &mut cam {
        if let Projection::Perspective(p) = &mut *proj {
            p.fov = settings.fov.to_radians();
        }
        fog.falloff = FogFalloff::from_visibility(settings.fog_visibility);
    }
}

fn lock_cursor(touch: Res<TouchCtl>, mut c: Single<&mut CursorOptions>) {
    cursor_policy(&touch, &mut c);
}

/// Sous X11, le grab du pointeur (repli de `Locked` -> `Confined`) avale tous les événements
/// tactiles : on ne grabe donc pas la souris tant que les contrôles tactiles sont actifs.
fn cursor_policy(touch: &TouchCtl, c: &mut CursorOptions) {
    let (grab, visible) = if touch.enabled { (CursorGrabMode::None, true) } else { (CursorGrabMode::Locked, false) };
    if c.grab_mode != grab {
        c.grab_mode = grab;
    }
    if c.visible != visible {
        c.visible = visible;
    }
}

fn update_cursor_policy(touch: Res<TouchCtl>, mut c: Single<&mut CursorOptions>) {
    cursor_policy(&touch, &mut c);
}

fn unlock_cursor(mut c: Single<&mut CursorOptions>) {
    c.visible = true;
    c.grab_mode = CursorGrabMode::None;
}

pub fn look(
    mode: Res<Mode>,
    motion: Res<AccumulatedMouseMotion>,
    touch: Res<TouchCtl>,
    settings: Res<Settings>,
    mut player: Single<(&mut Player, &mut Transform), Without<PlayerCam>>,
    mut cam: Single<&mut Transform, With<PlayerCam>>,
) {
    if *mode != Mode::Explore {
        return;
    }
    let (p, tf) = &mut *player;
    let d = (motion.delta + touch.look_delta * 1.6) * settings.sensitivity;
    p.yaw -= d.x;
    p.pitch = (p.pitch - d.y).clamp(-1.45, 1.45);
    tf.rotation = Quat::from_rotation_y(p.yaw);
    cam.rotation = Quat::from_rotation_x(p.pitch);
}

pub fn walk(
    mode: Res<Mode>,
    keys: Res<ButtonInput<KeyCode>>,
    touch: Res<TouchCtl>,
    time: Res<Time>,
    settings: Res<Settings>,
    walkable: Option<Res<Walkable>>,
    mut player: Single<&mut Transform, With<Player>>,
    mut cam: Single<&mut Transform, (With<PlayerCam>, Without<Player>)>,
) {
    let t = time.elapsed_secs();
    if *mode != Mode::Explore {
        return;
    }
    // Positions physiques : W/A/S/D = Z/Q/S/D sur un clavier AZERTY.
    let mut dir = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        dir += *player.forward();
    }
    if keys.pressed(KeyCode::KeyS) {
        dir += *player.back();
    }
    if keys.pressed(KeyCode::KeyA) {
        dir += *player.left();
    }
    if keys.pressed(KeyCode::KeyD) {
        dir += *player.right();
    }
    // joystick tactile : analogique (x = droite, y = avant)
    dir += *player.forward() * touch.move_axis.y + *player.right() * touch.move_axis.x;
    let moving = dir.length_squared() > 1e-6;
    if moving {
        let sprint = if keys.pressed(KeyCode::ShiftLeft) || touch.sprint { 1.7 } else { 1.0 };
        let step = dir.normalize() * dir.length().min(1.0) * settings.move_speed * sprint * time.delta_secs();
        let pos = player.translation;
        let Some(w) = walkable else { return };
        // glissement le long des murs : on essaie X puis Z séparément
        let nx = Vec2::new(pos.x + step.x, pos.z);
        if w.allows(nx, 0.4) {
            player.translation.x = nx.x;
        }
        let nz = Vec2::new(player.translation.x, pos.z + step.z);
        if w.allows(nz, 0.4) {
            player.translation.z = nz.y;
        }
    }
    // léger balancement de tête en marchant
    let bob = if moving { (t * 9.0).sin() * 0.025 } else { 0.0 };
    cam.translation.y += (EYE + bob - cam.translation.y) * 0.3;
}

/// Caméra d'ambiance dans le menu : lent tour du hall.
fn menu_orbit(time: Res<Time>, mut player: Single<(&mut Player, &mut Transform)>, mut cam: Single<&mut Transform, (With<PlayerCam>, Without<Player>)>) {
    let (p, tf) = &mut *player;
    let t = time.elapsed_secs();
    p.yaw = t * 0.12;
    p.pitch = -0.05;
    tf.translation = Vec3::new(0.0, 0.0, 0.0) + Vec3::new((t * 0.07).cos() * 2.0, 0.0, (t * 0.07).sin() * 2.0);
    tf.rotation = Quat::from_rotation_y(p.yaw);
    cam.rotation = Quat::from_rotation_x(p.pitch);
    cam.translation.y = EYE + (t * 0.5).sin() * 0.05;
}