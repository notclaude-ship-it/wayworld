//! Outil de dev : WAYWORLD_SHOT_DIR=/tmp/x → prend des captures (menu, monde, bras) puis quitte.

use bevy::{
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot},
};

use crate::{apps, menu::Fresh, GameState};

#[derive(Resource)]
pub struct ShotDir(pub String);

pub struct ShotPlugin(pub String);

impl Plugin for ShotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShotDir(self.0.clone())).add_systems(Update, shots);
    }
}

fn shots(
    mut commands: Commands,
    mut frame: Local<u32>,
    dir: Res<ShotDir>,
    mut next: ResMut<NextState<GameState>>,
    mut fresh: ResMut<Fresh>,
    mut arm: ResMut<apps::ArmForce>,
    bridge: Res<apps::Bridge>,
    mut exit: MessageWriter<AppExit>,
) {
    *frame += 1;
    let snap = |commands: &mut Commands, name: &str| {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(format!("{}/{name}.png", dir.0)));
    };
    match *frame {
        120 => snap(&mut commands, "1_menu"),
        140 => {
            fresh.0 = true;
            next.set(GameState::Playing);
        }
        150 => apps::launch("foot", &bridge.socket),
        420 => snap(&mut commands, "2_monde"),
        430 => arm.0 = true,
        450 => snap(&mut commands, "3_bras"),
        520 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
