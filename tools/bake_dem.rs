// Bake a compact land-height grid for the app's DEM relief displacement.
//
// Build & run (see tools/README.md for the source download):
//   rustc -O tools/bake_dem.rs -o bake_dem
//   ./bake_dem ETOPO_2022_v1_60s_N90W180_bed.nc assets/textures/etopo_dem.bin
//
// Reads a classic (netCDF-3) ETOPO-style grid: coord variables `lat`/`lon`
// and one elevation variable (`elev`, float32). The header is parsed with the
// classic grammar (tags big-endian u32, names padded to 4 bytes). Row/column
// order and spacing are discovered from the coord variables, never assumed.
// Elevations are read straight out of the (possibly huge) file buffer.
//
// Output: raw binary, box-averaged to 0.5 deg:
//   "GDEM" | u32 w | u32 h | u32 version(1) | i16 * (w*h)   (little-endian)
// Row 0 = lat +90 (north), col 0 = lon 0 deg E — the SphereGrid UV convention
// the app's textures already use.
//
// Sanity checks printed at bake time: Everest region ~ +8500 m, Mariana
// Trench ~ -10800 m, ocean fraction ~71%.

use std::convert::TryInto;
use std::fs;
use std::process;

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
    geo: String, // GeoTransform attribute if any file level defines one
}

fn type_size(nc_type: u32) -> usize {
    match nc_type {
        1 | 2 => 1, // BYTE, CHAR
        3 => 2,     // SHORT
        4 | 5 => 4, // INT, FLOAT
        6 => 8,     // DOUBLE
        8..=11 => 8,
        _ => 4,
    }
}

impl<'a> Reader<'a> {
    fn u32(&mut self) -> u32 {
        let s: [u8; 4] = self.b[self.p..self.p + 4].try_into().unwrap();
        self.p += 4;
        u32::from_be_bytes(s)
    }
    fn u64(&mut self) -> u64 {
        ((self.u32() as u64) << 32) | self.u32() as u64
    }
    /// nelems-length name, zero-padded so (4 + nelems) is a multiple of 4.
    fn name(&mut self) -> String {
        let n = self.u32() as usize;
        let s = String::from_utf8_lossy(&self.b[self.p..self.p + n]).into_owned();
        self.p += n + (4 - n % 4) % 4;
        s
    }
    fn skip_atts(&mut self) {
        if self.u32() == 0 {
            return; // ABSENT
        }
        let n = self.u32() as usize;
        for _ in 0..n {
            let name = self.name();
            let esize = type_size(self.u32());
            let nel = self.u32() as usize;
            let bytes = nel * esize;
            if name == "GeoTransform" && esize == 1 && self.geo.is_empty() {
                self.geo =
                    String::from_utf8_lossy(&self.b[self.p..self.p + nel]).into_owned();
            }
            self.p += bytes + (4 - bytes % 4) % 4;
        }
    }
}

struct Var {
    name: String,
    vtype: u32,
    begin: u64,
    nelems: usize,
}

fn read_coords(d: &[u8], v: &Var) -> Vec<f32> {
    let esize = type_size(v.vtype);
    (0..v.nelems)
        .map(|i| {
            let off = v.begin as usize + i * esize;
            if esize == 8 {
                let s: [u8; 8] = d[off..off + 8].try_into().unwrap();
                f64::from_be_bytes(s) as f32
            } else if esize == 4 {
                let s: [u8; 4] = d[off..off + 4].try_into().unwrap();
                f32::from_be_bytes(s)
            } else if esize == 2 {
                let s: [u8; 2] = d[off..off + 2].try_into().unwrap();
                i16::from_be_bytes(s) as f32
            } else {
                d[off] as f32
            }
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: bake_dem <etopo .nc> <out .bin>");
        process::exit(1);
    }
    let d = fs::read(&args[1]).expect("read input");
    if &d[0..3] != b"CDF" || (d[3] != 1 && d[3] != 2) {
        eprintln!("not a classic netCDF (CDF-1/2) file");
        process::exit(1);
    }
    let v2 = d[3] == 2;
    let mut r = Reader {
        b: &d,
        p: 4,
        geo: String::new(),
    };
    let _numrecs = r.u32();

    let mut dims: Vec<(String, u32)> = Vec::new();
    if r.u32() != 0 {
        let n = r.u32() as usize;
        for _ in 0..n {
            let name = r.name();
            dims.push((name, r.u32()));
        }
    }
    r.skip_atts(); // global attributes

    let mut vars: Vec<Var> = Vec::new();
    if r.u32() != 0 {
        let n = r.u32() as usize;
        for _ in 0..n {
            let name = r.name();
            let nd = r.u32() as usize;
            let mut nelems = 1usize;
            for _ in 0..nd {
                let id = r.u32() as usize;
                nelems *= dims[id].1 as usize;
            }
            r.skip_atts();
            let vtype = r.u32();
            let _vsize = r.u32();
            let begin = if v2 { r.u64() } else { r.u32() as u64 };
            vars.push(Var {
                name,
                vtype,
                begin,
                nelems,
            });
        }
    }

    let var = |name: &str| -> Option<usize> { vars.iter().position(|v| v.name == name) };
    let elev_name = if var("elev").is_some() {
        "elev"
    } else if var("elevation").is_some() {
        "elevation"
    } else if var("z").is_some() {
        "z" // ETOPO 2022 names its grids z
    } else {
        eprintln!(
            "no elev variable (vars: {:?})",
            vars.iter().map(|v| &v.name).collect::<Vec<_>>()
        );
        process::exit(1);
    };
    let ev = &vars[var(elev_name).expect("elev var")];
    let (lat, lon) = if !r.geo.is_empty() {
        // ETOPO 2022 stores an all-zeros `lat` coord array; the true grid is
        // in the GeoTransform attribute (pixel-corner convention, +0.5 -> centers).
        let g: Vec<f32> = r.geo.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if g.len() != 6 {
            eprintln!("unparsed GeoTransform: {:?}", r.geo);
            process::exit(1);
        }
        let dim = |n: &str| {
            dims.iter()
                .find(|(k, _)| k == n)
                .map(|(_, v)| *v as usize)
                .unwrap()
        };
        let (ny, nx) = (dim("lat"), dim("lon"));
        // GeoTransform claims a row order, but some pipelines (ETOPO 2022's
        // netCDF does exactly this) store rows south->north anyway. Settle it
        // empirically: the first storage row is the Arctic Ocean (deep
        // negative) in a north-first grid, or the Antarctic margin (near 0,
        // all ice or shelf) when flipped.
        let row_mean = |i: usize| -> f64 {
            let mut sm = 0.0;
            let mut n = 0;
            for j in (0..nx).step_by((nx / 40).max(1)) {
                let off = ev.begin as usize + (i * nx + j) * 4;
                let b: [u8; 4] = d[off..off + 4].try_into().unwrap();
                let v = f32::from_be_bytes(b);
                if v.is_finite() && v.abs() < 15000.0 {
                    sm += v as f64;
                    n += 1;
                }
            }
            sm / n as f64
        };
        let (m0, m1) = (row_mean(1), row_mean(ny - 2));
        let north_first = m0 < m1;
        println!(
            "row 0 mean {:>8.1}  row {} mean {:>8.1}  ->  storage {}",
            m0,
            ny - 2,
            m1,
            if north_first { "north-first" } else { "south-first (GeoTransform lied!)" }
        );
        let desc = g[5] < 0.0; // GeoTransform step sign
        let lat: Vec<f32> = if north_first == desc {
            (0..ny).map(|i| g[3] + (i as f32 + 0.5) * g[5]).collect()
        } else {
            (0..ny).map(|i| g[3] + (ny as f32 - 0.5 - i as f32) * g[5]).collect()
        };
        let lon: Vec<f32> = (0..nx).map(|j| g[0] + (j as f32 + 0.5) * g[1]).collect();
        (lat, lon)
    } else {
        (
            read_coords(&d, &vars[var("lat").expect("lat var")]),
            read_coords(&d, &vars[var("lon").expect("lon var")]),
        )
    };

    let ny = lat.len();
    let nx = lon.len();
    if ev.nelems != nx * ny {
        eprintln!("elev ({}) != lat*lon ({}x{})", ev.nelems, ny, nx);
        process::exit(1);
    }
    if ev.vtype != 5 {
        eprintln!("expected float32 elev, type {}", ev.vtype);
        process::exit(1);
    }

    // Uniform grid parameters, discovered from the coord arrays.
    let lat0 = lat[0];
    let dlat = (lat[ny - 1] - lat[0]) / (ny - 1) as f32;
    let lon0 = lon[0];
    let dlon = (lon[nx - 1] - lon[0]) / (nx - 1) as f32;
    println!(
        "input {}x{}  lon0 {:+.3} dlon {:+.5}  lat0 {:+.1} dlat {:+.5}",
        nx, ny, lon0, dlon, lat0, dlat
    );

    // Box-average decimation onto a 0.5 deg lat/lon lattice,
    // rows north->south, cols lon 0..360 E.
    let ow = 721u32;
    let oh = 361u32;
    let step = 0.5f32;
    let mut out: Vec<i16> = Vec::with_capacity((ow * oh) as usize);
    let mut ocean = 0u64;
    let mut peak_hi = f64::MIN;
    let mut peak_lo = f64::MAX;
    let mut raw_hi = f64::MIN;
    let mut raw_lo = f64::MAX;
    for ridx in 0..oh {
        let latv = 90.0 - ridx as f32 * step;
        let half = 0.5 * step;
        // Source rows within the window (uniform grid, index arithmetic).
        let a = ((latv + half - lat0) / dlat) as f64;
        let b = ((latv - half - lat0) / dlat) as f64;
        let (mut i0, mut i1) = (a.floor(), b.ceil());
        if i0 > i1 {
            std::mem::swap(&mut i0, &mut i1);
        }
        let i0 = (i0.max(0.0) as usize).min(ny - 1);
        let i1 = (i1.min(ny as f64 - 1.0) as usize).min(ny - 1);
        for cidx in 0..ow {
            let lonv = cidx as f32 * step;
            let j0 = ((lonv - half - lon0 as f32) / dlon).floor();
            let j1 = ((lonv + half - lon0 as f32) / dlon).ceil();
            let mut sum = 0.0f64;
            let mut cnt = 0u64;
            for i in i0..=i1 {
                for jj in j0 as i64..=j1 as i64 {
                    let j = jj.rem_euclid(nx as i64) as usize;
                    let val = {
                        let off = ev.begin as usize + (i * nx + j) * 4;
                        let s: [u8; 4] = d[off..off + 4].try_into().unwrap();
                        f32::from_be_bytes(s) as f64
                    };
                    if val.is_finite() && val.abs() < 15000.0 {
                        sum += val;
                        cnt += 1;
                        raw_hi = raw_hi.max(val);
                        raw_lo = raw_lo.min(val);
                    }
                }
            }
            let h = if cnt > 0 {
                (sum / cnt as f64).round().clamp(-30000.0, 30000.0) as i16
            } else {
                0
            };
            if (h as i32) <= 0 {
                ocean += 1;
            }
            peak_hi = peak_hi.max(h as f64);
            peak_lo = peak_lo.min(h as f64);
            out.push(h);
        }
    }

    let at = |latv: f32, lonv: f32| -> i16 {
        let r = ((90.0 - latv) / step).round() as usize;
        let c = ((lonv / step).round() as usize) % ow as usize;
        out[r * ow as usize + c]
    };
    println!("Everest region  {}", at(27.9, 86.9)); // 0.5 deg cell mean, expect ~ +5000
    println!("Mariana Trench  {}", at(11.3, 142.2)); // cell mean, expect ~ -7000
    println!("box-mean max/min  {} / {}", peak_hi, peak_lo);
    println!("source max/min    {} / {}", raw_hi, raw_lo); // ~8100/+8800, ~-10800
    println!(
        "ocean fraction  {:.1}%",
        100.0 * ocean as f64 / (ow * oh) as f64
    );
    if at(27.9, 86.9) < 4500 || at(11.3, 142.2) > -6500 || raw_hi < 8000.0 || raw_lo > -10000.0 {
        eprintln!("sanity checks failed - check axis/order handling");
        process::exit(1);
    }

    let mut bytes = Vec::with_capacity(16 + out.len() * 2);
    bytes.extend_from_slice(b"GDEM");
    bytes.extend_from_slice(&ow.to_le_bytes());
    bytes.extend_from_slice(&oh.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    for v in &out {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(&args[2], &bytes).expect("write output");
    println!(
        "wrote {} ({}x{}, {} KB)",
        args[2],
        ow,
        oh,
        bytes.len() / 1024
    );
}
