import * as THREE from 'three';

// ---------------------------------------------------------------------------
// TideOcean: a translucent water sphere whose surface is displaced by the
// Moon's tide-generating (differential gravity) potential.
//
// Equilibrium tide height as a function of psi, the angular distance from
// the sub-lunar point:
//
//     h(psi) = A * (3 cos^2(psi) - 1) / 2
//
// This produces the classic two-bulge shape: +A toward the Moon, +A at the
// antipode, and -A/2 at 90 deg. Because the bulge is pinned to the Moon's
// direction while the ground rotates underneath it, any fixed point on the
// surface passes through two high tides per (lunar) day.
//
// The displacement can be computed in two ways (switchable at runtime):
//   * "gpu": a tiny patch injected into the vertex shader — every vertex is
//     displaced on the GPU, cost per frame is ~0 on the CPU.
//   * "cpu": positions and normals are recomputed in JavaScript for every
//     vertex of the mesh every frame — a straightforward CPU simulation.
// ---------------------------------------------------------------------------

export class TideOcean {
  /**
   * @param {number} radius       base ocean radius (scene units)
   * @param {number} widthSegs    sphere width segments
   * @param {number} heightSegs   sphere height segments
   */
  constructor(radius, widthSegs = 160, heightSegs = 96) {
    this.radius = radius;
    this.geometry = new THREE.SphereGeometry(radius, widthSegs, heightSegs);

    // Cache the unit direction of every vertex for the CPU pass.
    const pos = this.geometry.attributes.position;
    this.dirs = new Float32Array(pos.count * 3);
    for (let i = 0; i < pos.count; i++) {
      const x = pos.getX(i), y = pos.getY(i), z = pos.getZ(i);
      const l = Math.hypot(x, y, z);
      this.dirs[i * 3] = x / l;
      this.dirs[i * 3 + 1] = y / l;
      this.dirs[i * 3 + 2] = z / l;
    }

    // Tide state. moonDir is in the ocean mesh's object space; the ocean mesh
    // is not parented to the spinning Earth, so object space == world space.
    this.moonDir = new THREE.Vector3(1, 0, 0);
    this.tideAmp = 0; // scene units

    // Shared uniforms for the GPU pass.
    this._gpuUniforms = {
      uMoonDir: { value: this.moonDir.clone() },
      uTideAmp: { value: 0 },
    };

    this.mode = 'gpu';
    this.mesh = new THREE.Mesh(this.geometry, this._makeMaterial(this.mode));
    this.mesh.name = 'ocean';

    // ms spent by the CPU tide pass last frame (for the stats readout).
    this.lastCpuMs = 0;
  }

  _makeMaterial(mode) {
    const mat = new THREE.MeshPhongMaterial({
      color: 0x1a4f8f,
      specular: 0x88bbff,
      shininess: 120,
      transparent: true,
      opacity: 0.6,
    });

    if (mode === 'gpu') {
      mat.onBeforeCompile = (shader) => {
        shader.uniforms.uMoonDir = this._gpuUniforms.uMoonDir;
        shader.uniforms.uTideAmp = this._gpuUniforms.uTideAmp;
        shader.vertexShader =
          'uniform vec3 uMoonDir;\nuniform float uTideAmp;\n' +
          shader.vertexShader.replace(
            '#include <begin_vertex>',
            /* glsl */ `
            #include <begin_vertex>
            vec3 radDir = normalize(position);
            float cosPsi = dot(radDir, uMoonDir);
            float tide = uTideAmp * (1.5 * cosPsi * cosPsi - 0.5);
            transformed += radDir * tide;
            `
          );
        // Base-sphere normals are kept; the bulge is smooth and shallow, so
        // lighting error is negligible at these amplitudes.
      };
      mat.customProgramCacheKey = () => 'tide-ocean-gpu';
    }
    return mat;
  }

  /** Switch between 'cpu' and 'gpu' simulation passes. */
  setMode(mode) {
    if (mode === this.mode) return;
    this.mode = mode;
    this.mesh.material.dispose();
    this.mesh.material = this._makeMaterial(mode);
    if (mode === 'gpu') this._resetPositions(); // undo any CPU displacement
    else this._applyCpu();
  }

  setMoonDir(v) {
    this.moonDir.copy(v);
  }

  /** Amplitude A in scene length units. */
  setAmplitude(units) {
    this.tideAmp = units;
    this._gpuUniforms.uTideAmp.value = units;
    if (this.mode === 'cpu') this._applyCpu();
  }

  /** Called once per frame. */
  update() {
    if (this.mode === 'gpu') {
      this._gpuUniforms.uMoonDir.value.copy(this.moonDir);
      this.lastCpuMs = 0;
    } else {
      const t0 = performance.now();
      this._applyCpu();
      this.lastCpuMs = performance.now() - t0;
    }
  }

  _resetPositions() {
    const pos = this.geometry.attributes.position;
    const { dirs, radius } = this;
    for (let i = 0; i < pos.count; i++) {
      pos.setXYZ(
        i,
        dirs[i * 3] * radius,
        dirs[i * 3 + 1] * radius,
        dirs[i * 3 + 2] * radius
      );
    }
    pos.needsUpdate = true;
  }

  _applyCpu() {
    const pos = this.geometry.attributes.position;
    const { dirs, radius, tideAmp, moonDir } = this;
    const mx = moonDir.x, my = moonDir.y, mz = moonDir.z;
    for (let i = 0; i < pos.count; i++) {
      const dx = dirs[i * 3], dy = dirs[i * 3 + 1], dz = dirs[i * 3 + 2];
      const cosPsi = dx * mx + dy * my + dz * mz;
      const h = radius + tideAmp * (1.5 * cosPsi * cosPsi - 0.5);
      pos.setXYZ(i, dx * h, dy * h, dz * h);
    }
    pos.needsUpdate = true;
    // Re-shade correctly: normals must follow the deformed surface.
    this.geometry.computeVertexNormals();
  }
}
