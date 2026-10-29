//! Réglages persistants (~/.config/wayworld/settings.json).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Resource, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// radians par pixel de souris
    pub sensitivity: f32,
    /// champ de vision vertical en degrés
    pub fov: f32,
    /// distance de visibilité du brouillard (m)
    pub fog_visibility: f32,
    pub move_speed: f32,
    pub fullscreen: bool,
    pub shadows: bool,
    /// index dans APP_PRESETS : app lancée en appuyant sur Jouer
    pub startup_app: usize,
    /// mode VR (OpenXR) ; aussi activable avec WAYWORLD_VR=1 (binaire compilé avec --features vr)
    pub vr: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sensitivity: 0.0022,
            fov: 80.0,
            fog_visibility: 28.0,
            move_speed: 3.2,
            fullscreen: false,
            shadows: true,
            startup_app: 0,
            vr: false,
        }
    }
}

/// (libellé, commande) — "" = ne rien lancer.
pub const APP_PRESETS: &[(&str, &str)] = &[
    ("Terminal (foot)", "foot"),
    ("Terminal (weston)", "weston-terminal"),
    ("Firefox", "firefox"),
    ("Fichiers (nautilus)", "nautilus"),
    ("Aucune", ""),
];

impl Settings {
    fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("wayworld").join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }

    pub fn startup_command(&self) -> &'static str {
        APP_PRESETS.get(self.startup_app).map(|p| p.1).unwrap_or("")
    }
}