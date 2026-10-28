// Environnement « studio » partagé par les rendus three.js du HUD (personnage du cœur, explorateur 3D des
// dossiers Data) : reflets des matières, calculés une fois par scène.
import * as THREE from './vendor/three.module.min.js';

export function studioEnvironment(renderer) {
  const scene = new THREE.Scene();
  const room = new THREE.Mesh(new THREE.SphereGeometry(10, 32, 16), new THREE.MeshBasicMaterial({ color: 0x0b1626, side: THREE.BackSide }));
  scene.add(room);
  const box = new THREE.PlaneGeometry(1, 1);
  const panel = (color, scale, x, y, z, w, h) => {
    const m = new THREE.Mesh(box, new THREE.MeshBasicMaterial({ color: new THREE.Color(color).multiplyScalar(scale), side: THREE.DoubleSide }));
    m.position.set(x, y, z);
    m.scale.set(w, h, 1);
    m.lookAt(0, 0, 0);
    scene.add(m);
  };
  panel(0xffffff, 6, 0, 6, 3, 6, 3);       // boîte à lumière au-dessus, devant
  panel(0xdfeeff, 3, 6, 2, 4, 3, 5);       // côté
  panel(0x7fd8ff, 2.5, -6, 1, -3, 3, 5);   // contre-jour froid
  panel(0x203040, 1, 0, -6, 0, 12, 12);    // sol sombre
  const pmrem = new THREE.PMREMGenerator(renderer);
  const env = pmrem.fromScene(scene, 0.04).texture;
  pmrem.dispose();
  scene.traverse((o) => {
    if (o.geometry) o.geometry.dispose();
    if (o.material) o.material.dispose();
  });
  return env;
}
