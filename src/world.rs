//! Le monde : un hébergement industriel sombre façon Lethal Company / R.E.P.O.
//! Hall central + couloir + atelier, textures PBR et props Poly Haven (CC0).

use bevy::{image::ImageLoaderSettings, math::Affine2, prelude::*};
use std::f32::consts::{PI, TAU};

use crate::settings::Settings;

pub const HEIGHT: f32 = 4.0;
const WALL_T: f32 = 0.3;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.55, 0.6, 0.85),
            brightness: 55.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.01, 0.012, 0.018)))
        .add_systems(Startup, build_world)
        .add_systems(Update, (flicker, apply_shadow_setting));
    }
}

/// Zones où le joueur peut marcher (rectangles XZ qui se touchent aux portes).
#[derive(Resource)]
pub struct Walkable(pub Vec<Rect>);

impl Walkable {
    pub fn allows(&self, p: Vec2, r: f32) -> bool {
        [(-r, -r), (r, -r), (-r, r), (r, r)]
            .iter()
            .all(|(dx, dz)| {
                let q = p + Vec2::new(*dx, *dz);
                self.0.iter().any(|rc| rc.contains(q))
            })
    }
}

/// Emplacements où les apps flottent (poses de départ).
#[derive(Resource)]
pub struct Slots(pub Vec<Transform>);

#[derive(Component)]
struct Flicker {
    base: f32,
    seed: f32,
}

#[derive(Component)]
struct ShadowLight;

struct Ctx<'a, 'w, 's> {
    c: &'a mut Commands<'w, 's>,
    meshes: &'a mut Assets<Mesh>,
    mats: &'a mut Assets<StandardMaterial>,
    assets: &'a AssetServer,
}

fn linear(path: String, assets: &AssetServer) -> Handle<Image> {
    assets.load_builder().with_settings(|s: &mut ImageLoaderSettings| s.is_srgb = false).load(path)
}

impl Ctx<'_, '_, '_> {
    fn pbr(&mut self, name: &str, tile: Vec2, tint: Color) -> Handle<StandardMaterial> {
        let m = StandardMaterial {
            base_color: tint,
            base_color_texture: Some(self.assets.load(format!("textures/{name}/albedo.jpg"))),
            normal_map_texture: Some(linear(format!("textures/{name}/normal.jpg"), self.assets)),
            metallic_roughness_texture: Some(linear(format!("textures/{name}/rough.jpg"), self.assets)),
            occlusion_texture: Some(linear(format!("textures/{name}/ao.jpg"), self.assets)),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            uv_transform: Affine2::from_scale(tile),
            ..default()
        };
        self.mats.add(m)
    }

    /// Boîte texturisée centrée en `center`.
    fn block(&mut self, tex: &str, tint: Color, center: Vec3, size: Vec3, tile: Vec2) {
        let mat = self.pbr(tex, tile, tint);
        let mesh = self.meshes.add(Cuboid::new(size.x, size.y, size.z));
        self.c.spawn((Mesh3d(mesh), MeshMaterial3d(mat), Transform::from_translation(center)));
    }

    /// Mur le long de X (z fixe) entre x0 et x1.
    fn wall_x(&mut self, tex: &str, z: f32, x0: f32, x1: f32) {
        let len = x1 - x0;
        self.block(
            tex,
            Color::srgb(0.75, 0.75, 0.78),
            Vec3::new((x0 + x1) / 2.0, HEIGHT / 2.0, z),
            Vec3::new(len, HEIGHT, WALL_T),
            Vec2::new(len / 2.5, HEIGHT / 2.5),
        );
    }

    /// Mur le long de Z (x fixe) entre z0 et z1.
    fn wall_z(&mut self, tex: &str, x: f32, z0: f32, z1: f32) {
        let len = z1 - z0;
        self.block(
            tex,
            Color::srgb(0.75, 0.75, 0.78),
            Vec3::new(x, HEIGHT / 2.0, (z0 + z1) / 2.0),
            Vec3::new(WALL_T, HEIGHT, len),
            Vec2::new(len / 2.5, HEIGHT / 2.5),
        );
    }

    fn floor_ceiling(&mut self, floor_tex: &str, x0: f32, x1: f32, z0: f32, z1: f32) {
        let (w, d) = (x1 - x0, z1 - z0);
        let c = Vec3::new((x0 + x1) / 2.0, 0.0, (z0 + z1) / 2.0);
        self.block(floor_tex, Color::srgb(0.7, 0.7, 0.72), c - Vec3::Y * 0.1, Vec3::new(w, 0.2, d), Vec2::new(w / 3.0, d / 3.0));
        self.block(
            "metal_plate",
            Color::srgb(0.5, 0.5, 0.55),
            c + Vec3::Y * (HEIGHT + 0.1),
            Vec3::new(w, 0.2, d),
            Vec2::new(w / 2.0, d / 2.0),
        );
    }

    fn prop(&mut self, name: &str, pos: Vec3, yaw_deg: f32, scale: f32) {
        let scene: Handle<WorldAsset> = self
            .assets
            .load(GltfAssetLabel::Scene(0).from_asset(format!("models/{name}/{name}_1k.gltf")));
        self.c.spawn((
            WorldAssetRoot(scene),
            Transform::from_translation(pos)
                .with_rotation(Quat::from_rotation_y(yaw_deg.to_radians()))
                .with_scale(Vec3::splat(scale)),
        ));
    }

    /// Lampe industrielle suspendue + lumière qui scintille.
    fn lamp(&mut self, x: f32, z: f32, color: Color, intensity: f32, shadows: bool, seed: f32) {
        self.prop("hanging_industrial_lamp", Vec3::new(x, HEIGHT, z), seed * 40.0, 1.0);
        let mut e = self.c.spawn((
            PointLight {
                color,
                intensity,
                range: 16.0,
                shadow_maps_enabled: shadows,
                ..default()
            },
            Transform::from_xyz(x, HEIGHT - 0.9, z),
            Flicker { base: intensity, seed },
        ));
        if shadows {
            e.insert(ShadowLight);
        }
    }
}

fn build_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
    settings: Res<Settings>,
) {
    let mut cx = Ctx { c: &mut commands, meshes: &mut meshes, mats: &mut mats, assets: &assets };
    let wall = "concrete_wall_007";
    let (hub, cor, atl) = ((-8.0, 8.0, -8.0, 8.0), (8.0, 20.0, -2.0, 2.0), (20.0, 32.0, -8.0, 8.0));

    // Sols + plafonds
    cx.floor_ceiling("concrete_floor_worn_001", hub.0, hub.1, hub.2, hub.3);
    cx.floor_ceiling("rusty_metal_02", cor.0, cor.1, cor.2, cor.3);
    cx.floor_ceiling("concrete_floor_worn_001", atl.0, atl.1, atl.2, atl.3);

    // Hall central (porte à l'est)
    cx.wall_x(wall, -8.0, -8.0, 8.0);
    cx.wall_x(wall, 8.0, -8.0, 8.0);
    cx.wall_z(wall, -8.0, -8.0, 8.0);
    cx.wall_z(wall, 8.0, -8.0, -2.0);
    cx.wall_z(wall, 8.0, 2.0, 8.0);
    // Couloir
    cx.wall_x("corrugated_iron_02", -2.0, 8.0, 20.0);
    cx.wall_x("corrugated_iron_02", 2.0, 8.0, 20.0);
    // Atelier (porte à l'ouest)
    cx.wall_x(wall, -8.0, 20.0, 32.0);
    cx.wall_x(wall, 8.0, 20.0, 32.0);
    cx.wall_z(wall, 32.0, -8.0, 8.0);
    cx.wall_z(wall, 20.0, -8.0, -2.0);
    cx.wall_z(wall, 20.0, 2.0, 8.0);

    // Lumières (couleurs chaudes/froides, quelques ombres seulement)
    let sh = settings.shadows;
    cx.lamp(-3.5, -3.5, Color::srgb(1.0, 0.78, 0.5), 140_000.0, sh, 0.3);
    cx.lamp(3.5, 3.5, Color::srgb(1.0, 0.8, 0.55), 140_000.0, sh, 1.7);
    cx.lamp(-3.5, 3.5, Color::srgb(0.75, 0.85, 1.0), 90_000.0, false, 2.9);
    cx.lamp(3.5, -3.5, Color::srgb(1.0, 0.7, 0.45), 110_000.0, false, 4.1);
    cx.lamp(14.0, 0.0, Color::srgb(1.0, 0.45, 0.25), 70_000.0, false, 5.3);
    cx.lamp(26.0, -3.5, Color::srgb(0.7, 0.85, 1.0), 120_000.0, sh, 6.2);
    cx.lamp(26.0, 3.5, Color::srgb(1.0, 0.8, 0.55), 120_000.0, false, 7.7);

    // Props — hall
    cx.prop("steel_frame_shelves_01", Vec3::new(-7.5, 0.0, -2.0), 90.0, 1.0);
    cx.prop("steel_frame_shelves_01", Vec3::new(-7.5, 0.0, 2.2), 90.0, 1.0);
    cx.prop("wooden_crate_01", Vec3::new(-6.5, 0.0, 6.5), 20.0, 1.0);
    cx.prop("plastic_crate_01", Vec3::new(-5.6, 0.0, 6.9), 75.0, 1.0);
    cx.prop("barrel_03", Vec3::new(6.8, 0.0, -6.8), 0.0, 1.0);
    cx.prop("barrel_03", Vec3::new(6.0, 0.0, -7.2), 40.0, 1.0);
    cx.prop("metal_trash_can", Vec3::new(7.2, 0.0, 5.8), 0.0, 1.0);
    cx.prop("power_box_01", Vec3::new(0.0, 1.4, -7.75), 0.0, 1.0);
    cx.prop("metal_office_desk", Vec3::new(0.0, 0.0, 6.6), 180.0, 1.0);
    // Props — atelier
    cx.prop("portable_generator", Vec3::new(30.5, 0.0, -6.5), 200.0, 1.0);
    cx.prop("old_military_crate", Vec3::new(30.0, 0.0, 5.5), 15.0, 1.0);
    cx.prop("old_military_crate", Vec3::new(30.1, 0.0, 6.9), 80.0, 1.0);
    cx.prop("metal_toolbox", Vec3::new(24.0, 0.0, 6.9), 30.0, 1.0);
    cx.prop("steel_frame_shelves_01", Vec3::new(26.0, 0.0, -7.5), 0.0, 1.0);
    cx.prop("steel_frame_shelves_01", Vec3::new(29.0, 0.0, -7.5), 0.0, 1.0);

    commands.insert_resource(Walkable(vec![
        Rect::new(hub.0 + 0.1, hub.2 + 0.1, hub.1, hub.3 - 0.1),
        Rect::new(cor.0, cor.2, cor.1, cor.3),
        Rect::new(atl.0, atl.2 + 0.1, atl.1 - 0.1, atl.3 - 0.1),
    ]));

    // Emplacements d'apps : arc dans le hall, puis dans l'atelier.
    let mut slots = Vec::new();
    for k in 0..5 {
        let a = PI * (0.38 + 0.31 * k as f32); // évite la porte à l'est (angle 0)
        let p = Vec3::new(5.6 * a.cos(), 1.75, 5.6 * a.sin());
        slots.push(facing(p, Vec3::new(0.0, 1.75, 0.0)));
    }
    for k in 0..4 {
        let z = -5.5 + 3.6 * k as f32;
        slots.push(facing(Vec3::new(26.0 + 4.0 * ((k % 2) as f32 - 0.5), 1.75, z), Vec3::new(23.0, 1.75, z)));
    }
    commands.insert_resource(Slots(slots));
    let _ = TAU;
}

/// Pose en `p` avec la face avant (+Z) tournée vers `toward`.
pub fn facing(p: Vec3, toward: Vec3) -> Transform {
    Transform::from_translation(p).looking_at(2.0 * p - toward, Vec3::Y)
}

fn flicker(time: Res<Time>, mut q: Query<(&mut PointLight, &Flicker)>) {
    let t = time.elapsed_secs();
    for (mut l, f) in &mut q {
        let slow = 0.92 + 0.08 * (t * 1.3 + f.seed).sin();
        // de temps en temps, une coupure franche façon tube néon mourant
        let n = (t * 7.0 + f.seed * 13.0).sin() * (t * 3.1 + f.seed).sin();
        let dip = if n > 0.93 { 0.25 } else { 1.0 };
        l.intensity = f.base * slow * dip;
    }
}

fn apply_shadow_setting(settings: Res<Settings>, mut q: Query<&mut PointLight, With<ShadowLight>>) {
    if settings.is_changed() {
        for mut l in &mut q {
            l.shadow_maps_enabled = settings.shadows;
        }
    }
}