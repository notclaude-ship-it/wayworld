//! Les apps Wayland comme objets du monde : fenêtres flottantes, épinglage au bras,
//! utilisation (clavier/souris redirigés vers l'app), pont avec le compositeur.

use std::{
    collections::HashMap,
    collections::HashSet,
    f32::consts::FRAC_PI_2,
    process::{Command, Stdio},
    sync::{mpsc::Sender, Mutex},
};

use bevy::{
    asset::RenderAssetUsages,
    input::mouse::{AccumulatedMouseMotion, MouseWheel},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::{
    comp::{ToComp, ToWorld, WinId},
    keymap,
    player::{Mode, PlayerCam},
    settings::Settings,
    touch::{self, Act, TouchCtl},
    world::{facing, Slots},
    GameState,
};

/// Hauteur d'une fenêtre d'app dans le monde (m).
pub const PANEL_H: f32 = 1.3;
/// Hauteur d'une app épinglée sur le bras (m).
const PIN_H: f32 = 0.095;
const PIN_Y: [f32; 3] = [-0.115, 0.0, 0.115];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Floating,
    Pinned(usize),
    /// Tenue devant la caméra pendant l'utilisation.
    Held,
}

#[derive(Component)]
pub struct AppWin {
    pub id: WinId,
    pub px: UVec2,
    pub image: Handle<Image>,
    pub place: Place,
    /// Place avant d'être tenue.
    pub before_held: Place,
    pub home: Transform,
    /// Index du slot de départ occupé (usize::MAX = pose libre).
    pub slot: usize,
    pub phase: f32,
    pub title: String,
    pub app_id: String,
    screen: Entity,
    frame: Entity,
    cursor: Entity,
}

impl AppWin {
    pub fn width(&self) -> f32 {
        PANEL_H * self.px.x as f32 / self.px.y.max(1) as f32
    }
}

#[derive(Component)]
struct CursorDot;
#[derive(Component)]
struct Arm;
#[derive(Component)]
pub struct PinMount;

#[derive(Resource)]
pub struct Bridge {
    pub rx: Mutex<std::sync::mpsc::Receiver<ToWorld>>,
    pub tx: Sender<ToComp>,
    pub socket: String,
}

#[derive(Resource)]
struct Shared {
    quad: Handle<Mesh>,
    cube: Handle<Mesh>,
    frame_mat: Handle<StandardMaterial>,
    dot_mat: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
pub struct ArmUp(pub f32);

/// Outil de dev : force le bras levé.
#[derive(Resource, Default)]
pub struct ArmForce(pub bool);

#[derive(Clone, Copy)]
pub struct AimHit {
    pub entity: Entity,
    pub id: WinId,
    pub px: Vec2,
    pub dist: f32,
}

#[derive(Resource, Default)]
pub struct Aim(pub Option<AimHit>);

/// Texte d'aide affiché en bas de l'écran.
#[derive(Resource, Default)]
pub struct HudInfo(pub String);

#[derive(Resource, Default)]
pub struct CursorPx(pub Vec2);

#[derive(Resource, Default)]
pub struct PressedKeys(HashSet<u32>);

pub struct AppsPlugin;

impl Plugin for AppsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ArmUp>()
            .init_resource::<ArmForce>()
            .init_resource::<Aim>()
            .init_resource::<HudInfo>()
            .init_resource::<CursorPx>()
            .init_resource::<PressedKeys>()
            .add_systems(Startup, (setup_shared, spawn_arm.after(crate::player::spawn_player)))
            .add_systems(Update, (poll_compositor, float_windows, arm_anim, cursor_dots))
            .add_systems(
                Update,
                (aim_system, interact)
                    .chain()
                    .after(touch::touch_input)
                    .run_if(in_state(GameState::Playing)),
            );
    }
}

fn setup_shared(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(Shared {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        frame_mat: mats.add(StandardMaterial {
            base_color: Color::srgb(0.07, 0.07, 0.08),
            perceptual_roughness: 0.6,
            metallic: 0.8,
            ..default()
        }),
        dot_mat: mats.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.95, 0.4),
            unlit: true,
            ..default()
        }),
    });
}

fn make_image(w: u32, h: u32, rgba: Vec<u8>) -> Image {
    Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Lance une commande dont la fenêtre apparaîtra dans le monde.
pub fn launch(cmd: &str, socket: &str) {
    if cmd.trim().is_empty() {
        return;
    }
    let _ = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .env("WAYLAND_DISPLAY", socket)
        .env("GDK_BACKEND", "wayland")
        .env("QT_QPA_PLATFORM", "wayland")
        .env("MOZ_ENABLE_WAYLAND", "1")
        .env("SDL_VIDEODRIVER", "wayland")
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

// ---------------------------------------------------------------------------
// Pont compositeur -> monde
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn poll_compositor(
    mut commands: Commands,
    bridge: Res<Bridge>,
    shared: Res<Shared>,
    slots: Option<Res<Slots>>,
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut wins: Query<(Entity, &mut AppWin)>,
    mut mode: ResMut<Mode>,
) {
    let mut latest: HashMap<WinId, (u32, u32, Vec<u8>)> = HashMap::new();
    let mut metas: Vec<(WinId, String, String)> = Vec::new();
    let mut closed: Vec<WinId> = Vec::new();
    {
        let rx = bridge.rx.lock().unwrap();
        while let Ok(m) = rx.try_recv() {
            match m {
                ToWorld::Created { .. } => {}
                ToWorld::Meta { id, title, app_id } => metas.push((id, title, app_id)),
                ToWorld::Frame { id, width, height, rgba } => {
                    latest.insert(id, (width, height, rgba)); // on garde la plus récente
                }
                ToWorld::Closed { id } => closed.push(id),
            }
        }
    }

    for (id, title, app_id) in metas {
        if let Some((_, mut w)) = wins.iter_mut().find(|(_, w)| w.id == id) {
            w.title = title;
            w.app_id = app_id;
        }
    }

    for (id, (w, h, rgba)) in latest {
        if let Some((_, mut win)) = wins.iter_mut().find(|(_, win)| win.id == id) {
            if let Some(mut img) = images.get_mut(&win.image) {
                if win.px == UVec2::new(w, h) {
                    img.data = Some(rgba);
                } else {
                    *img = make_image(w, h, rgba);
                    win.px = UVec2::new(w, h);
                    let pw = win.width();
                    commands.entity(win.screen).insert(Transform::from_scale(Vec3::new(pw, PANEL_H, 1.0)));
                    commands.entity(win.frame).insert(
                        Transform::from_xyz(0.0, 0.0, -0.03).with_scale(Vec3::new(pw + 0.12, PANEL_H + 0.12, 0.05)),
                    );
                }
            }
        } else if !closed.contains(&id) {
            // première image : on crée l'objet dans le monde
            let used: HashSet<usize> = wins.iter().map(|(_, w)| w.slot).collect();
            let n = wins.iter().count();
            let slot_i = slots.as_ref().and_then(|s| (0..s.0.len()).find(|i| !used.contains(i)));
            let home = match (slot_i, slots.as_ref()) {
                (Some(i), Some(s)) => s.0[i],
                _ => facing(Vec3::new(0.0, 1.75, -6.0 + n as f32 * 0.01), Vec3::new(0.0, 1.75, 0.0)),
            };
            let handle = images.add(make_image(w, h, rgba));
            let pw = PANEL_H * w as f32 / h.max(1) as f32;
            let mat = mats.add(StandardMaterial {
                base_color_texture: Some(handle.clone()),
                unlit: true,
                cull_mode: None,
                double_sided: true,
                ..default()
            });
            let screen = commands
                .spawn((Mesh3d(shared.quad.clone()), MeshMaterial3d(mat), Transform::from_scale(Vec3::new(pw, PANEL_H, 1.0))))
                .id();
            let frame = commands
                .spawn((
                    Mesh3d(shared.cube.clone()),
                    MeshMaterial3d(shared.frame_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, -0.03).with_scale(Vec3::new(pw + 0.12, PANEL_H + 0.12, 0.05)),
                ))
                .id();
            let cursor = commands
                .spawn((
                    CursorDot,
                    Mesh3d(shared.quad.clone()),
                    MeshMaterial3d(shared.dot_mat.clone()),
                    Transform::from_xyz(0.0, 0.0, 0.004).with_scale(Vec3::new(0.03, 0.05, 1.0)),
                    Visibility::Hidden,
                ))
                .id();
            commands
                .spawn((
                    AppWin {
                        id,
                        px: UVec2::new(w, h),
                        image: handle,
                        place: Place::Floating,
                        before_held: Place::Floating,
                        home,
                        slot: slot_i.unwrap_or(usize::MAX),
                        phase: id as f32 * 1.7,
                        title: String::new(),
                        app_id: String::new(),
                        screen,
                        frame,
                        cursor,
                    },
                    home,
                    Visibility::default(),
                ))
                .add_children(&[screen, frame, cursor]);
            let _ = time;
        }
    }

    for id in closed {
        if let Some((e, _)) = wins.iter().find(|(_, w)| w.id == id) {
            commands.entity(e).despawn();
        }
        if *mode == Mode::Using(id) {
            *mode = Mode::Explore;
        }
        let _ = bridge.tx.send(ToComp::Focus(None));
    }
}

// ---------------------------------------------------------------------------
// Animation
// ---------------------------------------------------------------------------

fn float_windows(time: Res<Time>, mut q: Query<(&AppWin, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (w, mut tf) in &mut q {
        if w.place != Place::Floating {
            continue;
        }
        tf.translation = w.home.translation + Vec3::Y * (t * 0.9 + w.phase).sin() * 0.06;
        tf.rotation = w.home.rotation * Quat::from_rotation_z((t * 0.6 + w.phase).sin() * 0.02);
        tf.scale = Vec3::ONE;
    }
}

fn spawn_arm(mut commands: Commands, cam: Single<Entity, With<PlayerCam>>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let sleeve = mats.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.42, 0.08),
        perceptual_roughness: 0.9,
        ..default()
    });
    let glove = mats.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.12, 0.13),
        perceptual_roughness: 0.8,
        ..default()
    });
    let plate = mats.add(StandardMaterial {
        base_color: Color::srgb(0.06, 0.06, 0.07),
        metallic: 0.9,
        perceptual_roughness: 0.4,
        ..default()
    });
    let arm = commands
        .spawn((Arm, Transform::default(), Visibility::default(), ChildOf(*cam)))
        .id();
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.10, 0.09, 0.50))),
        MeshMaterial3d(sleeve),
        Transform::default(),
        ChildOf(arm),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.09, 0.07, 0.14))),
        MeshMaterial3d(glove),
        Transform::from_xyz(0.0, 0.0, -0.32),
        ChildOf(arm),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.20, 0.012, 0.36))),
        MeshMaterial3d(plate),
        Transform::from_xyz(0.0, 0.052, -0.02),
        ChildOf(arm),
    ));
    // repère des pins : +Z = normale de la plaque, +Y = vers la main
    commands.spawn((
        PinMount,
        Transform::from_xyz(0.0, 0.062, -0.02).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
        Visibility::default(),
        ChildOf(arm),
    ));
}

fn arm_pose(t: f32) -> Transform {
    let low = Transform::from_xyz(-0.34, -0.36, -0.52)
        .with_rotation(Quat::from_euler(EulerRot::YXZ, -0.35, 0.10, -0.55));
    let high = Transform::from_xyz(-0.12, -0.12, -0.42)
        .with_rotation(Quat::from_euler(EulerRot::YXZ, -0.10, 1.20, 0.0));
    Transform {
        translation: low.translation.lerp(high.translation, t),
        rotation: low.rotation.slerp(high.rotation, t),
        scale: Vec3::ONE,
    }
}

fn arm_anim(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    touch: Res<TouchCtl>,
    mode: Res<Mode>,
    state: Res<State<GameState>>,
    mut up: ResMut<ArmUp>,
    force: Res<ArmForce>,
    mut arm: Single<(&mut Transform, &mut Visibility), With<Arm>>,
) {
    let playing = *state.get() == GameState::Playing;
    let target = if force.0 || (playing && *mode == Mode::Explore && (keys.pressed(KeyCode::KeyQ) || (touch.enabled && touch.arm_up))) { 1.0 } else { 0.0 };
    up.0 += (target - up.0) * (1.0 - (-12.0 * time.delta_secs()).exp());
    let (tf, vis) = &mut *arm;
    **tf = arm_pose(up.0);
    **vis = if playing { Visibility::Inherited } else { Visibility::Hidden };
}

fn cursor_dots(
    mode: Res<Mode>,
    cpx: Res<CursorPx>,
    wins: Query<&AppWin>,
    mut dots: Query<(&mut Transform, &mut Visibility), With<CursorDot>>,
) {
    for w in &wins {
        let Ok((mut tf, mut vis)) = dots.get_mut(w.cursor) else { continue };
        if *mode == Mode::Using(w.id) {
            *vis = Visibility::Inherited;
            let pw = w.width();
            tf.translation = Vec3::new(
                (cpx.0.x / w.px.x as f32 - 0.5) * pw + 0.012,
                (0.5 - cpx.0.y / w.px.y as f32) * PANEL_H - 0.02,
                0.004,
            );
        } else {
            *vis = Visibility::Hidden;
        }
    }
}

// ---------------------------------------------------------------------------
// Visée + interactions
// ---------------------------------------------------------------------------

/// Première fenêtre touchée par le rayon (origine `o`, direction `d`).
fn pick(o: Vec3, d: Vec3, wins: &Query<(Entity, &AppWin, &GlobalTransform)>, arm_up: f32) -> Option<AimHit> {
    let mut best: Option<AimHit> = None;
    for (e, w, gt) in wins.iter() {
        let reach = match w.place {
            Place::Floating => 8.0,
            Place::Pinned(_) if arm_up > 0.6 => 1.5,
            _ => continue,
        };
        let n: Vec3 = *gt.back();
        let denom = d.dot(n);
        if denom >= -1e-4 {
            continue;
        }
        let t = (gt.translation() - o).dot(n) / denom;
        if t <= 0.0 || t > reach {
            continue;
        }
        let local = gt.affine().inverse().transform_point3(o + d * t);
        let (pw, ph) = (w.width(), PANEL_H);
        if local.x.abs() > pw / 2.0 || local.y.abs() > ph / 2.0 {
            continue;
        }
        let px = Vec2::new((local.x / pw + 0.5) * w.px.x as f32, (0.5 - local.y / ph) * w.px.y as f32);
        if best.map_or(true, |b| t < b.dist) {
            best = Some(AimHit { entity: e, id: w.id, px, dist: t });
        }
    }
    best
}

pub fn aim_system(
    cam: Single<(&Camera, &GlobalTransform), With<PlayerCam>>,
    wins: Query<(Entity, &AppWin, &GlobalTransform)>,
    arm: Res<ArmUp>,
    mode: Res<Mode>,
    mut touch: ResMut<TouchCtl>,
    mut aim: ResMut<Aim>,
    mut hud: ResMut<HudInfo>,
) {
    let (camera, cam_gt) = *cam;
    aim.0 = None;
    if *mode != Mode::Explore {
        hud.0 = if touch.enabled {
            "Touchez l'app pour viser · Quitter pour revenir".into()
        } else {
            "[Échap] quitter l'app · la souris déplace le curseur de l'app".into()
        };
        return;
    }
    let mut best = pick(cam_gt.translation(), *cam_gt.forward(), &wins, arm.0);
    // tap rapide sur une app : on la vise puis on l'utilise
    if touch.acts.iter().any(|a| matches!(a, Act::Tap(_))) {
        let acts = std::mem::take(&mut touch.acts);
        for a in acts {
            match a {
                Act::Tap(p) => {
                    if let Ok(ray) = camera.viewport_to_world(cam_gt, p) {
                        if let Some(h) = pick(ray.origin, *ray.direction, &wins, arm.0) {
                            best = Some(h);
                            touch.acts.push(Act::Use);
                        }
                    }
                }
                other => touch.acts.push(other),
            }
        }
    }
    aim.0 = best;
    let title = best
        .and_then(|h| wins.get(h.entity).ok())
        .map(|(_, w, _)| if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() })
        .unwrap_or_default();
    hud.0 = match (best.is_some(), touch.enabled) {
        (true, true) => title,
        (true, false) => format!("{title}  —  [E] utiliser · [F] épingler/détacher · [X] fermer"),
        (false, true) => String::new(),
        (false, false) => "[T] nouvelle app · [Q] lever le bras · [Maj] courir · [Échap] menu".into(),
    };
}

fn put(
    commands: &mut Commands,
    e: Entity,
    win: &mut AppWin,
    tf: &mut Transform,
    place: Place,
    cam: Entity,
    mount: Entity,
) {
    win.place = place;
    match place {
        Place::Floating => {
            commands.entity(e).remove::<ChildOf>();
            *tf = win.home;
        }
        Place::Pinned(i) => {
            commands.entity(e).insert(ChildOf(mount));
            *tf = Transform::from_xyz(0.0, PIN_Y[i], 0.004).with_scale(Vec3::splat(PIN_H / PANEL_H));
        }
        Place::Held => {
            commands.entity(e).insert(ChildOf(cam));
            *tf = Transform::from_xyz(0.0, -0.02, -1.0).with_scale(Vec3::splat(0.8));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn interact(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    motion: Res<AccumulatedMouseMotion>,
    settings: Res<Settings>,
    bridge: Res<Bridge>,
    aim: Res<Aim>,
    mut mode: ResMut<Mode>,
    mut next: ResMut<NextState<GameState>>,
    mut cpx: ResMut<CursorPx>,
    mut pressed: ResMut<PressedKeys>,
    mut touch: ResMut<TouchCtl>,
    mut wins: Query<(Entity, &mut AppWin, &mut Transform)>,
    cam: Single<(Entity, &GlobalTransform), With<PlayerCam>>,
    mount: Single<Entity, With<PinMount>>,
) {
    let (cam_e, cam_gt) = *cam;
    let mount_e = *mount;
    let acts: Vec<Act> = if touch.acts.is_empty() { Vec::new() } else { std::mem::take(&mut touch.acts) };
    let has = |a: Act| acts.contains(&a);

    match *mode {
        Mode::Explore => {
            wheel.clear();
            if keys.just_pressed(KeyCode::Escape) || has(Act::Menu) {
                next.set(GameState::Paused);
                return;
            }
            if keys.just_pressed(KeyCode::KeyT) || has(Act::NewApp) {
                launch(settings.startup_command(), &bridge.socket);
            }
            let Some(hit) = aim.0 else { return };
            if keys.just_pressed(KeyCode::KeyX) || has(Act::Close) {
                let _ = bridge.tx.send(ToComp::Close { id: hit.id });
            }
            if keys.just_pressed(KeyCode::KeyE) || has(Act::Use) {
                let size = if let Ok((e, mut w, mut tf)) = wins.get_mut(hit.entity) {
                    w.before_held = w.place;
                    put(&mut commands, e, &mut w, &mut tf, Place::Held, cam_e, mount_e);
                    w.px
                } else {
                    return;
                };
                cpx.0 = size.as_vec2() / 2.0;
                *mode = Mode::Using(hit.id);
                let _ = bridge.tx.send(ToComp::Focus(Some(hit.id)));
            } else if keys.just_pressed(KeyCode::KeyF) || has(Act::Pin) {
                let used: HashSet<usize> = wins
                    .iter()
                    .filter_map(|(_, w, _)| if let Place::Pinned(i) = w.place { Some(i) } else { None })
                    .collect();
                if let Ok((e, mut w, mut tf)) = wins.get_mut(hit.entity) {
                    match w.place {
                        Place::Floating => {
                            if let Some(i) = (0..PIN_Y.len()).find(|i| !used.contains(i)) {
                                put(&mut commands, e, &mut w, &mut tf, Place::Pinned(i), cam_e, mount_e);
                            }
                        }
                        Place::Pinned(_) => {
                            let p = cam_gt.translation() + *cam_gt.forward() * 1.7 - Vec3::Y * 0.15;
                            w.home = facing(p, cam_gt.translation());
                            put(&mut commands, e, &mut w, &mut tf, Place::Floating, cam_e, mount_e);
                        }
                        Place::Held => {}
                    }
                }
            }
        }
        Mode::Using(id) => {
            if keys.just_pressed(KeyCode::Escape) || has(Act::Leave) {
                for code in pressed.0.drain() {
                    let _ = bridge.tx.send(ToComp::Key { code, pressed: false });
                }
                if let Some((e, mut w, mut tf)) = wins.iter_mut().find(|(_, w, _)| w.id == id).map(|(e, w, t)| (e, w, t)) {
                    let back = w.before_held;
                    put(&mut commands, e, &mut w, &mut tf, back, cam_e, mount_e);
                }
                let _ = bridge.tx.send(ToComp::PointerLeave);
                let _ = bridge.tx.send(ToComp::Focus(None));
                *mode = Mode::Explore;
                return;
            }
            let Some((_, w, _)) = wins.iter().find(|(_, w, _)| w.id == id) else {
                *mode = Mode::Explore;
                return;
            };
            let size = w.px.as_vec2();
            // curseur virtuel
            cpx.0 = (cpx.0 + motion.delta * 1.25).clamp(Vec2::ZERO, size - Vec2::ONE);
            let _ = bridge.tx.send(ToComp::PointerMove { id, x: cpx.0.x as f64, y: cpx.0.y as f64 });
            // boutons
            for (b, code) in [
                (MouseButton::Left, keymap::BTN_LEFT),
                (MouseButton::Right, keymap::BTN_RIGHT),
                (MouseButton::Middle, keymap::BTN_MIDDLE),
            ] {
                if mouse.just_pressed(b) {
                    let _ = bridge.tx.send(ToComp::Button { code, pressed: true });
                }
                if mouse.just_released(b) {
                    let _ = bridge.tx.send(ToComp::Button { code, pressed: false });
                }
            }
            for ev in wheel.read() {
                let _ = bridge.tx.send(ToComp::Scroll { dx: -(ev.x as f64) * 15.0, dy: -(ev.y as f64) * 15.0 });
            }
            // clavier
            for k in keys.get_just_pressed() {
                if let Some(code) = keymap::evdev(*k) {
                    pressed.0.insert(code);
                    let _ = bridge.tx.send(ToComp::Key { code, pressed: true });
                }
            }
            for k in keys.get_just_released() {
                if let Some(code) = keymap::evdev(*k) {
                    if pressed.0.remove(&code) {
                        let _ = bridge.tx.send(ToComp::Key { code, pressed: false });
                    }
                }
            }
            // tactile : pointeur direct, boutons, molette, clavier à l'écran
            for a in &acts {
                match *a {
                    Act::Ptr(p) => {
                        cpx.0 = p.clamp(Vec2::ZERO, size - Vec2::ONE);
                        let _ = bridge.tx.send(ToComp::PointerMove { id, x: cpx.0.x as f64, y: cpx.0.y as f64 });
                    }
                    Act::Btn { code, pressed: down } => {
                        let _ = bridge.tx.send(ToComp::Button { code, pressed: down });
                    }
                    Act::Scroll(d) => {
                        let _ = bridge.tx.send(ToComp::Scroll { dx: -(d.x as f64), dy: -(d.y as f64) });
                    }
                    Act::Key { code, pressed: down } => {
                        if down {
                            pressed.0.insert(code);
                        } else {
                            pressed.0.remove(&code);
                        }
                        let _ = bridge.tx.send(ToComp::Key { code, pressed: down });
                    }
                    _ => {}
                }
            }
        }
    }
}