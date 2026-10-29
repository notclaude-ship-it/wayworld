//! Contrôles tactiles : joystick flottant (moitié gauche), regard au doigt (moitié droite),
//! boutons d'action, tap sur une app pour l'utiliser, pointeur direct sur l'app tenue
//! (tap = clic, glisser = drag, deux doigts = molette) et clavier à l'écran.
//!
//! Les contrôles n'apparaissent qu'après le premier toucher (et disparaissent dès qu'on
//! utilise le clavier ou la souris). `WAYWORLD_TOUCH=1` les force, `=0` les désactive.
//! `WAYWORLD_KB=azerty|qwerty` force les étiquettes du clavier à l'écran (sinon : selon
//! `XKB_DEFAULT_LAYOUT`).

use std::collections::{HashMap, HashSet};

use bevy::{prelude::*, window::PrimaryWindow};

use crate::{
    apps::{AppWin, CursorPx, Place, PANEL_H},
    keymap,
    player::{Mode, PlayerCam},
    GameState,
};

/// Actions produites par les doigts, consommées par `apps::interact` / `aim_system`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Use,
    Pin,
    Close,
    NewApp,
    Menu,
    Leave,
    /// Tap rapide dans le monde (position écran, px logiques) : utiliser l'app visée.
    Tap(Vec2),
    /// Déplacement du pointeur, en pixels de l'app tenue.
    Ptr(Vec2),
    Btn { code: u32, pressed: bool },
    Scroll(Vec2),
    Key { code: u32, pressed: bool },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Wid {
    Menu,
    Use,
    Pin,
    Close,
    NewApp,
    ArmUp,
    Sprint,
    Leave,
    KbToggle,
    RightClick,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ListBg,
    ListTitle,
    AppItem(u32),
    Key(u32),
}

#[derive(Clone, Debug)]
struct Widget {
    id: Wid,
    rect: Rect,
    label: String,
    on: bool,
    font: f32,
}

#[derive(Clone, Copy, Debug)]
enum Role {
    Stick { origin: Vec2 },
    Look { start: Vec2, t0: f32, moved: bool },
    Widget(Wid),
    Pointer { start: Vec2, inside: bool, pressed: Option<u32> },
    Scroll,
    /// Doigt dans la liste d'apps (appui long sur Nouvelle).
    List { start: Vec2, last_y: f32, moved: bool, slot: Option<usize> },
}

#[derive(Resource, Default)]
pub struct TouchCtl {
    /// Contrôles affichés / actifs.
    pub enabled: bool,
    forced: Option<bool>,
    azerty: bool,
    /// Joystick : x = droite, y = avant, dans [-1, 1].
    pub move_axis: Vec2,
    /// Déplacement du regard sur cette frame (px logiques).
    pub look_delta: Vec2,
    pub sprint: bool,
    pub arm_up: bool,
    /// Zoom de l'app tenue (1.0 = normal).
    pub zoom: f32,
    /// Liste d'apps installees (appui long sur Nouvelle).
    list_open: bool,
    list_scroll: f32,
    new_press: Option<f32>,
    apps: Vec<AppEntry>,
    pub acts: Vec<Act>,
    pending: Vec<Act>,
    kb_open: bool,
    right_once: bool,
    /// Maj, Ctrl, Alt collés (relâchés après la prochaine touche).
    latched: [bool; 3],
    roles: HashMap<u64, Role>,
    widgets: Vec<Widget>,
    stick: Option<(Vec2, Vec2)>,
    unit: f32,
}

#[derive(Clone, Debug)]
pub struct AppEntry {
    name: String,
    exec: String,
}

const LIST_ROWS: usize = 8;

/// Rectangle de la liste + hauteur d'une ligne (la premiere ligne est le titre).
fn list_geom(win: Vec2) -> (Rect, f32) {
    let rh = (win.y * 0.84 / (LIST_ROWS as f32 + 1.0)).clamp(34.0, 80.0);
    let w = (win.x * 0.7).min(900.0);
    let h = rh * (LIST_ROWS as f32 + 1.0);
    (Rect::from_center_size(win / 2.0, Vec2::new(w, h)), rh)
}

fn list_slot_at(win: Vec2, pos: Vec2, top: usize, len: usize) -> Option<usize> {
    let (panel, rh) = list_geom(win);
    let s = ((pos.y - panel.min.y) / rh).floor() as i32 - 1;
    if s < 0 || s as usize >= LIST_ROWS {
        return None;
    }
    let i = top + s as usize;
    (i < len).then_some(i)
}

/// Lit les lanceurs .desktop (applications visibles, triees par nom).
fn load_apps() -> Vec<AppEntry> {
    let mut dirs: Vec<std::path::PathBuf> = vec![
        "/usr/share/applications".into(),
        "/usr/local/share/applications".into(),
        "/var/lib/snapd/desktop/applications".into(),
    ];
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(h).join(".local/share/applications"));
    }
    let mut out: Vec<AppEntry> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(txt) = std::fs::read_to_string(&p) else { continue };
            let mut name: Option<String> = None;
            let mut exec: Option<String> = None;
            let (mut skip, mut term, mut main) = (false, false, false);
            for line in txt.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    main = l == "[Desktop Entry]";
                    continue;
                }
                if !main {
                    continue;
                }
                if let Some(v) = l.strip_prefix("Name=") {
                    name.get_or_insert(v.to_string());
                } else if let Some(v) = l.strip_prefix("Exec=") {
                    exec.get_or_insert(v.to_string());
                } else if l == "NoDisplay=true" || l == "Hidden=true" {
                    skip = true;
                } else if l == "Terminal=true" {
                    term = true;
                } else if let Some(v) = l.strip_prefix("Type=") {
                    if v != "Application" {
                        skip = true;
                    }
                }
            }
            let (Some(name), Some(exec)) = (name, exec) else { continue };
            if skip {
                continue;
            }
            let mut cmd = String::new();
            for tok in exec.split_whitespace() {
                if tok.len() == 2 && tok.starts_with('%') {
                    continue;
                }
                if !cmd.is_empty() {
                    cmd.push(' ');
                }
                cmd.push_str(tok);
            }
            if cmd.is_empty() {
                continue;
            }
            if (cmd.contains("chromium") || cmd.contains("chrome")) && !cmd.contains("ozone-platform") {
                cmd.push_str(" --ozone-platform=wayland");
            }
            if term {
                cmd = format!("foot -e {cmd}");
            }
            if !out.iter().any(|a| a.name == name) {
                out.push(AppEntry { name, exec: cmd });
            }
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

const MODS: [u32; 3] = [42, 29, 56];

fn mod_index(code: u32) -> Option<usize> {
    MODS.iter().position(|&m| m == code)
}

const COL_IDLE: Color = Color::srgba(0.10, 0.08, 0.06, 0.55);
const COL_DOWN: Color = Color::srgba(0.85, 0.42, 0.08, 0.85);
const COL_ON: Color = Color::srgba(0.55, 0.28, 0.06, 0.78);
const COL_BG: Color = Color::srgba(0.06, 0.05, 0.04, 0.96);
const COL_TEXT: Color = Color::srgb(0.95, 0.9, 0.8);
const COL_ACCENT: Color = Color::srgb(0.95, 0.5, 0.12);

#[derive(Component)]
struct TouchWidget(Wid);
#[derive(Component)]
struct TouchLabel(Wid);
#[derive(Component, PartialEq, Eq)]
enum StickPart {
    Base,
    Knob,
}

pub struct TouchPlugin;

impl Plugin for TouchPlugin {
    fn build(&self, app: &mut App) {
        let forced = match std::env::var("WAYWORLD_TOUCH").as_deref() {
            Ok("1") => Some(true),
            Ok("0") => Some(false),
            _ => None,
        };
        let azerty = match std::env::var("WAYWORLD_KB").as_deref() {
            Ok("azerty") => true,
            Ok("qwerty") => false,
            _ => std::env::var("XKB_DEFAULT_LAYOUT").map(|l| l.starts_with("fr")).unwrap_or(false),
        };
        app.insert_resource(TouchCtl { enabled: forced == Some(true), forced, azerty, zoom: 1.0, ..default() })
            .add_systems(Startup, spawn_controls)
            .add_systems(Update, (touch_input, update_widgets, held_layout).chain());
    }
}

// ---------------------------------------------------------------------------
// Disposition
// ---------------------------------------------------------------------------

fn unit(win: Vec2) -> f32 {
    (win.min_element() * 0.13).clamp(52.0, 96.0)
}

fn rows() -> Vec<Vec<(u32, f32)>> {
    let keys = |a: u32, b: u32| (a..=b).map(|c| (c, 1.0f32)).collect::<Vec<_>>();
    let mut r0: Vec<(u32, f32)> = vec![(1, 1.4)];
    r0.extend(keys(2, 13));
    r0.push((14, 1.8));
    let mut r1: Vec<(u32, f32)> = vec![(15, 1.5)];
    r1.extend(keys(16, 27));
    r1.push((28, 1.7));
    let mut r2: Vec<(u32, f32)> = keys(30, 40);
    r2.push((43, 1.0));
    let mut r3: Vec<(u32, f32)> = vec![(42, 2.0)];
    r3.extend(keys(44, 53));
    r3.push((103, 1.2));
    let r4: Vec<(u32, f32)> = vec![(29, 1.5), (56, 1.5), (57, 6.0), (105, 1.2), (108, 1.2), (106, 1.2)];
    vec![r0, r1, r2, r3, r4]
}

fn key_label(code: u32, azerty: bool) -> String {
    let special = match code {
        1 => "Esc",
        14 => "Eff",
        15 => "Tab",
        28 => "Ent",
        29 => "Ctrl",
        42 => "Maj",
        56 => "Alt",
        57 => "Espace",
        103 => "^",
        105 => "<",
        106 => ">",
        108 => "v",
        _ => "",
    };
    if !special.is_empty() {
        return special.to_string();
    }
    let (q, a, base): (&str, &str, u32) = match code {
        2..=13 => ("1234567890-=", "&é\"'(-è_çà)=", 2),
        16..=27 => ("qwertyuiop[]", "azertyuiop^$", 16),
        30..=41 => ("asdfghjkl;'`", "qsdfghjklmù²", 30),
        44..=53 => ("zxcvbnm,./", "wxcvbn,;:!", 44),
        43 => return (if azerty { "*" } else { "\\" }).to_string(),
        _ => return String::new(),
    };
    let s = if azerty { a } else { q };
    s.chars().nth((code - base) as usize).map(|c| c.to_string()).unwrap_or_default()
}

fn layout(win: Vec2, mode: Mode, ctl: &TouchCtl) -> Vec<Widget> {
    let u = unit(win);
    let m = 0.25 * u;
    let font = (u * 0.24).clamp(13.0, 24.0);
    let mut v: Vec<Widget> = Vec::new();
    if ctl.list_open && matches!(mode, Mode::Explore) {
        let (panel, rh) = list_geom(win);
        v.push(Widget { id: Wid::ListBg, rect: panel, label: String::new(), on: false, font });
        let title = format!("Applications ({}) - glisser pour defiler", ctl.apps.len());
        v.push(Widget {
            id: Wid::ListTitle,
            rect: Rect::from_corners(panel.min, Vec2::new(panel.max.x, panel.min.y + rh)),
            label: title,
            on: false,
            font,
        });
        let top = ctl.list_scroll as usize;
        let ifont = (rh * 0.36).clamp(13.0, 24.0);
        for s in 0..LIST_ROWS {
            let Some(e) = ctl.apps.get(top + s) else { break };
            let y0 = panel.min.y + rh * (s as f32 + 1.0);
            v.push(Widget {
                id: Wid::AppItem(s as u32),
                rect: Rect::from_corners(Vec2::new(panel.min.x + 6.0, y0 + 2.0), Vec2::new(panel.max.x - 6.0, y0 + rh - 2.0)),
                label: e.name.clone(),
                on: false,
                font: ifont,
            });
        }
        return v;
    }
    let mut add = |id: Wid, rect: Rect, label: &str, on: bool, font: f32| {
        v.push(Widget { id, rect, label: label.to_string(), on, font });
    };
    match mode {
        Mode::Explore => {
            add(
                Wid::Menu,
                Rect::from_corners(Vec2::new(win.x - m - 1.3 * u, m), Vec2::new(win.x - m, m + 0.8 * u)),
                "Menu",
                false,
                font,
            );
            let b = |right: f32, bottom: f32, size: f32| {
                Rect::from_center_size(Vec2::new(win.x - right * u, win.y - bottom * u), Vec2::splat(size * u))
            };
            add(Wid::Use, b(1.3, 1.4, 1.5), "Utiliser", false, font);
            add(Wid::Pin, b(3.0, 1.0, 1.1), "Epingler", false, font);
            add(Wid::Close, b(1.3, 3.2, 1.1), "Fermer", false, font);
            add(Wid::NewApp, b(3.0, 2.6, 1.1), "Nouvelle", false, font);
            add(Wid::ArmUp, b(4.7, 1.0, 1.1), "Bras", ctl.arm_up, font);
            add(Wid::Sprint, b(4.7, 2.4, 1.1), "Courir", ctl.sprint, font);
        }
        Mode::Using(_) => {
            let (bw, bh) = (1.7 * u, 0.75 * u);
            let top = |i: f32| {
                let x1 = win.x - m - i * (m + bw);
                Rect::from_corners(Vec2::new(x1 - bw, m), Vec2::new(x1, m + bh))
            };
            add(Wid::Leave, top(0.0), "Quitter", false, font);
            add(Wid::KbToggle, top(1.0), "Clavier", ctl.kb_open, font);
            add(Wid::RightClick, top(2.0), "Clic D", ctl.right_once, font);
            let row2 = |i: f32| {
                let r = top(i);
                let dy = Vec2::new(0.0, bh + m);
                Rect::from_corners(r.min + dy, r.max + dy)
            };
            add(Wid::ZoomIn, row2(0.0), "Zoom +", false, font);
            add(Wid::ZoomReset, row2(1.0), &format!("{}%", (ctl.zoom * 100.0).round() as i32), false, font);
            add(Wid::ZoomOut, row2(2.0), "Zoom -", false, font);
            if ctl.kb_open {
                let rh = (win.y * 0.4 / 5.0).clamp(34.0, 78.0);
                let kw = (win.x * 0.98).min(1500.0);
                let x0 = (win.x - kw) / 2.0;
                let y0 = win.y * 0.99 - rh * 5.0;
                let kfont = (rh * 0.36).clamp(12.0, 22.0);
                for (ri, row) in rows().iter().enumerate() {
                    let total: f32 = row.iter().map(|k| k.1).sum();
                    let mut x = x0;
                    for &(code, wt) in row {
                        let w = kw * wt / total;
                        let rect = Rect::from_corners(
                            Vec2::new(x + 2.0, y0 + ri as f32 * rh + 2.0),
                            Vec2::new(x + w - 2.0, y0 + (ri + 1) as f32 * rh - 2.0),
                        );
                        x += w;
                        let on = mod_index(code).map_or(false, |i| ctl.latched[i]);
                        add(Wid::Key(code), rect, &key_label(code, ctl.azerty), on, kfont);
                    }
                }
            }
        }
    }
    v
}

// ---------------------------------------------------------------------------
// Interface (nœuds UI)
// ---------------------------------------------------------------------------

fn spawn_controls(mut commands: Commands) {
    let mut ids = vec![
        Wid::Menu,
        Wid::Use,
        Wid::Pin,
        Wid::Close,
        Wid::NewApp,
        Wid::ArmUp,
        Wid::Sprint,
        Wid::Leave,
        Wid::KbToggle,
        Wid::RightClick,
        Wid::ZoomIn,
        Wid::ZoomOut,
        Wid::ZoomReset,
        Wid::ListBg,
        Wid::ListTitle,
    ];
    ids.extend((0..LIST_ROWS as u32).map(Wid::AppItem));
    for row in rows() {
        for (code, _) in row {
            ids.push(Wid::Key(code));
        }
    }
    for id in ids {
        commands
            .spawn((
                TouchWidget(id),
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(px(2)),
                    border_radius: BorderRadius::all(px(10)),
                    ..default()
                },
                BackgroundColor(COL_IDLE),
                BorderColor::all(COL_ACCENT),
                GlobalZIndex(match id {
                    Wid::ListBg => 21,
                    Wid::ListTitle | Wid::AppItem(_) => 22,
                    _ => 20,
                }),
            ))
            .with_children(|b| {
                b.spawn((
                    TouchLabel(id),
                    Text::new(""),
                    TextFont { font_size: FontSize::Px(20.0), ..default() },
                    TextColor(COL_TEXT),
                ));
            });
    }
    for part in [StickPart::Base, StickPart::Knob] {
        let (bg, border) = if part == StickPart::Base {
            (Color::srgba(1.0, 1.0, 1.0, 0.06), Color::srgba(1.0, 1.0, 1.0, 0.35))
        } else {
            (Color::srgba(1.0, 1.0, 1.0, 0.30), Color::srgba(1.0, 1.0, 1.0, 0.0))
        };
        commands.spawn((
            part,
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                border: UiRect::all(px(3)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(bg),
            BorderColor::all(border),
            GlobalZIndex(19),
        ));
    }
}

fn update_widgets(
    ctl: Res<TouchCtl>,
    mut nodes: Query<(&TouchWidget, &mut Node, &mut BackgroundColor)>,
    mut labels: Query<(&TouchLabel, &mut Text, &mut TextFont)>,
    mut stick: Query<(&StickPart, &mut Node), Without<TouchWidget>>,
) {
    let by_id: HashMap<Wid, &Widget> = ctl.widgets.iter().map(|w| (w.id, w)).collect();
    let down: HashSet<Wid> = ctl
        .roles
        .values()
        .filter_map(|r| if let Role::Widget(id) = r { Some(*id) } else { None })
        .collect();
    for (tw, mut node, mut bg) in &mut nodes {
        match by_id.get(&tw.0) {
            Some(w) => {
                let size = w.rect.size();
                let (l, t, wd, h) = (px(w.rect.min.x), px(w.rect.min.y), px(size.x), px(size.y));
                if node.display != Display::Flex || node.left != l || node.top != t || node.width != wd || node.height != h {
                    node.display = Display::Flex;
                    node.left = l;
                    node.top = t;
                    node.width = wd;
                    node.height = h;
                }
                let want = if w.id == Wid::ListBg {
                    COL_BG
                } else if down.contains(&w.id) {
                    COL_DOWN
                } else if w.on {
                    COL_ON
                } else {
                    COL_IDLE
                };
                if bg.0 != want {
                    bg.0 = want;
                }
            }
            None => {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
    for (tl, mut text, mut font) in &mut labels {
        if let Some(w) = by_id.get(&tl.0) {
            if text.0 != w.label {
                text.0 = w.label.clone();
            }
            let fs = FontSize::Px(w.font);
            if font.font_size != fs {
                font.font_size = fs;
            }
        }
    }
    let u = ctl.unit;
    for (part, mut node) in &mut stick {
        let want = match (ctl.stick, part) {
            (Some((origin, _)), StickPart::Base) => Some((origin, 2.4 * u)),
            (Some((_, pos)), StickPart::Knob) => Some((pos, 1.0 * u)),
            _ => None,
        };
        match want {
            Some((c, size)) => {
                let (l, t, s) = (px(c.x - size / 2.0), px(c.y - size / 2.0), px(size));
                if node.display != Display::Flex || node.left != l || node.top != t || node.width != s || node.height != s {
                    node.display = Display::Flex;
                    node.left = l;
                    node.top = t;
                    node.width = s;
                    node.height = s;
                }
            }
            None => {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
}

/// Pendant l'utilisation : zoom de l'app tenue (suit le pointeur quand on zoome) ; avec le
/// clavier à l'écran elle monte et rétrécit.
fn held_layout(mut ctl: ResMut<TouchCtl>, mode: Res<Mode>, cpx: Res<CursorPx>, mut q: Query<(&AppWin, &mut Transform)>) {
    if !matches!(*mode, Mode::Using(_)) {
        if ctl.zoom != 1.0 {
            ctl.zoom = 1.0;
        }
        return;
    }
    let (ty, base) = if ctl.enabled && ctl.kb_open { (0.30, 0.5) } else { (-0.02, 0.8) };
    let ts = base * ctl.zoom;
    let k = (ctl.zoom - 1.0).clamp(0.0, 1.0);
    for (w, mut tf) in &mut q {
        if w.place != Place::Held {
            continue;
        }
        let n = (cpx.0 / w.px.as_vec2()).clamp(Vec2::ZERO, Vec2::ONE) - Vec2::splat(0.5);
        let tx = -n.x * w.width() * ts * k;
        let ty = ty + n.y * PANEL_H * ts * k;
        let dx = tx - tf.translation.x;
        let dy = ty - tf.translation.y;
        let ds = ts - tf.scale.x;
        if dx.abs() > 1e-3 || dy.abs() > 1e-3 || ds.abs() > 1e-3 {
            tf.translation.x += dx * 0.25;
            tf.translation.y += dy * 0.25;
            tf.scale = Vec3::splat(tf.scale.x + ds * 0.25);
        }
    }
}

// ---------------------------------------------------------------------------
// Entrées
// ---------------------------------------------------------------------------

/// Position écran -> pixel de l'app, et `true` si le doigt est bien sur l'app.
fn screen_to_px(camera: &Camera, cam_gt: &GlobalTransform, screen: Vec2, win_gt: &GlobalTransform, win: &AppWin) -> Option<(Vec2, bool)> {
    let ray = camera.viewport_to_world(cam_gt, screen).ok()?;
    let d: Vec3 = *ray.direction;
    let n: Vec3 = *win_gt.back();
    let denom = d.dot(n);
    if denom.abs() < 1e-5 {
        return None;
    }
    let t = (win_gt.translation() - ray.origin).dot(n) / denom;
    if t <= 0.0 {
        return None;
    }
    let local = win_gt.affine().inverse().transform_point3(ray.origin + d * t);
    let (pw, ph) = (win.width(), PANEL_H);
    let inside = local.x.abs() <= pw / 2.0 && local.y.abs() <= ph / 2.0;
    let px = Vec2::new((local.x / pw + 0.5) * win.px.x as f32, (0.5 - local.y / ph) * win.px.y as f32);
    Some((px.clamp(Vec2::ZERO, win.px.as_vec2() - Vec2::ONE), inside))
}

fn press_widget(id: Wid, ctl: &mut TouchCtl, acts: &mut Vec<Act>) {
    match id {
        Wid::Menu => acts.push(Act::Menu),
        Wid::Use => acts.push(Act::Use),
        Wid::Pin => acts.push(Act::Pin),
        Wid::Close => acts.push(Act::Close),
        Wid::NewApp | Wid::ListBg | Wid::ListTitle | Wid::AppItem(_) => {}
        Wid::Leave => acts.push(Act::Leave),
        Wid::ArmUp => ctl.arm_up = !ctl.arm_up,
        Wid::Sprint => ctl.sprint = !ctl.sprint,
        Wid::KbToggle => ctl.kb_open = !ctl.kb_open,
        Wid::RightClick => ctl.right_once = !ctl.right_once,
        Wid::ZoomIn => ctl.zoom = (ctl.zoom * 1.25).min(3.0),
        Wid::ZoomOut => ctl.zoom = (ctl.zoom / 1.25).max(0.5),
        Wid::ZoomReset => ctl.zoom = 1.0,
        Wid::Key(code) => match mod_index(code) {
            Some(i) => ctl.latched[i] = !ctl.latched[i],
            None => {
                for (i, m) in MODS.iter().enumerate() {
                    if ctl.latched[i] {
                        acts.push(Act::Key { code: *m, pressed: true });
                    }
                }
                acts.push(Act::Key { code, pressed: true });
            }
        },
    }
}

fn release_widget(id: Wid, ctl: &mut TouchCtl, acts: &mut Vec<Act>) {
    if id == Wid::NewApp && ctl.new_press.take().is_some() {
        acts.push(Act::NewApp);
    }
    if let Wid::Key(code) = id {
        if mod_index(code).is_none() {
            acts.push(Act::Key { code, pressed: false });
            for (i, m) in MODS.iter().enumerate() {
                if ctl.latched[i] {
                    acts.push(Act::Key { code: *m, pressed: false });
                    ctl.latched[i] = false;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn touch_input(
    mut ctl: ResMut<TouchCtl>,
    touches: Res<Touches>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<State<GameState>>,
    mode: Res<Mode>,
    bridge: Res<crate::apps::Bridge>,
    window: Single<&Window, With<PrimaryWindow>>,
    cam: Single<(&Camera, &GlobalTransform), With<PlayerCam>>,
    wins: Query<(&AppWin, &GlobalTransform)>,
) {
    let t_now = time.elapsed_secs();
    let mut acts: Vec<Act> = std::mem::take(&mut ctl.pending);
    let mut roles = std::mem::take(&mut ctl.roles);
    ctl.look_delta = Vec2::ZERO;
    ctl.move_axis = Vec2::ZERO;
    ctl.stick = None;

    // affichage : au premier toucher, masqué dès qu'on touche au clavier / à la souris
    match ctl.forced {
        Some(f) => ctl.enabled = f,
        None => {
            if touches.any_just_pressed() {
                ctl.enabled = true;
            } else if keys.get_just_pressed().next().is_some() || mouse.get_just_pressed().next().is_some() {
                ctl.enabled = false;
            }
        }
    }
    if !ctl.enabled || *state.get() != GameState::Playing {
        roles.clear();
        ctl.widgets.clear();
        ctl.roles = roles;
        ctl.pending = acts;
        return;
    }

    let win = Vec2::new(window.width(), window.height());
    let u = unit(win);
    ctl.unit = u;
    if !matches!(*mode, Mode::Explore) {
        ctl.list_open = false;
    }
    let widgets = layout(win, *mode, &ctl);

    let (camera, cam_gt) = *cam;
    let held = if let Mode::Using(id) = *mode { wins.iter().find(|(w, _)| w.id == id) } else { None };
    let map = |p: Vec2| -> Option<(Vec2, bool)> {
        held.and_then(|(w, gt)| screen_to_px(camera, cam_gt, p, gt, w))
    };

    // 1. nouveaux doigts
    let new: Vec<(u64, Vec2)> = touches.iter_just_pressed().map(|t| (t.id(), t.position())).collect();
    for (id, pos) in new {
        if ctl.list_open {
            if list_geom(win).0.contains(pos) {
                let slot = list_slot_at(win, pos, ctl.list_scroll as usize, ctl.apps.len());
                roles.insert(id, Role::List { start: pos, last_y: pos.y, moved: false, slot });
            } else {
                ctl.list_open = false;
            }
            continue;
        }
        if let Some(w) = widgets.iter().find(|w| w.rect.contains(pos)) {
            roles.insert(id, Role::Widget(w.id));
            if w.id == Wid::NewApp {
                ctl.new_press = Some(t_now);
            }
            press_widget(w.id, &mut ctl, &mut acts);
            continue;
        }
        match *mode {
            Mode::Explore => {
                let has_stick = roles.values().any(|r| matches!(r, Role::Stick { .. }));
                if pos.x < win.x * 0.42 && !has_stick {
                    roles.insert(id, Role::Stick { origin: pos });
                } else {
                    roles.insert(id, Role::Look { start: pos, t0: t_now, moved: false });
                }
            }
            Mode::Using(_) => {
                let others = roles.values().filter(|r| matches!(r, Role::Pointer { .. } | Role::Scroll)).count();
                if others == 0 {
                    let inside = map(pos).map_or(false, |m| m.1);
                    roles.insert(id, Role::Pointer { start: pos, inside, pressed: None });
                } else {
                    // deuxième doigt : on passe en molette, sans clic
                    for r in roles.values_mut() {
                        if let Role::Pointer { pressed, .. } = *r {
                            if let Some(code) = pressed {
                                acts.push(Act::Btn { code, pressed: false });
                            }
                            *r = Role::Scroll;
                        }
                    }
                    roles.insert(id, Role::Scroll);
                }
            }
        }
    }

    // 2. doigts maintenus
    let mut scroll_sum = Vec2::ZERO;
    let mut scroll_n = 0u32;
    let ids: Vec<u64> = roles.keys().copied().collect();
    for id in ids {
        let Some(t) = touches.get_pressed(id) else { continue };
        let pos = t.position();
        let Some(role) = roles.get_mut(&id) else { continue };
        match role {
            Role::Stick { origin } => {
                let radius = 1.1 * u;
                let v = ((pos - *origin) / radius).clamp_length_max(1.0);
                let axis = if v.length() < 0.12 { Vec2::ZERO } else { v };
                ctl.move_axis = Vec2::new(axis.x, -axis.y);
                ctl.stick = Some((*origin, *origin + v * radius));
            }
            Role::Look { start, moved, .. } => {
                if (pos - *start).length() > 12.0 {
                    *moved = true;
                }
                ctl.look_delta += t.delta();
            }
            Role::Pointer { start, inside, pressed } => {
                if !*inside {
                    continue;
                }
                if pressed.is_none() && (pos - *start).length() > 10.0 {
                    // début de glissement : on appuie à l'endroit où le doigt a commencé
                    let code = if ctl.right_once {
                        ctl.right_once = false;
                        keymap::BTN_RIGHT
                    } else {
                        keymap::BTN_LEFT
                    };
                    if let Some((p, _)) = map(*start) {
                        acts.push(Act::Ptr(p));
                    }
                    acts.push(Act::Btn { code, pressed: true });
                    *pressed = Some(code);
                }
                if pressed.is_some() && t.delta() != Vec2::ZERO {
                    if let Some((p, _)) = map(pos) {
                        acts.push(Act::Ptr(p));
                    }
                }
            }
            Role::Scroll => {
                scroll_sum += t.delta();
                scroll_n += 1;
            }
            Role::Widget(Wid::NewApp) => {
                if let Some(t0) = ctl.new_press {
                    if t_now - t0 > 0.45 {
                        ctl.new_press = None;
                        ctl.apps = load_apps();
                        ctl.list_scroll = 0.0;
                        ctl.list_open = true;
                    }
                }
            }
            Role::List { start, last_y, moved, .. } => {
                if (pos - *start).length() > 12.0 {
                    *moved = true;
                }
                if *moved {
                    let rh = list_geom(win).1;
                    let max = ctl.apps.len().saturating_sub(LIST_ROWS) as f32;
                    ctl.list_scroll = (ctl.list_scroll - (pos.y - *last_y) / rh).clamp(0.0, max);
                }
                *last_y = pos.y;
            }
            Role::Widget(_) => {}
        }
    }
    if scroll_n > 0 {
        let avg = scroll_sum / scroll_n as f32;
        if avg != Vec2::ZERO {
            acts.push(Act::Scroll(avg));
        }
    }

    // 3. doigts levés
    let released: Vec<(u64, Vec2, bool)> = touches
        .iter_just_released()
        .map(|t| (t.id(), t.position(), true))
        .chain(touches.iter_just_canceled().map(|t| (t.id(), t.position(), false)))
        .collect();
    for (id, pos, ok) in released {
        let Some(role) = roles.remove(&id) else { continue };
        match role {
            Role::Look { moved, t0, .. } => {
                if ok && !moved && t_now - t0 < 0.4 {
                    acts.push(Act::Tap(pos));
                }
            }
            Role::Widget(wid) => release_widget(wid, &mut ctl, &mut acts),
            Role::Pointer { start, inside, pressed } => {
                if let Some(code) = pressed {
                    acts.push(Act::Btn { code, pressed: false });
                } else if ok && inside {
                    // simple tap = clic (relâché à la frame suivante)
                    let code = if ctl.right_once {
                        ctl.right_once = false;
                        keymap::BTN_RIGHT
                    } else {
                        keymap::BTN_LEFT
                    };
                    if let Some((p, _)) = map(start) {
                        acts.push(Act::Ptr(p));
                    }
                    acts.push(Act::Btn { code, pressed: true });
                    ctl.pending.push(Act::Btn { code, pressed: false });
                }
            }
            Role::List { moved, slot, .. } => {
                if ok && !moved {
                    if let Some(e) = slot.and_then(|i| ctl.apps.get(i)).cloned() {
                        crate::apps::launch(&e.exec, &bridge.socket);
                        ctl.list_open = false;
                    }
                }
            }
            _ => {}
        }
    }

    ctl.roles = roles;
    ctl.acts = acts;
    ctl.widgets = widgets;
}