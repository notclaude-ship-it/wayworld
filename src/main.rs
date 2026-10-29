//! Wayworld : un environnement de bureau Wayland qui est un jeu.
//! Les apps Wayland flottent dans un monde 3D ; on peut les épingler à son bras.

mod apps;
mod comp;
mod keymap;
mod menu;
mod player;
mod settings;
mod shot;
mod touch;
#[cfg(feature = "vr")]
mod vr;
mod world;

use bevy::{
    image::{ImageAddressMode, ImageFilterMode, ImageSamplerDescriptor},
    prelude::*,
    window::{PresentMode, WindowResolution},
};

#[derive(States, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameState {
    #[default]
    Menu,
    Playing,
    Paused,
}

fn asset_dir() -> String {
    if let Ok(p) = std::env::var("WAYWORLD_ASSETS") {
        return p;
    }
    for p in ["/usr/share/wayworld/assets", "/usr/local/share/wayworld/assets"] {
        if std::path::Path::new(p).is_dir() {
            return p.to_string();
        }
    }
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets").to_string()
}

fn main() {
    let settings = settings::Settings::load();

    // Compositeur Wayland intégré : premier socket libre wayworld-0..9
    let handle = (0..10)
        .find_map(|i| comp::spawn(&format!("wayworld-{i}"), (1024, 640)).ok())
        .expect("impossible de créer un socket Wayland (XDG_RUNTIME_DIR défini ?)");
    println!("[wayworld] les apps se connectent via WAYLAND_DISPLAY={}", handle.socket);

    let fullscreen = settings.fullscreen;
    #[cfg(feature = "vr")]
    let vr_on = settings.vr || std::env::var("WAYWORLD_VR").map(|v| v == "1").unwrap_or(false);
    let plugins = DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: "Wayworld".into(),
                resolution: WindowResolution::new(1600, 900),
                present_mode: PresentMode::AutoVsync,
                mode: if fullscreen {
                    bevy::window::WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Primary)
                } else {
                    bevy::window::WindowMode::Windowed
                },
                ..default()
            }),
            ..default()
        })
        .set(AssetPlugin { file_path: asset_dir(), ..default() })
        .set(ImagePlugin {
            default_sampler: ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                address_mode_w: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                anisotropy_clamp: 8,
                ..default()
            },
        });
    // VR : le plugin OpenXR remplace le rendu ; sans runtime OpenXR on reste en écran plat.
    #[cfg(feature = "vr")]
    let plugins = if vr_on {
        bevy_mod_openxr::add_xr_plugins(plugins.disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>())
    } else {
        plugins
    };
    let mut app = App::new();
    app.insert_resource(settings)
        .insert_resource(apps::Bridge {
            rx: std::sync::Mutex::new(handle.rx),
            tx: handle.tx,
            socket: handle.socket,
        })
        .add_plugins(plugins)
        .add_plugins((world::WorldPlugin, player::PlayerPlugin, apps::AppsPlugin, menu::MenuPlugin, touch::TouchPlugin));
    #[cfg(feature = "vr")]
    if vr_on {
        app.add_plugins(vr::VrPlugin);
    }
    if let Ok(dir) = std::env::var("WAYWORLD_SHOT_DIR") {
        let _ = std::fs::create_dir_all(&dir);
        app.add_plugins(shot::ShotPlugin(dir));
    }
    app.run();
}
