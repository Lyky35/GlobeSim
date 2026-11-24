import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { TideOcean } from './tide.js';
import {
  SIDEREAL_DAY_S,
  SIDEREAL_MONTH_S,
  AXIAL_TILT_DEG,
  MOON_ORBIT_INCL_DEG,
  MOON_DISTANCE_KM,
  EQUILIBRIUM_TIDE_M,
  EARTH_SCENE_RADIUS,
  MOON_SCENE_DISTANCE,
  MOON_SCENE_RADIUS,
  KM_PER_UNIT,
} from './constants.js';

const DEG = Math.PI / 180;

// ---------------------------------------------------------------------------
// Renderer / scene / camera
// ---------------------------------------------------------------------------
const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
renderer.setSize(window.innerWidth, window.innerHeight);
document.body.appendChild(renderer.domElement);

const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(
  45, window.innerWidth / window.innerHeight, 0.05, 500
);
camera.position.set(0, 1.4, 3.4);

const controls = new OrbitControls(camera, renderer.domElement);
controls.enableDamping = true;
controls.dampingFactor = 0.06;
controls.minDistance = 1.2;
controls.maxDistance = 60;

const texLoader = new THREE.TextureLoader();
function loadTex(url) {
  const t = texLoader.load(url);
  t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = renderer.capabilities.getMaxAnisotropy();
  return t;
}

// Starfield background (equirectangular).
scene.background = loadTex('../assets/textures/stars_milky_way_2k.jpg');

// Sun light + ambient fill.
const sun = new THREE.DirectionalLight(0xffffff, 2.4);
sun.position.set(25, 6, 10);
scene.add(sun);
scene.add(new THREE.AmbientLight(0x223344, 1.1));

// ---------------------------------------------------------------------------
// Earth: axial tilt group -> spinner (sidereal rotation) -> land sphere
// ---------------------------------------------------------------------------
const earthGroup = new THREE.Group();
earthGroup.rotation.z = AXIAL_TILT_DEG * DEG;
scene.add(earthGroup);

const spinner = new THREE.Group();
earthGroup.add(spinner);

const earthMesh = new THREE.Mesh(
  new THREE.SphereGeometry(EARTH_SCENE_RADIUS, 128, 96),
  new THREE.MeshPhongMaterial({
    map: loadTex('../assets/textures/earth_topography_3840.jpg'),
    specular: 0x111111,
    shininess: 6,
  })
);
spinner.add(earthMesh);

// Polar axis + equator guide (tilted with the axis, does not spin).
const axisHelper = new THREE.Group();
{
  const axisMat = new THREE.LineBasicMaterial({ color: 0xff5555, transparent: true, opacity: 0.8 });
  const axisGeo = new THREE.BufferGeometry().setFromPoints([
    new THREE.Vector3(0, -1.5, 0), new THREE.Vector3(0, 1.5, 0),
  ]);
  axisHelper.add(new THREE.Line(axisGeo, axisMat));

  const eqPts = [];
  for (let i = 0; i <= 128; i++) {
    const a = (i / 128) * Math.PI * 2;
    eqPts.push(new THREE.Vector3(Math.cos(a) * 1.001, 0, Math.sin(a) * 1.001));
  }
  axisHelper.add(
    new THREE.Line(
      new THREE.BufferGeometry().setFromPoints(eqPts),
      new THREE.LineBasicMaterial({ color: 0x66aaff, transparent: true, opacity: 0.5 })
    )
  );
}
axisHelper.visible = false; // matches the unchecked "Rotation axis" box
earthGroup.add(axisHelper);

// ---------------------------------------------------------------------------
// Ocean with tidal bulge (CPU or GPU displacement)
// ---------------------------------------------------------------------------
const ocean = new TideOcean(EARTH_SCENE_RADIUS * 1.0005);
scene.add(ocean.mesh);

// ---------------------------------------------------------------------------
// Moon: orbits in the ecliptic plane (inclined 5.15 deg), sidereal month.
// Drawn at 14 Earth radii instead of 60 so it stays on screen.
// ---------------------------------------------------------------------------
const moonOrbit = new THREE.Group();
moonOrbit.rotation.x = MOON_ORBIT_INCL_DEG * DEG;
scene.add(moonOrbit);

const moonMesh = new THREE.Mesh(
  new THREE.SphereGeometry(MOON_SCENE_RADIUS, 64, 48),
  new THREE.MeshPhongMaterial({ map: loadTex('../assets/textures/moon_2k.jpg') })
);
moonMesh.position.set(MOON_SCENE_DISTANCE, 0, 0);
moonOrbit.add(moonMesh);

// Gravity line: Moon -> Earth centre (dashed).
const gravityLine = new THREE.Line(
  new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(), new THREE.Vector3()]),
  new THREE.LineDashedMaterial({ color: 0xffcc44, dashSize: 0.25, gapSize: 0.15, transparent: true, opacity: 0.85 })
);
scene.add(gravityLine);

// ---------------------------------------------------------------------------
// Simulation state + UI
// ---------------------------------------------------------------------------
const state = {
  running: true,
  speed: 3600,          // sim seconds per real second (24 h in 24 s)
  tideKm: 250,          // visual tide amplitude, exaggerated
  mode: 'gpu',
  simTime: 0,           // elapsed simulated seconds
};

ocean.setAmplitude(state.tideKm / KM_PER_UNIT);

const $ = (id) => document.getElementById(id);
const fmtX = (n) =>
  n >= 1e6 ? `x${(n / 1e6).toFixed(2)}M`
  : n >= 1e3 ? `x${(n / 1e3).toFixed(2)}k`
  : `x${n.toFixed(0)}`;
const fmtH = (sec) => {
  if (sec < 60) return `${sec.toFixed(1)} s`;
  const h = sec / 3600;
  return h >= 24 ? `${(h / 24).toFixed(1)} d` : `${h.toFixed(1)} h`;
};

// Speed slider is logarithmic: value 0..6 -> speed 1..1e6.
const speedSlider = $('speed');
speedSlider.value = Math.log10(state.speed);
$('speed-val').textContent = fmtX(state.speed);
speedSlider.addEventListener('input', () => {
  state.speed = 10 ** parseFloat(speedSlider.value);
  $('speed-val').textContent = fmtX(state.speed);
  updateStatsStatic();
});
document.querySelectorAll('[data-speed]').forEach((btn) => {
  btn.addEventListener('click', () => {
    speedSlider.value = Math.log10(parseFloat(btn.dataset.speed));
    state.speed = parseFloat(btn.dataset.speed);
    $('speed-val').textContent = fmtX(state.speed);
    updateStatsStatic();
  });
});

const tideSlider = $('tide');
function refreshTideLabel() {
  const km = state.tideKm;
  ocean.setAmplitude(km / KM_PER_UNIT);
  const ex = km > 0 ? Math.round((km * 1000) / EQUILIBRIUM_TIDE_M) : 0;
  $('tide-val').textContent =
    km === 0 ? 'off' : `${km} km  (~x${ex.toLocaleString()} of the real ${EQUILIBRIUM_TIDE_M} m bulge)`;
}
tideSlider.addEventListener('input', () => {
  state.tideKm = parseInt(tideSlider.value, 10);
  refreshTideLabel();
});
refreshTideLabel();

document.querySelectorAll('input[name=mode]').forEach((radio) => {
  radio.addEventListener('change', () => {
    state.mode = radio.value;
    ocean.setMode(state.mode);
    updateStatsStatic();
  });
});

$('chk-moon').addEventListener('change', (e) => {
  moonMesh.visible = e.target.checked;
  gravityLine.visible = e.target.checked && $('chk-gravity').checked;
});
$('chk-gravity').addEventListener('change', (e) => {
  gravityLine.visible = e.target.checked && $('chk-moon').checked;
});
$('chk-axis').addEventListener('change', (e) => {
  axisHelper.visible = e.target.checked;
});

$('btn-pause').addEventListener('click', togglePause);
function togglePause() {
  state.running = !state.running;
  $('btn-pause').textContent = state.running ? 'Pause' : 'Resume';
}
window.addEventListener('keydown', (e) => {
  if (e.code === 'Space' && e.target === document.body) {
    e.preventDefault();
    togglePause();
  }
});

$('btn-reset').addEventListener('click', () => {
  controls.reset();
  camera.position.set(0, 1.4, 3.4);
});

// Static part of the stats panel (tide period etc).
function updateStatsStatic() {
  // Lunar day (Sun-to-Sun... Moon-to-Moon passage of the meridian):
  // 1 / (1/sidereal day - 1/sidereal month). Fixed points see two high
  // tides per lunar day, hence lunarDay / 2 between high tides.
  const lunarDay = SIDEREAL_DAY_S * SIDEREAL_MONTH_S / (SIDEREAL_MONTH_S - SIDEREAL_DAY_S);
  $('stat-period').textContent =
    `high tide every ~${fmtH((lunarDay / 2) / state.speed)} real time`;
}
updateStatsStatic();

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------
const moonWorldDir = new THREE.Vector3();
let last = performance.now();
let fps = 60, fpsAccum = 0, fpsFrames = 0, statClock = 0;

function frame(now) {
  const dt = Math.min((now - last) / 1000, 0.1); // real seconds, capped
  last = now;
  if (state.running) state.simTime += dt * state.speed;

  // Earth: one turn per sidereal day of sim time.
  spinner.rotation.y = (2 * Math.PI * state.simTime) / SIDEREAL_DAY_S;

  // Moon: one orbit per sidereal month of sim time (offset so it starts
  // in the camera's field of view).
  moonOrbit.rotation.y =
    (40 * DEG) + (2 * Math.PI * state.simTime) / SIDEREAL_MONTH_S;
  moonMesh.lookAt(0, 0, 0); // tidally locked: same face to Earth

  // Tide-driving direction = direction of the Moon from Earth.
  moonMesh.getWorldPosition(moonWorldDir).normalize();
  ocean.setMoonDir(moonWorldDir);
  ocean.update();

  // Gravity line.
  if (gravityLine.visible) {
    const p = gravityLine.geometry.attributes.position;
    moonMesh.getWorldPosition(moonWorldDir);
    p.setXYZ(0, moonWorldDir.x, moonWorldDir.y, moonWorldDir.z);
    p.setXYZ(1, 0, 0, 0);
    p.needsUpdate = true;
    gravityLine.computeLineDistances();
  }

  controls.update();
  renderer.render(scene, camera);

  // Stats.
  fpsAccum += dt; fpsFrames++; statClock += dt;
  if (statClock > 0.5) {
    fps = fpsFrames / fpsAccum;
    fpsAccum = 0; fpsFrames = 0; statClock = 0;
    $('stat-fps').textContent = `${fps.toFixed(0)} fps`;
    $('stat-clock').textContent = `t = ${(state.simTime / 86400).toFixed(2)} sim days`;
    $('stat-cpu').textContent = state.mode === 'cpu'
      ? `tide pass: ${ocean.lastCpuMs.toFixed(2)} ms/frame on CPU (${ocean.geometry.attributes.position.count} verts)`
      : 'tide pass: vertex shader (GPU), ~0 ms on CPU';
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

window.addEventListener('resize', () => {
  camera.aspect = window.innerWidth / window.innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(window.innerWidth, window.innerHeight);
});
