// Bake an equirectangular sea-level free-air gravity anomaly map from EGM96.
//
// Build & run (see tools/README.md for the coefficient downloads):
//   rustc -O tools/bake_gravity.rs -o bake_gravity
//   ./bake_gravity <egm96.egm.cof> <wgs84.egm.cof> gravity_anomaly.ppm
//   magick gravity_anomaly.ppm -quality 90 assets/textures/gravity_anomaly.jpg
//
// Reads GeographicLib-format .egm.cof coefficient files (little-endian):
//   [8-byte ID] then per set: N(i32) M(i32) Cnm(column-major m, n=m..N) Snm(m=1..N)
// Computes the spherical free-air anomaly exactly as GeographicLib's
// GravityModel::SphericalAnomaly:
//   Dg01 = -dT/dr - 2T/R = (GM/R) * sum_n (n-1) (a/R)^(n+1) *
//          sum_m Pbar_nm(sin theta) * (Dc cos m lambda + Ds sin m lambda)
// where D = EGM96 coefficients minus the WGS84 normal-field zonal terms
// (for egm96 the model and reference share a and GM; C00 = 1).
// Pbar uses GeographicLib's fully-normalized, Condon-Shortley-free recursion
// (Holmes & Featherstone 2012 Eqs. 11/13) - the convention baked into the .cof.
//
// Output: PPM, top row = north pole, left column = lon 0 deg E (SphereGrid UV).

use std::convert::TryInto;
use std::fs;
use std::sync::Arc;
use std::thread;

const A: f64 = 6378137.0; // ModelRadius == ReferenceRadius
const GM: f64 = 3986004.418e8; // ModelMass == ReferenceMass
const F: f64 = 1.0 / 298.257223563; // WGS84 flattening

struct Set {
    n: usize,
    c: Vec<f64>,
    s: Vec<f64>,
}

fn read_cof(path: &str) -> Vec<Set> {
    let d = fs::read(path).expect("read cof");
    let mut sets = Vec::new();
    let mut pos = 8usize;
    while pos + 8 <= d.len() {
        let n = i32::from_le_bytes(d[pos..pos + 4].try_into().unwrap()) as i64;
        let m = i32::from_le_bytes(d[pos + 4..pos + 8].try_into().unwrap()) as i64;
        pos += 8;
        if n < 0 || m < 0 {
            break;
        }
        let nc = ((m + 1) * (2 * n - m + 2) / 2) as usize;
        let ns = (m * (2 * n - m + 1) / 2) as usize;
        let mut c = Vec::with_capacity(nc);
        let mut s = Vec::with_capacity(ns);
        for i in 0..nc {
            c.push(f64::from_le_bytes(
                d[pos + 8 * i..pos + 8 * i + 8].try_into().unwrap(),
            ));
        }
        pos += 8 * nc;
        for i in 0..ns {
            s.push(f64::from_le_bytes(
                d[pos + 8 * i..pos + 8 * i + 8].try_into().unwrap(),
            ));
        }
        pos += 8 * ns;
        sets.push(Set {
            n: n as usize,
            c,
            s,
        });
    }
    sets
}

// Column starts in the column-major layout (verified against the N=M=3 doc
// example: C00,C10,C20,C30,C11,... and S11,S21,S31,S22,...).
fn cstart(nmax: usize, m: usize) -> usize {
    m * (2 * nmax - m + 3) / 2
}
fn sstart(nmax: usize, m: usize) -> usize {
    // m >= 1; sum_{k=1..m-1} (N-k+1)
    (m - 1) * (2 * nmax - m + 2) / 2
}

const W: usize = 1440;
const H: usize = 720;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: bake_gravity <egm96.egm.cof> <wgs84.egm.cof> [out.ppm]";
    let cof_path = args.get(1).expect(usage);
    let zref_path = args.get(2).expect(usage);
    let out_path = args
        .get(3)
        .map(String::as_str)
        .unwrap_or("gravity_anomaly.ppm");

    let sets = read_cof(cof_path);
    let egm = &sets[0];
    let nmax = egm.n; // 360
    eprintln!("EGM96 set: N={nmax}, C={} S={}", egm.c.len(), egm.s.len());
    assert_eq!(egm.c.len(), (nmax + 1) * (nmax + 2) / 2);
    assert_eq!(egm.s.len(), nmax * (nmax + 1) / 2);

    // WGS84 normal-field zonal terms, m=0 column (index == degree, entry 0 = 1)
    let zref = read_cof(zref_path);
    let zc = Arc::new(zref[0].c.clone());
    eprintln!("zonal reference degrees: {}", zc.len());

    // Recursion factors for Pbar[n,m] in st (fixed m), n = m+2..N
    //   P[n,m] = a[n,m]*st*P[n-1,m] - b[n,m]*P[n-2,m]
    let arec: Arc<Vec<Vec<f64>>> = Arc::new(
        (0..=nmax)
            .map(|m| {
                let mf = m as f64;
                (m + 2..=nmax)
                    .map(|n| {
                        let nf = n as f64;
                        ((2.0 * nf - 1.0) * (2.0 * nf + 1.0) / ((nf - mf) * (nf + mf))).sqrt()
                    })
                    .collect()
            })
            .collect(),
    );
    let brec: Arc<Vec<Vec<f64>>> = Arc::new(
        (0..=nmax)
            .map(|m| {
                let mf = m as f64;
                (m + 2..=nmax)
                    .map(|n| {
                        let nf = n as f64;
                        ((2.0 * nf + 1.0) * (nf + mf - 1.0) * (nf - mf - 1.0)
                            / ((nf - mf) * (nf + mf) * (2.0 * nf - 3.0)))
                            .sqrt()
                    })
                    .collect()
            })
            .collect(),
    );

    // trig tables: cos/sin(m*lon_j) at pixel centers lon_j = (j+0.5)*360/W
    let trig: Arc<Vec<(Vec<f64>, Vec<f64>)>> = Arc::new(
        (0..=nmax)
            .map(|m| {
                (
                    (0..W)
                        .map(|j| {
                            let lam =
                                (m as f64) * (j as f64 + 0.5) * std::f64::consts::TAU / W as f64;
                            lam.cos()
                        })
                        .collect(),
                    (0..W)
                        .map(|j| {
                            let lam =
                                (m as f64) * (j as f64 + 0.5) * std::f64::consts::TAU / W as f64;
                            lam.sin()
                        })
                        .collect(),
                )
            })
            .collect(),
    );

    let diag = [
        ("Tibet        ", 31.5, 87.5),
        ("Himalaya     ", 28.5, 85.5),
        ("AtlanticRidge", 0.5, 335.0),
        ("PeruTrench   ", -23.0, 285.0),
        ("HudsonBay    ", 58.0, 275.0),
        ("Iceland      ", 65.0, 340.0),
        ("MidPacific   ", -10.0, 200.0),
    ];

    let grid: Vec<f64> = vec![0.0; W * H];
    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(4);
    let egm_c = Arc::new(egm.c.clone());
    let egm_s = Arc::new(egm.s.clone());
    let ncoef = egm.c.len();
    let handles: Vec<_> = (0..nthreads)
        .map(|t| {
            let arec = Arc::clone(&arec);
            let brec = Arc::clone(&brec);
            let trig = Arc::clone(&trig);
            let zc = Arc::clone(&zc);
            let egm_c = Arc::clone(&egm_c);
            let egm_s = Arc::clone(&egm_s);
            thread::spawn(move || {
                let mut pbar = vec![0.0f64; ncoef];
                let mut row = vec![0.0f64; W];
                let mut rown = vec![0.0f64; W];
                let mut out: Vec<(usize, Vec<f64>, Vec<f64>)> = Vec::new();
                let mut i = t;
                while i < H {
                    let lat = (90.0 - (i as f64 + 0.5) * 180.0 / H as f64).to_radians();
                    let (slat, clat) = lat.sin_cos();
                    let e2 = F * (2.0 - F);
                    let dn = (1.0 - e2 * slat * slat).sqrt();
                    let xp = A * clat / dn;
                    let zp = A * (1.0 - e2) * slat / dn;
                    let r = (xp * xp + zp * zp).sqrt();
                    let st = zp / r; // sin(geocentric latitude) = engine t
                    let u = (1.0 - st * st).sqrt(); // cos(geocentric latitude)
                    let q = A / r;
                    // Pbar table, column-major by m (GeographicLib FULL convention)
                    pbar[cstart(nmax, 0)] = 1.0; // P00
                    for m in 0..=nmax {
                        let cs = cstart(nmax, m);
                        if m >= 1 {
                            let f = if m > 1 {
                                ((2 * m + 1) as f64 / (2 * m) as f64).sqrt()
                            } else {
                                3.0f64.sqrt()
                            };
                            pbar[cs] = u * f * pbar[cstart(nmax, m - 1)]; // Pmm
                        }
                        if m + 1 <= nmax {
                            pbar[cs + 1] = ((2 * m + 3) as f64).sqrt() * st * pbar[cs];
                        }
                        for (ii, n) in (m + 2..=nmax).enumerate() {
                            pbar[cs + n - m] = arec[m][ii] * st * pbar[cs + n - m - 1]
                                - brec[m][ii] * pbar[cs + n - m - 2];
                        }
                    }
                    row.iter_mut().for_each(|v| *v = 0.0);
                    rown.iter_mut().for_each(|v| *v = 0.0);
                    let gmf = GM / (A * r) * 1e5; // Dg01 scale: (1/R)*(GM/A)*q^(n+1) -> mGal
                                                  // normal gravity on the WGS84 ellipsoid (Helmert closed form)
                    let g0 = 9.7803253353 * (1.0 + 0.00193185265245 * slat * slat)
                        / (1.0 - e2 * slat * slat).sqrt();
                    let gnf = GM / A / g0; // geoid height scale: (GM/A)*q^(n+1)/g0 -> m
                    let mut qn = 1.0f64; // q^(n+1)
                    for n in 0..=nmax {
                        qn *= q;
                        let wfac = (n as f64 - 1.0) * qn * gmf;
                        let wfacn = qn * gnf;
                        for m in 0..=n {
                            let ci = cstart(nmax, m) + n - m;
                            // model C00 stored as 0 in .cof but set to 1 by GravityModel
                            let mut pc = if ci == 0 { 1.0 } else { egm_c[ci] };
                            let ps = if m == 0 {
                                // GravityModel sets the model C00 to 1 and the
                                // normal zonal C00 to 1 (the .cof file stores 0)
                                let z = if n == 0 {
                                    1.0
                                } else if n < zc.len() {
                                    zc[n]
                                } else {
                                    0.0
                                };
                                pc -= z;
                                0.0
                            } else {
                                egm_s[sstart(nmax, m) + n - m]
                            };
                            if pc == 0.0 && ps == 0.0 {
                                continue;
                            }
                            let pb = wfac * pbar[ci];
                            let pbn = wfacn * pbar[ci];
                            let (cm, sm) = &trig[m];
                            if ps == 0.0 {
                                for j in 0..W {
                                    row[j] += pb * pc * cm[j];
                                    rown[j] += pbn * pc * cm[j];
                                }
                            } else {
                                for j in 0..W {
                                    let cv = pc * cm[j] + ps * sm[j];
                                    row[j] += pb * cv;
                                    rown[j] += pbn * cv;
                                }
                            }
                        }
                    }
                    out.push((i, row.clone(), rown.clone()));
                    i += nthreads;
                }
                out
            })
        })
        .collect();
    let mut grid = grid;
    let mut geoid: Vec<f64> = vec![0.0; W * H];
    for h in handles {
        for (i, row, rown) in h.join().unwrap() {
            grid[i * W..(i + 1) * W].copy_from_slice(&row);
            geoid[i * W..(i + 1) * W].copy_from_slice(&rown);
        }
    }

    let g = grid.as_slice();
    let eval = |latd: f64, lond: f64| -> f64 {
        let i = (((90.0 - latd) / 180.0) * H as f64 - 0.5)
            .round()
            .clamp(0.0, H as f64 - 1.0) as usize;
        let j = ((lond.rem_euclid(360.0) / 360.0) * W as f64 - 0.5)
            .round()
            .clamp(0.0, W as f64 - 1.0) as usize;
        g[i * W + j]
    };
    for (name, la, lo) in diag {
        eprintln!("{name} lat{la:+.1} lon{lo:+.1}: {:8.1} mGal", eval(la, lo));
    }
    let mn = g.iter().cloned().fold(f64::INFINITY, f64::min);
    let mx = g.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let rms = (g.iter().map(|v| v * v).sum::<f64>() / g.len() as f64).sqrt();
    eprintln!("min {mn:.1}  max {mx:.1}  rms {rms:.1} mGal");

    // geoid diagnostics (metres) + raw grid for cross-checking vs WW15MGH.GRD
    let eg = geoid.as_slice();
    let evaln = |latd: f64, lond: f64| -> f64 {
        let i = (((90.0 - latd) / 180.0) * H as f64 - 0.5)
            .round()
            .clamp(0.0, H as f64 - 1.0) as usize;
        let j = ((lond.rem_euclid(360.0) / 360.0) * W as f64 - 0.5)
            .round()
            .clamp(0.0, W as f64 - 1.0) as usize;
        eg[i * W + j]
    };
    eprintln!(
        "geoid: NewGuinea(-6,137) {:.1}  IndiaOcean(10,70) {:.1}  GulfOfAlaska(55,225) {:.1}  min {:.1} max {:.1}",
        evaln(-6.0, 137.0),
        evaln(10.0, 70.0),
        evaln(55.0, 225.0),
        eg.iter().cloned().fold(f64::INFINITY, f64::min),
        eg.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
    );
    let mut raw = Vec::with_capacity(W * H * 8);
    for v in eg {
        raw.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(format!("{out_path}.geoid.bin"), raw).unwrap();

    // diverging colormap: white at 0, blue negative, red positive
    let scale = 120.0f64;
    let mut ppm = Vec::with_capacity(W * H * 3 + 32);
    ppm.extend_from_slice(format!("P6\n{W} {H}\n255\n").as_bytes());
    for v in g {
        let t = (v / scale).clamp(-1.0, 1.0);
        let (r, gc, b) = if t >= 0.0 {
            (245.0 - 40.0 * t, 245.0 - 205.0 * t, 245.0 - 210.0 * t)
        } else {
            let t = -t;
            (245.0 - 219.0 * t, 245.0 - 188.0 * t, 245.0 - 58.0 * t)
        };
        ppm.push(r.clamp(0.0, 255.0) as u8);
        ppm.push(gc.clamp(0.0, 255.0) as u8);
        ppm.push(b.clamp(0.0, 255.0) as u8);
    }
    fs::write(out_path, ppm).unwrap();
    eprintln!("wrote {out_path}");
}
