//! Support VR (OpenXR) : SteamVR + Meta Quest 2 (Link, Air Link, ALVR ou Virtual Desktop).
//!
//! Compilation : `cargo build --release --features vr`. Lancement : SteamVR (runtime OpenXR actif)
//! puis `WAYWORLD_VR=1 wayworld`. Sans runtime OpenXR, Wayworld tourne normalement en écran plat.
//!
//! Commandes (manettes Touch) :
//!   Explorer : stick gauche = marcher (dans la direction du regard), stick droit = rotation par
//!   crans, gâchette gauche = courir, gâchette droite = utiliser l'app visée (laser de la main
//!   droite), grip droit = épingler/détacher, A = nouvelle app, X = fermer l'app visée.
//!   Utiliser une app : le laser de la main droite déplace le curseur, gâchette droite = clic
//!   gauche, grip droit = clic droit, stick droit haut/bas = molette, stick gauche haut/bas =
//!   zoom, B ou Y = quitter l'app. Le clavier physique du PC reste utilisable.

use std::ops::Deref;

use bevy::prelude::*;
use bevy_mod_openxr::{
    action_binding::{OxrSendActionBindings, OxrSuggestActionBinding},
    action_set_attaching::OxrAttachActionSet,
    action_set_syncing::{OxrActionSetSyncSet, OxrSyncActionSet},
    openxr_session_running,
    resources::OxrInstance,
    session::OxrSession,
};
use bevy_mod_xr::{
    camera::XrCamera,
    session::{session_available, XrSessionCreated, XrTrackingRoot},
};
use openxr::{Posef, Vector2f};

use crate::{
    apps::{self, Aim, AppWin, PANEL_H},
    keymap,
    menu::Fresh,
    player::{self, Mode, Player, PlayerCam},
    touch::{self, Act, TouchCtl},
    GameState,
};

const PROFILE_TOUCH: &str = "/interaction_profiles/oculus/touch_controller";
const PROFILE_INDEX: &str = "/interaction_profiles/valve/index_controller";
const PROFILE_SIMPLE: &str = "/interaction_profiles/khr/simple_controller";

/// Angle d'un cran de rotation (radians).
const SNAP: f32 = 30.0 * std::f32::consts::PI / 180.0;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum Hand {
    Left,
    Right,
}

#[derive(Component)]
struct Laser;

/// Longueur du laser quand on utilise une app (distance jusqu'à l'écran visé).
#[derive(Resource)]
struct VrRay(f32);

/// Garde aussi le rendu de la fenêtre du PC (vue du spectateur) si WAYWORLD_VR_MIRROR=1.
#[derive(Resource)]
struct VrMirror(bool);

#[derive(Resource)]
struct VrActions {
    set: openxr::ActionSet,
    walk: openxr::Action<Vector2f>,
    turn: openxr::Action<Vector2f>,
    trig_l: openxr::Action<f32>,
    trig_r: openxr::Action<f32>,
    grip_r: openxr::Action<f32>,
    a: openxr::Action<bool>,
    b: openxr::Action<bool>,
    x: openxr::Action<bool>,
    y: openxr::Action<bool>,
    aim_l: openxr::Action<Posef>,
    aim_r: openxr::Action<Posef>,
}

pub struct VrPlugin;

impl Plugin for VrPlugin {
    fn build(&self, app: &mut App) {
        let mirror = std::env::var("WAYWORLD_VR_MIRROR").map(|v| v == "1").unwrap_or(false);
        app.insert_resource(VrRay(3.0))
            .insert_resource(VrMirror(mirror))
            .add_systems(Startup, create_actions.run_if(session_available))
            .add_systems(OxrSendActionBindings, suggest_bindings)
            .add_systems(XrSessionCreated, (attach_set, spawn_hands))
            .add_systems(PreUpdate, sync_actions.before(OxrActionSetSyncSet).run_if(openxr_session_running))
            .add_systems(Update, attach_root)
            .add_systems(
                Update,
                autostart.run_if(openxr_session_running).run_if(in_state(GameState::Menu)),
            )
            .add_systems(
                Update,
                vr_input
                    .after(touch::touch_input)
                    .before(player::look)
                    .before(player::walk)
                    .before(apps::aim_system)
                    .before(apps::interact)
                    .run_if(openxr_session_running)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(
                Update,
                vr_pose
                    .after(player::walk)
                    .before(apps::aim_system)
                    .run_if(openxr_session_running)
                    .run_if(in_state(GameState::Playing)),
            )
            .add_systems(Update, update_laser.run_if(openxr_session_running));
    }
}

// ---------------------------------------------------------------------------
// Actions OpenXR

fn create_actions(instance: Res<OxrInstance>, mut cmds: Commands) {
    let set = instance.create_action_set("wayworld", "Wayworld", 0).unwrap();
    let v2 = |n: &str, l: &str| set.create_action::<Vector2f>(n, l, &[]).unwrap();
    let f = |n: &str, l: &str| set.create_action::<f32>(n, l, &[]).unwrap();
    let b = |n: &str, l: &str| set.create_action::<bool>(n, l, &[]).unwrap();
    let p = |n: &str, l: &str| set.create_action::<Posef>(n, l, &[]).unwrap();
    let actions = VrActions {
        walk: v2("walk", "Marcher"),
        turn: v2("turn", "Tourner"),
        trig_l: f("trigger_left", "Gachette gauche"),
        trig_r: f("trigger_right", "Gachette droite"),
        grip_r: f("grip_right", "Grip droit"),
        a: b("button_a", "Bouton A"),
        b: b("button_b", "Bouton B"),
        x: b("button_x", "Bouton X"),
        y: b("button_y", "Bouton Y"),
        aim_l: p("aim_left", "Visee gauche"),
        aim_r: p("aim_right", "Visee droite"),
        set: set.clone(),
    };
    cmds.insert_resource(actions);
}

fn suggest_bindings(actions: Res<VrActions>, mut w: MessageWriter<OxrSuggestActionBinding>) {
    let mut send = |action: openxr::sys::Action, profile: &'static str, path: &'static str| {
        w.write(OxrSuggestActionBinding {
            action,
            interaction_profile: profile.into(),
            bindings: vec![path.into()],
        });
    };
    let a = &*actions;
    // Manettes Touch (Quest 2 via SteamVR / ALVR / Link)
    for (act, path) in [
        (a.walk.as_raw(), "/user/hand/left/input/thumbstick"),
        (a.turn.as_raw(), "/user/hand/right/input/thumbstick"),
        (a.trig_l.as_raw(), "/user/hand/left/input/trigger/value"),
        (a.trig_r.as_raw(), "/user/hand/right/input/trigger/value"),
        (a.grip_r.as_raw(), "/user/hand/right/input/squeeze/value"),
        (a.a.as_raw(), "/user/hand/right/input/a/click"),
        (a.b.as_raw(), "/user/hand/right/input/b/click"),
        (a.x.as_raw(), "/user/hand/left/input/x/click"),
        (a.y.as_raw(), "/user/hand/left/input/y/click"),
        (a.aim_l.as_raw(), "/user/hand/left/input/aim/pose"),
        (a.aim_r.as_raw(), "/user/hand/right/input/aim/pose"),
    ] {
        send(act, PROFILE_TOUCH, path);
    }
    // Valve Index (X/Y = A/B de la main gauche)
    for (act, path) in [
        (a.walk.as_raw(), "/user/hand/left/input/thumbstick"),
        (a.turn.as_raw(), "/user/hand/right/input/thumbstick"),
        (a.trig_l.as_raw(), "/user/hand/left/input/trigger/value"),
        (a.trig_r.as_raw(), "/user/hand/right/input/trigger/value"),
        (a.grip_r.as_raw(), "/user/hand/right/input/squeeze/value"),
        (a.a.as_raw(), "/user/hand/right/input/a/click"),
        (a.b.as_raw(), "/user/hand/right/input/b/click"),
        (a.x.as_raw(), "/user/hand/left/input/a/click"),
        (a.y.as_raw(), "/user/hand/left/input/b/click"),
        (a.aim_l.as_raw(), "/user/hand/left/input/aim/pose"),
        (a.aim_r.as_raw(), "/user/hand/right/input/aim/pose"),
    ] {
        send(act, PROFILE_INDEX, path);
    }
    // Manette minimale : viser + « select » pour utiliser
    send(a.trig_r.as_raw(), PROFILE_SIMPLE, "/user/hand/right/input/select/click");
    send(a.b.as_raw(), PROFILE_SIMPLE, "/user/hand/right/input/menu/click");
    send(a.aim_l.as_raw(), PROFILE_SIMPLE, "/user/hand/left/input/aim/pose");
    send(a.aim_r.as_raw(), PROFILE_SIMPLE, "/user/hand/right/input/aim/pose");
}

fn attach_set(actions: Res<VrActions>, mut attach: MessageWriter<OxrAttachActionSet>) {
    attach.write(OxrAttachActionSet(actions.set.clone()));
}

fn sync_actions(actions: Res<VrActions>, mut sync: MessageWriter<OxrSyncActionSet>) {
    sync.write(OxrSyncActionSet(actions.set.clone()));
}

fn spawn_hands(
    actions: Res<VrActions>,
    session: Res<OxrSession>,
    mut cmds: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let left = session
        .create_action_space(&actions.aim_l, openxr::Path::NULL, Isometry3d::IDENTITY)
        .unwrap();
    let right = session
        .create_action_space(&actions.aim_r, openxr::Path::NULL, Isometry3d::IDENTITY)
        .unwrap();
    let laser_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.55, 0.15),
        unlit: true,
        ..default()
    });
    let hand_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.9, 0.9, 0.95),
        unlit: true,
        ..default()
    });
    let laser = cmds
        .spawn((
            Laser,
            Mesh3d(meshes.add(Cuboid::new(0.004, 0.004, 1.0))),
            MeshMaterial3d(laser_mat),
            Transform::from_xyz(0.0, 0.0, -1.5).with_scale(Vec3::new(1.0, 1.0, 3.0)),
        ))
        .id();
    let r = cmds
        .spawn((
            right,
            Hand::Right,
            Mesh3d(meshes.add(Cuboid::new(0.03, 0.03, 0.09))),
            MeshMaterial3d(hand_mat.clone()),
        ))
        .id();
    cmds.entity(r).add_child(laser);
    cmds.spawn((left, Hand::Left, Mesh3d(meshes.add(Sphere::new(0.025))), MeshMaterial3d(hand_mat)));
}

/// Le repère de suivi XR devient enfant du joueur : marcher/tourner déplace le casque.
fn attach_root(
    mut done: Local<bool>,
    mut cmds: Commands,
    root: Query<Entity, With<XrTrackingRoot>>,
    player: Query<Entity, With<Player>>,
) {
    if *done {
        return;
    }
    let (Ok(r), Ok(p)) = (root.single(), player.single()) else { return };
    cmds.entity(r).insert(ChildOf(p));
    *done = true;
}

/// En VR on ne peut pas cliquer sur « Jouer » dans la fenêtre : on démarre tout seul.
fn autostart(mut next: ResMut<NextState<GameState>>, mut fresh: ResMut<Fresh>) {
    fresh.0 = true;
    next.set(GameState::Playing);
}

// ---------------------------------------------------------------------------
// Entrées

#[derive(Default)]
struct Prev {
    trig_r: bool,
    grip_r: bool,
    a: bool,
    x: bool,
    b: bool,
    y: bool,
    snap: bool,
    was_using: bool,
    sent_left: bool,
    sent_right: bool,
}

fn pressed(v: f32, was: bool) -> bool {
    if was {
        v > 0.4
    } else {
        v > 0.65
    }
}

fn deadzone(v: Vec2, dz: f32) -> Vec2 {
    if v.length() < dz {
        Vec2::ZERO
    } else {
        v
    }
}

#[allow(clippy::too_many_arguments)]
fn vr_input(
    time: Res<Time>,
    session: Option<Res<OxrSession>>,
    actions: Option<Res<VrActions>>,
    mode: Res<Mode>,
    mut ctl: ResMut<TouchCtl>,
    mut prev: Local<Prev>,
    mut ray: ResMut<VrRay>,
    hands: Query<(&Hand, &GlobalTransform)>,
    heads: Query<&GlobalTransform, With<XrCamera>>,
    mut player: Single<(&mut Player, &mut Transform), Without<PlayerCam>>,
    wins: Query<(&AppWin, &GlobalTransform)>,
) {
    let (Some(session), Some(act)) = (session, actions) else { return };
    let s = session.deref().deref();
    let np = openxr::Path::NULL;
    let v2 = |a: &openxr::Action<Vector2f>| {
        a.state(s, np)
            .ok()
            .filter(|st| st.is_active)
            .map(|st| Vec2::new(st.current_state.x, st.current_state.y))
            .unwrap_or(Vec2::ZERO)
    };
    let f = |a: &openxr::Action<f32>| a.state(s, np).ok().filter(|st| st.is_active).map(|st| st.current_state).unwrap_or(0.0);
    let b = |a: &openxr::Action<bool>| a.state(s, np).ok().filter(|st| st.is_active).map(|st| st.current_state).unwrap_or(false);

    let walk = deadzone(v2(&act.walk), 0.15);
    let turn = deadzone(v2(&act.turn), 0.15);
    let trig_l = f(&act.trig_l);
    let trig_r = f(&act.trig_r);
    let grip_r = f(&act.grip_r);
    let (a, bb, x, y) = (b(&act.a), b(&act.b), b(&act.x), b(&act.y));

    let trig_now = pressed(trig_r, prev.trig_r);
    let grip_now = pressed(grip_r, prev.grip_r);
    let using = matches!(*mode, Mode::Using(_));

    // tête : position moyenne des deux yeux, direction du premier
    let n = heads.iter().count();
    let head_pos = if n > 0 { heads.iter().map(|g| g.translation()).sum::<Vec3>() / n as f32 } else { Vec3::ZERO };
    let head_fwd = heads.iter().next().map(|g| *g.forward()).unwrap_or(Vec3::NEG_Z);

    ctl.sprint = trig_l > 0.6;

    if !using {
        // relâché pendant une app : rien à envoyer
        if prev.was_using {
            if prev.sent_left {
                ctl.acts.push(Act::Btn { code: keymap::BTN_LEFT, pressed: false });
            }
            prev.sent_left = false;
            prev.sent_right = false;
        }
        // marcher dans la direction du regard
        if walk != Vec2::ZERO {
            let flat = Vec3::new(head_fwd.x, 0.0, head_fwd.z).normalize_or_zero();
            let right = flat.cross(Vec3::Y);
            let dir = flat * walk.y + right * walk.x;
            let (_, ptf) = &*player;
            let (pf, pr): (Vec3, Vec3) = (*ptf.forward(), *ptf.right());
            ctl.move_axis = Vec2::new(dir.dot(pr), dir.dot(pf));
        }
        // rotation par crans autour de la tête
        if turn.x.abs() > 0.7 && !prev.snap {
            prev.snap = true;
            let delta = if turn.x > 0.0 { -SNAP } else { SNAP };
            let rot = Quat::from_rotation_y(delta);
            let (p, tf) = &mut *player;
            let h = Vec3::new(head_pos.x, tf.translation.y, head_pos.z);
            tf.translation = h + rot * (tf.translation - h);
            p.yaw += delta;
            tf.rotation = Quat::from_rotation_y(p.yaw);
        } else if turn.x.abs() < 0.3 {
            prev.snap = false;
        }
        if trig_now && !prev.trig_r {
            ctl.acts.push(Act::Use);
        }
        if grip_now && !prev.grip_r {
            ctl.acts.push(Act::Pin);
        }
        if a && !prev.a {
            ctl.acts.push(Act::NewApp);
        }
        if x && !prev.x {
            ctl.acts.push(Act::Close);
        }
    } else {
        if !prev.was_using {
            // la gâchette qui a lancé l'app est encore enfoncée : ce n'est pas un clic
            prev.trig_r = true;
            prev.grip_r = true;
        }
        // pointeur : le laser coupe l'écran tenu
        if let (Mode::Using(id), Some((_, aim))) = (*mode, hands.iter().find(|(h, _)| **h == Hand::Right)) {
            if let Some((w, gt)) = wins.iter().find(|(w, _)| w.id == id) {
                let o = aim.translation();
                let d: Vec3 = *aim.forward();
                let nrm: Vec3 = *gt.back();
                let denom = d.dot(nrm);
                if denom < -1e-4 {
                    let t = (gt.translation() - o).dot(nrm) / denom;
                    if t > 0.0 {
                        ray.0 = t;
                        let local = gt.affine().inverse().transform_point3(o + d * t);
                        let (pw, ph) = (w.width(), PANEL_H);
                        if local.x.abs() <= pw / 2.0 && local.y.abs() <= ph / 2.0 {
                            let px = Vec2::new((local.x / pw + 0.5) * w.px.x as f32, (0.5 - local.y / ph) * w.px.y as f32);
                            ctl.acts.push(Act::Ptr(px));
                        }
                    }
                }
            }
        }
        if trig_now && !prev.trig_r {
            ctl.acts.push(Act::Btn { code: keymap::BTN_LEFT, pressed: true });
            prev.sent_left = true;
        } else if !trig_now && prev.trig_r && prev.sent_left {
            ctl.acts.push(Act::Btn { code: keymap::BTN_LEFT, pressed: false });
            prev.sent_left = false;
        }
        if grip_now && !prev.grip_r {
            ctl.acts.push(Act::Btn { code: keymap::BTN_RIGHT, pressed: true });
            prev.sent_right = true;
        } else if !grip_now && prev.grip_r && prev.sent_right {
            ctl.acts.push(Act::Btn { code: keymap::BTN_RIGHT, pressed: false });
            prev.sent_right = false;
        }
        // molette (stick droit) et zoom (stick gauche)
        if turn.y != 0.0 {
            ctl.acts.push(Act::Scroll(Vec2::new(0.0, turn.y * 6.0)));
        }
        if walk.y != 0.0 {
            ctl.zoom = (ctl.zoom * (1.0 + walk.y * time.delta_secs() * 1.2)).clamp(0.5, 3.0);
        }
        if (bb && !prev.b) || (y && !prev.y) {
            ctl.acts.push(Act::Leave);
        }
    }

    prev.trig_r = trig_now;
    prev.grip_r = grip_now;
    prev.a = a;
    prev.x = x;
    prev.b = bb;
    prev.y = y;
    prev.was_using = using;
}

/// La caméra du joueur (utilisée pour viser et pour tenir les apps) suit la main droite en
/// exploration ; pendant l'utilisation d'une app elle reste figée là où on regardait, pour que
/// l'écran reste fixe dans le monde au lieu de coller au visage.
fn vr_pose(
    mode: Res<Mode>,
    mirror: Res<VrMirror>,
    hands: Query<(&Hand, &GlobalTransform)>,
    heads: Query<&GlobalTransform, With<XrCamera>>,
    player: Single<&GlobalTransform, With<Player>>,
    mut cam: Single<(&mut Transform, &mut Camera), With<PlayerCam>>,
    mut frozen: Local<Option<Transform>>,
) {
    let n = heads.iter().count();
    if n == 0 {
        return;
    }
    let inv = player.to_matrix().inverse();
    let head_pos = heads.iter().map(|g| g.translation()).sum::<Vec3>() / n as f32;
    let head_fwd = heads.iter().next().map(|g| *g.forward()).unwrap_or(Vec3::NEG_Z);
    let (tf, camera) = &mut *cam;
    match *mode {
        Mode::Explore => {
            *frozen = None;
            let target = hands
                .iter()
                .find(|(h, _)| **h == Hand::Right)
                .map(|(_, g)| g.to_matrix())
                .unwrap_or_else(|| Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::NEG_Z, head_fwd), head_pos));
            **tf = Transform::from_matrix(inv * target);
        }
        Mode::Using(_) => {
            if frozen.is_none() {
                let yaw = (-head_fwd.x).atan2(-head_fwd.z);
                let target = Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), head_pos);
                *frozen = Some(Transform::from_matrix(inv * target));
            }
            if let Some(t) = *frozen {
                **tf = t;
            }
        }
    }
    if camera.is_active != mirror.0 {
        camera.is_active = mirror.0;
    }
}

fn update_laser(aim: Res<Aim>, ray: Res<VrRay>, mode: Res<Mode>, mut q: Query<&mut Transform, With<Laser>>) {
    let d = match *mode {
        Mode::Explore => aim.0.map(|h| h.dist).unwrap_or(3.0),
        Mode::Using(_) => ray.0,
    }
    .clamp(0.05, 12.0);
    for mut t in &mut q {
        t.translation.z = -d / 2.0;
        t.scale.z = d;
    }
}