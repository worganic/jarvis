# Modules de Jarvis

Jarvis est le système principal (coquille, noyau, moteur vocal, HUD commun). Chaque card est un **module
indépendant**, rangé ici dans son propre dossier : on peut l'ajouter, l'enlever ou en créer un sans toucher au reste.

```
modules/
├── _modele/          module exemple à copier (interface seule) — ignoré au chargement (dossier en « _ »)
├── actions/ audio/ bureau/ checkup/ conversation/ devant/ listes/ mails/ materiel/ musique/
│   navigateur/ nomade/ notes/ diction/ projets/ reunions/ savoir/ terminal/ achats/ agenda/   les cards de Jarvis
└── <id>/
    ├── module.json   la fiche (obligatoire)
    ├── ui/           l'interface de la card : scripts et styles (lus sur le disque, sans recompiler)
    └── core/         la partie noyau, facultative : une crate Rust (recompilation nécessaire)
```

## La fiche `module.json`

```json
{
  "id": "audio",
  "nom": "Audio",
  "version": "1.0.0",
  "description": "Ce que fait le module, en une phrase.",
  "card": { "type": 1, "numero": 2, "titre": "AUDIO" },
  "ui": { "scripts": ["ui/audio.js"], "styles": ["ui/audio.css"] },
  "core": "jarvis-mod-audio",
  "actions": [{ "ico": "▤", "titre": "Ouvrir", "sous": "dans la card Actions", "dire": "phrase envoyée à Jarvis" }],
  "parDefaut": true
}
```

| Champ | Rôle |
|---|---|
| `id` | identifiant = **nom du dossier** (minuscules, chiffres, tirets) |
| `nom`, `version`, `description` | affichés dans Administration › Modules et dans la page d'aide |
| `card` | type 1 (toujours active) : numéro 1 à 99 ; type 2 (temporaire) : 100 à 199. Numéro unique entre modules. `cle` (facultatif) : clé de la card dans `J.CARD_NO` si elle diffère de l'id |
| `ui.scripts`, `ui.styles`, `ui.es` | fichiers de `ui/`, chargés dans l'ordre par `ui/modules-loader.js`, après les scripts du système ; `es` : modules JavaScript (`import`), chargés avant les scripts du module (chemins absolus vers le système : `/vendor/…`) |
| `core` | nom de la crate de `core/` (absent : module sans partie noyau) |
| `card.emplacement` | card ouverte par le noyau : son emplacement de numéro (`note`, `tool:Terminal`…, voir `orch/cards.rs › slot_of`) ; le numéro de la fiche devient celui par défaut |
| `widgets` | kinds des événements « widget » que le module affiche (`note`…) : ignorés quand il est désactivé |
| `actions` | lignes ajoutées à la card Actions (facultatif) |
| `parDefaut` | actif tant qu'on ne l'a pas désactivé dans l'administration (vrai par défaut) |
| `socle` | indispensable : toujours installé (installer.ps1 ne propose pas de le retirer) ; faux par défaut |
| `dependances` | ids des modules dont celui-ci a besoin (installés avec lui, et impossibles à retirer tant qu'il est là) |

La fiche est vérifiée au démarrage ; une fiche refusée est signalée dans Administration › Modules.

## Modules créés par l'utilisateur

La card **Create module** (N° 121) permet de créer ses propres modules avec l'IA, sans toucher au dépôt : ils sont
rangés dans `data/user-modules/perso-<nom>/` (même structure : `module.json`, `ui/`), chargés comme les autres et
numérotés de 130 à 198. Le noyau (`module_studio.rs`) écrit la fiche et vérifie le code avant de l'enregistrer.
Un module supprimé va dans `data/user-modules/.corbeille/`. L'id commence toujours par `perso-`.

## Interface (`ui/`)

Les scripts du module s'exécutent dans le HUD et utilisent l'API commune :

- `J.cardHeader({ key, no, title, onCollapse })` : en-tête standard (poignée, numéro, titre, 📍 📌 –) ;
- `J.CARD_NO[id]` : numéro de la card (celui de la fiche, ou celui choisi dans l'administration) ;
- `J.modules[id]` : la fiche du module telle que servie par le noyau ;
- `window.jarvisBridge.call('méthode', …)` : appel d'une méthode du noyau ou de la partie noyau du module ;
- `window.jarvisBridge.onTaskEvent(cb)` : événements diffusés par le noyau ;
- `J.widgets` : cards communes (déplacer, réduire…) ;
- `J.widgetKit` : cadre commun des cards de `ui/widgets.js` — `h` (création d'éléments), `place(clé, el, kind, durée)`,
  `close`, `gripHandle`, `numBadge`, `closeButton`, `renderRows` (card de liste en lignes), `register(kind, rendu)`
  (affiche les événements « widget » de ce kind), `on(type, fn)` (autres événements), `onClose(clé, fn)`, `vanish(clé)`
  (la card disparaît à la fermeture au lieu de rester en fantôme).

Préfixer les sélecteurs CSS par l'id du module (`#mod-<id>`, `.au-…`) pour ne rien toucher d'autre. La classe
`mod-<id>` sur `<html>` indique qu'un module est actif (une autre card peut s'en servir).

Une modification de `ui/` ou de `module.json` recharge les fenêtres, sans recompiler.

## Partie noyau (`core/`)

Une crate qui dépend de `jarvis-core` et implémente `jarvis_core::Module` :

```rust
impl Module for MonModule {
    fn id(&self) -> &'static str { "mon-module" }
    fn start(&self, ctx: Rc<ModuleCtx>) {
        ctx.register("monModuleEtat", &["hud"], |args, _client| Box::pin(async move { Ok(json!({ "ok": true })) }));
    }
    fn stop(&self) {}
}
```

`ModuleCtx` donne : `register` (méthode appelable par les fenêtres ; un nom déjà pris est refusé), `broadcast`
(événement vers les fenêtres), `log`, `data_dir()` (`data/modules/<id>/`) et `core()` (services partagés du cœur :
`emit`, `record` pour le journal scellé, `jarvis_volume`, `audio_device_changed`). `subcommand` permet un processus
auxiliaire (`jarvis-core <commande>`), comme le mélangeur du module Audio.

Pour la lier à Jarvis : ajouter la crate dans `crates/jarvis-modules/Cargo.toml` et dans `crates/jarvis-modules/src/lib.rs › all()`.
Le test `cargo test -p jarvis-modules` vérifie que chaque module qui déclare `core` est bien lié, et inversement.

## Ce qui reste au système

Le cadre commun des cards (ui/widgets.js : placement, déplacement, réduction en bouton, 📌 📍, fantômes, card de liste
en lignes et son mode compact, card-header.js), les boutons du HUD (cards-dock.js : RANGER, PLACER, FIGER…), le canevas
du HUD (app.js : cœur, card Conversation) et la logique du noyau partagée entre plusieurs cards ou avec la voix (orch/) :
mails, agenda, notes, réunions, listes, terminaux, bureau… Un module en appelle les méthodes ; seul le module Audio a
aujourd'hui sa propre partie noyau. Points d'accroche du cadre pour un module : `register`, `registerTool`, `on`,
`onClose`, `vanish`, `guardClose`, `onIdle`, `ownKey`, `headButtons`, `footButtons`, `noAppClick`, `ghostKey`,
`addGhost`, `dispatch` (voir ui/widgets.js › J.widgetKit).

## Créer un module

1. Copier `modules/_modele` en `modules/<id>` ;
2. dans `module.json` : `id` = `<id>`, `nom`, `description`, un numéro de card libre ;
3. renommer et écrire `ui/<id>.js` et `ui/<id>.css` (et les déclarer dans la fiche) ;
4. s'il faut une logique Rust : ajouter `core/` (voir le module Audio) et la lier dans `crates/jarvis-modules` ;
5. vérifier : `cargo test -p jarvis-core -p jarvis-modules`.

## Enlever un module

Le désactiver dans Administration › Modules, ou supprimer son dossier. S'il a une partie noyau, retirer aussi ses
deux lignes de `crates/jarvis-modules`, puis recompiler.
