//! Menu de session (Jouer / Paramètres / Quitter), pause, HUD.

use bevy::{
    prelude::*,
    window::{MonitorSelection, PrimaryWindow, WindowMode},
};

use crate::{
    apps::{launch, AppWin, Bridge, HudInfo},
    player::{Mode, Player},
    settings::{Settings, APP_PRESETS},
    GameState,
};

const COL_BG: Color = Color::srgba(0.02, 0.02, 0.03, 0.78);
const COL_BTN: Color = Color::srgb(0.16, 0.12, 0.08);
const COL_BTN_HOVER: Color = Color::srgb(0.32, 0.2, 0.08);
const COL_BTN_PRESS: Color = Color::srgb(0.85, 0.42, 0.08);
const COL_TEXT: Color = Color::srgb(0.95, 0.9, 0.8);
const COL_ACCENT: Color = Color::srgb(0.95, 0.5, 0.12);

#[derive(Resource, Default, PartialEq, Eq, Clone, Copy)]
enum Screen {
    #[default]
    Main,
    Settings,
}

/// Vrai quand on vient de cliquer sur Jouer depuis le menu principal.
#[derive(Resource, Default)]
pub struct Fresh(pub bool);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Sensitivity,
    Fov,
    Fog,
    Speed,
}

#[derive(Component, Clone, Copy, Debug)]
enum Action {
    Play,
    Resume,
    OpenSettings,
    Back,
    Quit,
    Adjust(Field, i32),
    ToggleFullscreen,
    ToggleShadows,
    CycleApp(i32),
    BackToMainMenu,
}

#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct HudRoot;
#[derive(Component)]
struct PromptText;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<Screen>()
            .init_resource::<Fresh>()
            .add_systems(OnEnter(GameState::Menu), reset_screen)
            .add_systems(OnEnter(GameState::Paused), reset_screen)
            .add_systems(OnEnter(GameState::Playing), (spawn_hud, begin_game))
            .add_systems(OnExit(GameState::Playing), despawn_hud)
            .add_systems(
                Update,
                (rebuild_menu, buttons, apply_window_settings)
                    .run_if(in_state(GameState::Menu).or_else(in_state(GameState::Paused))),
            )
            .add_systems(Update, update_prompt.run_if(in_state(GameState::Playing)))
            .add_systems(OnExit(GameState::Menu), despawn_menu)
            .add_systems(OnExit(GameState::Paused), despawn_menu);
    }
}

fn reset_screen(mut s: ResMut<Screen>) {
    *s = Screen::Main;
}

fn despawn_menu(mut commands: Commands, q: Query<Entity, With<MenuRoot>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn despawn_hud(mut commands: Commands, q: Query<Entity, With<HudRoot>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

/// Au premier "Jouer" : on place le joueur et on lance l'app de démarrage.
fn begin_game(
    mut fresh: ResMut<Fresh>,
    mut mode: ResMut<Mode>,
    settings: Res<Settings>,
    bridge: Res<Bridge>,
    mut player: Single<(&mut Player, &mut Transform)>,
    wins: Query<&AppWin>,
) {
    if !fresh.0 {
        return;
    }
    fresh.0 = false;
    *mode = Mode::Explore;
    let (p, tf) = &mut *player;
    p.yaw = 0.0;
    p.pitch = 0.0;
    tf.translation = Vec3::new(0.0, 0.0, 5.0);
    tf.rotation = Quat::IDENTITY;
    if wins.iter().count() == 0 {
        launch(settings.startup_command(), &bridge.socket);
    }
}

fn spawn_hud(mut commands: Commands) {
    commands
        .spawn((
            HudRoot,
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|p| {
            // point de visée
            p.spawn((
                Node { width: px(6), height: px(6), border_radius: BorderRadius::MAX, ..default() },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
            ));
            // aide en bas
            p.spawn(Node {
                position_type: PositionType::Absolute,
                bottom: px(24),
                width: percent(100),
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_children(|b| {
                b.spawn((
                    PromptText,
                    Text::new(""),
                    TextFont { font_size: FontSize::Px(20.0), ..default() },
                    TextColor(COL_TEXT),
                ));
            });
        });
}

fn update_prompt(hud: Res<HudInfo>, mut q: Query<&mut Text, With<PromptText>>) {
    for mut t in &mut q {
        if t.0 != hud.0 {
            t.0 = hud.0.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

fn button(p: &mut ChildSpawnerCommands, label: &str, action: Action, width: f32) {
    p.spawn((
        Button,
        action,
        Node {
            width: px(width),
            height: px(48),
            margin: UiRect::all(px(5)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(px(2)),
            ..default()
        },
        BorderColor::all(COL_ACCENT),
        BackgroundColor(COL_BTN),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new(label),
            TextFont { font_size: FontSize::Px(22.0), ..default() },
            TextColor(COL_TEXT),
        ));
    });
}

fn label(p: &mut ChildSpawnerCommands, text: &str, size: f32, color: Color) {
    p.spawn((
        Text::new(text),
        TextFont { font_size: FontSize::Px(size), ..default() },
        TextColor(color),
        Node { margin: UiRect::all(px(4)), ..default() },
    ));
}

fn setting_row(p: &mut ChildSpawnerCommands, name: &str, value: String, minus: Action, plus: Action) {
    p.spawn(Node {
        width: px(560),
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|r| {
        r.spawn((Text::new(name), TextFont { font_size: FontSize::Px(22.0), ..default() }, TextColor(COL_TEXT)));
        r.spawn(Node { align_items: AlignItems::Center, ..default() }).with_children(|c| {
            button(c, "−", minus, 48.0);
            c.spawn((
                Text::new(value),
                TextFont { font_size: FontSize::Px(22.0), ..default() },
                TextColor(COL_ACCENT),
                Node { width: px(150), justify_content: JustifyContent::Center, ..default() },
            ));
            button(c, "+", plus, 48.0);
        });
    });
}

fn rebuild_menu(
    mut commands: Commands,
    screen: Res<Screen>,
    settings: Res<Settings>,
    state: Res<State<GameState>>,
    old: Query<Entity, With<MenuRoot>>,
) {
    let exists = old.iter().next().is_some();
    if exists && !screen.is_changed() && !settings.is_changed() {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    let paused = *state.get() == GameState::Paused;
    commands
        .spawn((
            MenuRoot,
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(px(60), px(40)),
                    border: UiRect::all(px(2)),
                    ..default()
                },
                BackgroundColor(COL_BG),
                BorderColor::all(Color::srgba(0.95, 0.5, 0.12, 0.5)),
            ))
            .with_children(|p| {
                label(p, "WAYWORLD", 64.0, COL_ACCENT);
                label(p, "Ton bureau Linux, en monde ouvert", 20.0, COL_TEXT);
                p.spawn(Node { height: px(24), ..default() });
                match *screen {
                    Screen::Main => {
                        if paused {
                            button(p, "REPRENDRE", Action::Resume, 320.0);
                        } else {
                            button(p, "JOUER", Action::Play, 320.0);
                        }
                        button(p, "PARAMÈTRES", Action::OpenSettings, 320.0);
                        if paused {
                            button(p, "MENU PRINCIPAL", Action::BackToMainMenu, 320.0);
                        }
                        button(p, "QUITTER", Action::Quit, 320.0);
                    }
                    Screen::Settings => {
                        setting_row(
                            p,
                            "Sensibilité souris",
                            format!("{:.1}", settings.sensitivity * 1000.0),
                            Action::Adjust(Field::Sensitivity, -1),
                            Action::Adjust(Field::Sensitivity, 1),
                        );
                        setting_row(
                            p,
                            "Champ de vision",
                            format!("{:.0}°", settings.fov),
                            Action::Adjust(Field::Fov, -1),
                            Action::Adjust(Field::Fov, 1),
                        );
                        setting_row(
                            p,
                            "Brouillard (visibilité)",
                            format!("{:.0} m", settings.fog_visibility),
                            Action::Adjust(Field::Fog, -1),
                            Action::Adjust(Field::Fog, 1),
                        );
                        setting_row(
                            p,
                            "Vitesse de marche",
                            format!("{:.1} m/s", settings.move_speed),
                            Action::Adjust(Field::Speed, -1),
                            Action::Adjust(Field::Speed, 1),
                        );
                        setting_row(
                            p,
                            "App au démarrage",
                            APP_PRESETS[settings.startup_app % APP_PRESETS.len()].0.to_string(),
                            Action::CycleApp(-1),
                            Action::CycleApp(1),
                        );
                        button(
                            p,
                            if settings.fullscreen { "Plein écran : OUI" } else { "Plein écran : NON" },
                            Action::ToggleFullscreen,
                            360.0,
                        );
                        button(
                            p,
                            if settings.shadows { "Ombres : OUI" } else { "Ombres : NON" },
                            Action::ToggleShadows,
                            360.0,
                        );
                        button(p, "RETOUR", Action::Back, 320.0);
                    }
                }
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn buttons(
    mut q: Query<(&Interaction, &Action, &mut BackgroundColor), (Changed<Interaction>, With<Button>)>,
    mut screen: ResMut<Screen>,
    mut settings: ResMut<Settings>,
    mut next: ResMut<NextState<GameState>>,
    mut fresh: ResMut<Fresh>,
    mut exit: MessageWriter<AppExit>,
) {
    for (inter, action, mut bg) in &mut q {
        match inter {
            Interaction::Hovered => *bg = COL_BTN_HOVER.into(),
            Interaction::None => *bg = COL_BTN.into(),
            Interaction::Pressed => {
                *bg = COL_BTN_PRESS.into();
                match *action {
                    Action::Play => {
                        fresh.0 = true;
                        next.set(GameState::Playing);
                    }
                    Action::Resume => next.set(GameState::Playing),
                    Action::OpenSettings => *screen = Screen::Settings,
                    Action::Back => {
                        settings.save();
                        *screen = Screen::Main;
                    }
                    Action::BackToMainMenu => next.set(GameState::Menu),
                    Action::Quit => {
                        settings.save();
                        exit.write(AppExit::Success);
                    }
                    Action::Adjust(f, dir) => {
                        let d = dir as f32;
                        match f {
                            Field::Sensitivity => settings.sensitivity = (settings.sensitivity + d * 0.0003).clamp(0.0004, 0.01),
                            Field::Fov => settings.fov = (settings.fov + d * 5.0).clamp(55.0, 120.0),
                            Field::Fog => settings.fog_visibility = (settings.fog_visibility + d * 4.0).clamp(8.0, 80.0),
                            Field::Speed => settings.move_speed = (settings.move_speed + d * 0.4).clamp(1.5, 8.0),
                        }
                    }
                    Action::ToggleFullscreen => settings.fullscreen = !settings.fullscreen,
                    Action::ToggleShadows => settings.shadows = !settings.shadows,
                    Action::CycleApp(dir) => {
                        let n = APP_PRESETS.len() as i32;
                        settings.startup_app = ((settings.startup_app as i32 + dir).rem_euclid(n)) as usize;
                    }
                }
            }
        }
    }
}

fn apply_window_settings(settings: Res<Settings>, mut window: Single<&mut Window, With<PrimaryWindow>>) {
    if !settings.is_changed() {
        return;
    }
    let want = if settings.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    if window.mode != want {
        window.mode = want;
    }
}