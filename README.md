# Jarvis v4

Assistant vocal personnel pour Windows, réécrit entièrement en Rust à partir de Jarvis v3
(Node + Python + C#). Mêmes options, mêmes fonctionnalités, même interface (`ui/`, inchangée).

## Installer Jarvis (Windows 10 ou 11)

Dans PowerShell (pas besoin des droits administrateur) :

```
powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/worganic/jarvis/main/installer.ps1 | iex"
```

Le script installe ce qu'il faut pour compiler (Git, Rust, Visual Studio Build Tools, WebView2, via winget),
récupère la dernière version publiée, te laisse choisir les modules, compile Jarvis sur ton PC (10 à 20 minutes la
première fois), l'installe dans `%LOCALAPPDATA%\Jarvis\app`, supprime les sources et télécharge les modèles vocaux
(environ 3,6 Go). Il propose enfin de restaurer tes données depuis une archive `.jarvis`. Ensuite Jarvis vérifie
seul les nouvelles versions (Administration › Mise à jour) ; tes données restent dans `%LOCALAPPDATA%\Jarvis\donnees`.
Désinstaller : `powershell -ExecutionPolicy Bypass -File "%LOCALAPPDATA%\Jarvis\app\installer.ps1" -Desinstaller`.

## Composants

| Crate | Exécutable | Rôle |
|---|---|---|
| `crates/jarvis-shell` | `jarvis.exe` | Coquille Tauri : HUD, widget, administration, Bureau JARVIS, zone de notification, raccourcis |
| `crates/jarvis-core` | `jarvis-core.exe` | Noyau : orchestrateur, sécurité, mémoire, journal scellé, connecteurs, passerelle des fenêtres, serveurs MCP |
| `crates/jarvis-voice` | `jarvis-voice.exe` | Moteur vocal : micro, anti-écho WebRTC, VAD, mot d'éveil, Whisper, empreinte vocale, synthèse |
| `crates/jarvis-launcher` | `Jarvis v4.exe` | Lanceur à la racine du projet : démarre la coquille, vérifie que la v3 n'occupe pas les ports |
| `crates/jarvis-native` | (bibliothèque) | API Windows : fenêtres, UI Automation, audio, médias, OCR, capture d'écran, saisie |

La coquille lance le noyau, qui lance le moteur vocal. Ports locaux : 8797 (fenêtres), 8798
(moteur vocal), 8799 (outils MCP). Un seul Jarvis à la fois (v3 ou v4).

## Publier une version (développeur)

`pwsh tools/publier.ps1` publie la version de `Cargo.toml` sur le dépôt public de diffusion (contrôles, export sans
historique ni fichiers de travail, recherche de secrets, catalogue des modules, tag) ; `-Essai` contrôle sans rien
envoyer, `-Beta` publie sur le canal bêta, `-Modeles` empaquette les modèles vocaux en pièces jointes d'une version
GitHub et écrit `modeles.json`.

## Compiler

Prérequis : Rust stable (MSVC) et Visual Studio Build Tools. Aucune autre installation : les
binaires WebRTC, sherpa-onnx et onnxruntime DirectML sont téléchargés à la première compilation.

```
cargo build --release
```

Les exécutables et leurs DLL sont dans `target/release/`.

## Lancer

Double-cliquer sur **`Jarvis v4.exe`** à la racine du projet (ou en faire un raccourci sur le
Bureau). Ce lanceur démarre `target/release/jarvis.exe` ; si Jarvis v4 tourne déjà, le HUD revient
au premier plan ; si la v3 est lancée, il demande de la fermer d'abord. Après une modification du
lanceur (`crates/jarvis-launcher`), le recopier :
`copy target\release\jarvis-launcher.exe "Jarvis v4.exe"`.

## Modèles

Dans le dossier de données (`%LOCALAPPDATA%\Jarvis\donnees\models\`, anciennement `data/models/`) :
- `whisper-large-v3-turbo-onnx/` : Whisper (export ONNX `onnx-community/whisper-large-v3-turbo`,
  encodeur et décodeur fp16 pour le GPU, décodeur pas-à-pas float32) ;
- `vad/silero_vad_v6.onnx`, `kws/` (mot d'éveil), `speaker/` (empreinte CAM++), `tts/` (Piper).

## Reprendre les données de la v3

```
target\release\jarvis-core.exe migrate --from "E:\www\A-En cours\jarvis-v3"
```

La v3 n'est que lue. Mémoire, réglages, listes, empreintes vocales, corpus, notes et journal scellé
sont repris (le journal continue sa chaîne). Le rapport `data/migration-v3.json` est inscrit au
journal au premier démarrage.

## Outils en ligne de commande

- `jarvis-core relaunch --apercu` : aperçu de l'écran de mise à jour ;
- `jarvis-voice stt <wav…>` : transcription de fichiers ; `jarvis-voice wake <wav…>` : VAD et mot
  d'éveil ; `jarvis-voice say "<texte>" <wav>` : synthèse edge-tts ;
- `jarvis-voice bench stt|speaker --data data` : bancs d'essai (aussi lançables depuis
  l'administration).

Le détail du portage, module par module, est dans `PORTAGE.md`.

## Historique des versions

`ui/changelog.json` est la source unique : versions classées **du plus récent au plus ancien**, chaque
modification porte un thème (interface, coeur, son, noyau, sécurité, mémoire, connecteurs, système,
mise à jour, outils — liste dans le fichier). La première entrée est la version courante : elle
s'affiche dans la fenêtre de mise à jour et en bas du HUD ; un clic sur ce numéro ouvre
`ui/changelog.html` (filtres par thème, recherche) dans le navigateur.

À chaque évolution : ajouter une entrée en tête de `ui/changelog.json`, et aligner `version` dans
`Cargo.toml` (`[workspace.package]`) et `crates/jarvis-shell/tauri.conf.json`.

## Page d'aide

Le bouton **? AIDE** du HUD (ou **F1**) ouvre `ui/aide.html` dans le navigateur : fonctions, fonctionnement,
raccourcis, administration, sécurité et toutes les informations des RAG. Le texte rédigé est dans
`ui/aide-contenu.json` ; les commandes, les capacités, les RAG et l'état sont lus en direct dans le noyau
(`/aide/donnees.json`, voir `crates/jarvis-core/src/orch/help.rs`). **À chaque nouvelle fonction, mettre à jour
`ui/aide-contenu.json`** et reporter la version dans `aJourVersion` : un test échoue sinon (consignes complètes
dans `CLAUDE.md`).
