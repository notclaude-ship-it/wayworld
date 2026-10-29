# Wayworld

Un environnement de bureau **Wayland** qui est aussi un **jeu 3D** : tes applications Linux flottent dans un
entrepôt que tu parcours à la première personne. Tu peux les saisir, les utiliser, les épingler à ton bras,
et, avec la compilation `vr`, t'y déplacer en réalité virtuelle (SteamVR + Meta Quest 2).

Ecrit en Rust avec [Bevy](https://bevyengine.org) 0.19 ; le compositeur Wayland intégré utilise
[smithay](https://github.com/Smithay/smithay). Les apps se connectent au socket `wayworld-N` affiché au démarrage.

## Fonctions

- Compositeur Wayland intégré : n'importe quelle app Wayland (terminal `foot`, Firefox, Chromium, Nautilus…).
- Apps flottantes dans le monde 3D, ou **épinglées au bras** (lever le bras pour les voir).
- Contrôles **tactiles** complets (joystick, regard, clavier à l'écran, zoom, clic droit, molette).
- **Appui long sur « Nouvelle »** : liste défilante de toutes les applications installées (lanceurs `.desktop`).
- Zoom de l'app tenue (boutons `Zoom +/-`).
- Mode **VR** OpenXR (voir plus bas).
- Peut servir de session de bureau complète via [cage](https://github.com/cage-kiosk/cage).

## Télécharger

Des binaires prêts à l'emploi (avec les assets) sont dans les
[Releases](../../releases) : `x86_64` (avec le support VR) et `arm64` (sans VR).

```sh
tar xzf wayworld-<version>-linux-x86_64.tar.gz
cd wayworld-<version>-linux-x86_64
./wayworld            # fenêtre ; ou ./install.sh pour une installation système
```

Il faut un GPU/pilote Vulkan, `libxkbcommon`, `libudev` et `libasound`.

## Compiler

```sh
python3 scripts/fetch_assets.py 1k     # télécharge les assets CC0 (Poly Haven) dans assets/
cargo run --release                    # écran plat
cargo run --release --features vr      # avec le support VR
scripts/build-arm64.sh                 # cross-compilation arm64 depuis x86_64
```

`./install.sh` compile puis installe le binaire, les assets et la session « Wayworld » (nécessite `cage`).

## Commandes

Clavier / souris : `W A S D` (positions physiques, donc `Z Q S D` en AZERTY) marcher, `Maj` courir, `E` utiliser
l'app visée, `F` épingler/détacher, `X` fermer, `T` nouvelle app, `Q` lever le bras, `Echap` menu / quitter l'app.

Tactile : joystick à gauche, regard à droite, boutons à l'écran. Les contrôles tactiles s'activent au premier
toucher, ou en forçant `WAYWORLD_TOUCH=1`. Sous X11, le curseur n'est pas capturé tant que le tactile est actif.

### Variables d'environnement

| Variable | Effet |
| --- | --- |
| `WAYWORLD_ASSETS` | dossier des assets |
| `WAYWORLD_TOUCH=1/0` | force / désactive les contrôles tactiles |
| `WAYWORLD_KB=azerty/qwerty` | disposition du clavier à l'écran |
| `WAYWORLD_VR=1` | active la VR (binaire compilé avec `--features vr`) |
| `WAYWORLD_VR_MIRROR=1` | garde aussi le rendu de la fenêtre du PC en VR |
| `WAYWORLD_SHOT_DIR` | outil de dev : prend des captures puis quitte |

Les réglages sont dans `~/.config/wayworld/settings.json`.

## VR (SteamVR + Quest 2)

Le mode VR passe par **OpenXR** ([bevy_mod_openxr](https://github.com/awtterpip/bevy_oxr)). Wayworld est un
compositeur Wayland, donc il tourne sur **Linux** : il faut SteamVR pour Linux avec le Quest 2 connecté par
[ALVR](https://github.com/alvr-org/ALVR) (ou tout runtime OpenXR de ton casque), SteamVR étant le runtime OpenXR actif.

```sh
WAYWORLD_VR=1 ./wayworld
```

Le jeu démarre tout seul (pas de menu à cliquer). Sans runtime OpenXR, Wayworld reste en écran plat.

| Contexte | Commande |
| --- | --- |
| Explorer | stick gauche : marcher (dans la direction du regard) · stick droit : rotation par crans de 30° · gâchette gauche : courir |
| Explorer | laser de la main droite : viser · gâchette droite : utiliser l'app visée · grip droit : épingler/détacher · A : nouvelle app · X : fermer l'app visée |
| Utiliser une app | laser : curseur · gâchette droite : clic gauche · grip droit : clic droit · stick droit : molette · stick gauche haut/bas : zoom · B ou Y : quitter l'app |

Profils de manettes : Oculus Touch, Valve Index et manette OpenXR simple.

**Etat : expérimental.** Le code compile mais n'a pas encore été testé avec un casque. Limites connues : pas de
clavier virtuel en VR (le clavier physique du PC fonctionne), pas de menu pause lisible dans le casque, et les
noms d'apps avec accents peuvent s'afficher avec des carrés (police par défaut de Bevy).

## Structure

```
src/        apps, compositeur (comp), joueur, menu, tactile, monde, VR (vr.rs, feature `vr`)
scripts/    fetch_assets.py, build-arm64.sh (+ sysroot), package-release.sh
session/    entrée de session de bureau (cage)
assets/     CREDITS.md ; textures et modèles téléchargés par fetch_assets.py
```

## Licence

Code sous licence MIT (voir `LICENSE`). Les assets viennent de [Poly Haven](https://polyhaven.com), licence CC0
(voir `assets/CREDITS.md`).