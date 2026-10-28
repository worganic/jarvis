# Compétences de Jarvis

Chaque fichier `.js` de ce dossier devient une commande que Jarvis comprend **sans IA**.
Il est pris en compte en une seconde (pas besoin de relancer Jarvis) et apparaît dans l'aide
(« aide », « quelle commande pour… »). Un fichier dont le nom commence par `_` est ignoré.

```js
module.exports = {
  name: 'Pile ou face',
  // Phrases montrées dans l'aide aux commandes.
  examples: ['pile ou face', 'lance une pièce'],
  // Texte « replié » : minuscules, sans accents ni ponctuation (« lance une piece »).
  // Une fonction (t, raw) => résultat est aussi acceptée.
  match: /^(?:pile ou face|lance une piece)$/,
  // Ce que Jarvis dit (texte), ou { say, display }.
  async run({ text, raw, match, tool, System, orch }) {
    return Math.random() < 0.5 ? 'Pile !' : 'Face !';
  }
};
```

`run` reçoit :

| Nom | Contenu |
|---|---|
| `text` | la phrase repliée |
| `raw` | la phrase telle qu'elle a été dite ou tapée |
| `match` | le résultat de `match` (groupes de l'expression régulière) |
| `tool(nom, params, exec)` | exécute une action **sous la politique de sécurité de Jarvis** (journal, code à 4 chiffres si l'action l'exige) |
| `System` | les actions Windows de Jarvis (`orchestrator/system.js`) : ouvrir une appli, une URL, le volume… |
| `orch` | l'orchestrateur (à utiliser avec prudence) |

Toute action qui modifie quelque chose doit passer par `tool` avec le nom de l'outil correspondant
(par exemple `tool('mcp__system__ouvrir_url', { url }, (p) => System.ouvrir_url(p))`) : c'est ce qui
applique la sécurité réglée dans l'administration.
