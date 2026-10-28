# Bibliothèques tierces du HUD

| Fichier | Origine | Licence |
|---|---|---|
| `three.module.min.js` | three.js r170 (`build/three.module.min.js`) — personnage du cœur, explorateur 3D | MIT (`three-LICENSE.txt`) |
| `GLTFLoader.js` | three.js r170 (`examples/jsm/loaders/GLTFLoader.js`) — modèles `.glb` des dossiers Data | MIT |
| `BufferGeometryUtils.js` | three.js r170 (`examples/jsm/utils/BufferGeometryUtils.js`) — utilisé par `GLTFLoader.js` | MIT |
| `OrbitControls.js` | three.js r170 (`examples/jsm/controls/OrbitControls.js`) — rotation autour du produit (explorateur 3D) | MIT |

Les trois derniers sont copiés tels quels, sauf leur `import … from 'three'` remplacé par `./three.module.min.js`
(pas de carte d'import dans le HUD).
