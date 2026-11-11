// Terrain as an adaptive right-triangle irregular network (RTIN, the Martini
// scheme). The DEM is sampled once onto a (2^k+1)^2 grid, then each triangle
// is split only where the surface strays further than the error budget, so flat
// ground is a few large triangles and hills get detail. Ground overlays are
// clipped onto these same triangles, which keeps them exactly on the surface.

use crate::dem::DemSet;
use crate::geom::{Window, METRES_PER_DEG_LAT};
use crate::glb::{Mesh, Prim};
use crate::tess::{self, Pt, Tri};

const MIN_CELL_M: f64 = 5.0;
const MAX_ERROR_M: f64 = 0.3;
const MAX_LEVEL: u32 = 8;
const BUCKETS: usize = 24;
const WIRE_HALF_M: f64 = 0.2;
const WIRE_LIFT_M: f64 = 0.4;

/// Metres on a side for the finest ground patch. A missing, non-positive, or
/// non-finite `Data.Grid.dMinPatchM` keeps the 5 m default.
pub fn min_patch_metres (value: Option<f64>) -> f64
{
   match value
   {
      Some (v) if v.is_finite () && v > 0.0 => v,
      _ => MIN_CELL_M,
   }
}

/// Vertical error budget in metres for `Data.Grid.dMaxErrorM`.
pub fn max_error_metres (value: Option<f64>) -> f64
{
   match value
   {
      Some (v) if v.is_finite () && v > 0.0 => v,
      _ => MAX_ERROR_M,
   }
}

fn level (window: &Window, patch: f64) -> u32
{
   let short = window.width_m.min (window.length_m);
   let mut k = 1;

   while k < MAX_LEVEL && short / f64::from (1u32 << (k + 1)) >= patch
   {
      k += 1;
   }

   k
}

pub struct Tin
{
   pub elev_min: f64,
   pub elev_max: f64,
   pub cell_m: f64,
   pub v: Vec<[f64; 3]>,
   pub n: Vec<[f64; 3]>,
   pub t: Vec<[u32; 3]>,
   half_w: f64,
   half_l: f64,
   bounds: Vec<[f64; 4]>,
   buckets: Vec<Vec<u32>>,
}

pub fn build (window: &Window, dem: &DemSet, min_patch_m: f64, max_error_m: f64) -> Tin
{
   let k = level (window, min_patch_metres (Some (min_patch_m)));
   let size = (1usize << k) + 1;
   let tile = (size - 1) as f64;
   let (sx, sy) = (window.width_m / tile, window.length_m / tile);
   let mut raw = vec![f64::NAN; size * size];

   for gy in 0..size
   {
      for gx in 0..size
      {
         let e = -window.width_m * 0.5 + gx as f64 * sx;
         let n = window.length_m * 0.5 - gy as f64 * sy;
         let lat = window.lat + n / METRES_PER_DEG_LAT;
         let lon = window.lon + e / window.metres_per_deg_lon ();

         if let Some (h) = dem.sample (lon, lat)
         {
            raw[gy * size + gx] = h;
         }
      }
   }

   Tin::from_grid (window, k, raw, max_error_metres (Some (max_error_m)))
}

impl Tin
{
   // `raw` is row 0 = north, (2^k+1)^2 elevations; NaN where the DEM had no data.
   pub fn from_grid (window: &Window, k: u32, mut raw: Vec<f64>, max_error: f64) -> Tin
   {
      let size = (1usize << k) + 1;
      let tile = size - 1;
      let (sx, sy) = (window.width_m / tile as f64, window.length_m / tile as f64);
      let known = raw.iter ().copied ().filter (|h| h.is_finite ());
      let elev_min = known.clone ().fold (f64::INFINITY, f64::min);
      let elev_max = known.fold (f64::NEG_INFINITY, f64::max);
      let (elev_min, elev_max) = if elev_min.is_finite () { (elev_min, elev_max) } else { (0.0, 0.0) };

      for h in raw.iter_mut ()
      {
         *h = if h.is_finite () { *h - elev_min } else { 0.0 };
      }

      let tris = select (&raw, size, max_error);
      let (half_w, half_l) = (window.width_m * 0.5, window.length_m * 0.5);
      let mut slot = vec![u32::MAX; size * size];
      let mut v: Vec<[f64; 3]> = Vec::new ();
      let mut n: Vec<[f64; 3]> = Vec::new ();
      let mut t: Vec<[u32; 3]> = Vec::with_capacity (tris.len ());

      for tri in &tris
      {
         let mut ids = [0u32; 3];

         for (j, &(gx, gy)) in tri.iter ().enumerate ()
         {
            let cell = gy * size + gx;

            if slot[cell] == u32::MAX
            {
               slot[cell] = v.len () as u32;
               v.push ([-half_w + gx as f64 * sx, half_l - gy as f64 * sy, raw[cell]]);
               n.push (normal (&raw, size, gx, gy, sx, sy));
            }

            ids[j] = slot[cell];
         }

         let p = |i: u32| (v[i as usize][0], v[i as usize][1]);

         if tess::tri_area (&[p (ids[0]), p (ids[1]), p (ids[2])]) < 0.0
         {
            ids.swap (1, 2);
         }

         t.push (ids);
      }

      let mut bounds = Vec::with_capacity (t.len ());
      let mut buckets: Vec<Vec<u32>> = vec![Vec::new (); BUCKETS * BUCKETS];

      let mut tin = Tin {
         elev_min,
         elev_max,
         cell_m: window.width_m.min (window.length_m) / tile as f64,
         v,
         n,
         t,
         half_w,
         half_l,
         bounds: Vec::new (),
         buckets: Vec::new (),
      };

      for (i, tri) in tin.t.iter ().enumerate ()
      {
         let pts = tri.map (|j| tin.v[j as usize]);
         let b = [
            pts.iter ().map (|p| p[0]).fold (f64::INFINITY, f64::min),
            pts.iter ().map (|p| p[0]).fold (f64::NEG_INFINITY, f64::max),
            pts.iter ().map (|p| p[1]).fold (f64::INFINITY, f64::min),
            pts.iter ().map (|p| p[1]).fold (f64::NEG_INFINITY, f64::max),
         ];
         let (c0, r0) = tin.cell (b[0], b[2]);
         let (c1, r1) = tin.cell (b[1], b[3]);

         for r in r0..=r1
         {
            for c in c0..=c1
            {
               buckets[r * BUCKETS + c].push (i as u32);
            }
         }

         bounds.push (b);
      }

      tin.bounds = bounds;
      tin.buckets = buckets;
      tin
   }

   fn cell (&self, e: f64, n: f64) -> (usize, usize)
   {
      let c = ((e + self.half_w) / (self.half_w * 2.0) * BUCKETS as f64).floor ();
      let r = ((n + self.half_l) / (self.half_l * 2.0) * BUCKETS as f64).floor ();

      (c.clamp (0.0, (BUCKETS - 1) as f64) as usize, r.clamp (0.0, (BUCKETS - 1) as f64) as usize)
   }

   fn candidates (&self, b: [f64; 4]) -> Vec<u32>
   {
      let (c0, r0) = self.cell (b[0], b[2]);
      let (c1, r1) = self.cell (b[1], b[3]);
      let mut out: Vec<u32> = Vec::new ();

      for r in r0..=r1
      {
         for c in c0..=c1
         {
            out.extend_from_slice (&self.buckets[r * BUCKETS + c]);
         }
      }

      out.sort_unstable ();
      out.dedup ();
      out
   }

   fn corners (&self, i: usize) -> [[f64; 3]; 3]
   {
      self.t[i].map (|j| self.v[j as usize])
   }

   fn bary (&self, i: usize, e: f64, n: f64) -> [f64; 3]
   {
      let [a, b, c] = self.corners (i);
      let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);

      if d.abs () < 1e-12 { return [1.0, 0.0, 0.0]; }

      let l1 = ((b[1] - c[1]) * (e - c[0]) + (c[0] - b[0]) * (n - c[1])) / d;
      let l2 = ((c[1] - a[1]) * (e - c[0]) + (a[0] - c[0]) * (n - c[1])) / d;

      [l1, l2, 1.0 - l1 - l2]
   }

   fn locate (&self, e: f64, n: f64) -> (usize, [f64; 3])
   {
      let (e, n) = (e.clamp (-self.half_w, self.half_w), n.clamp (-self.half_l, self.half_l));
      let mut best = (0usize, [1.0, 0.0, 0.0], f64::NEG_INFINITY);

      for id in self.candidates ([e, e, n, n])
      {
         let i = id as usize;
         let w = self.bary (i, e, n);
         let m = w[0].min (w[1]).min (w[2]);

         if m >= -1e-9 { return (i, w); }

         if m > best.2 { best = (i, w, m); }
      }

      (best.0, best.1)
   }

   pub fn height_at (&self, e: f64, n: f64) -> f64
   {
      if self.t.is_empty () { return 0.0; }

      let (i, w) = self.locate (e, n);
      let c = self.corners (i);

      c[0][2] * w[0] + c[1][2] * w[1] + c[2][2] * w[2]
   }

   /// The terrain surface itself.
   pub fn surface (&self, prim: &mut Prim)
   {
      let base = prim.count ();

      for (p, n) in self.v.iter ().zip (&self.n)
      {
         prim.vert (*p, *n);
      }

      for t in &self.t
      {
         prim.tri (base + t[0], base + t[1], base + t[2]);
      }
   }

   /// Lays a planar triangle on the terrain: clipped against every terrain
   /// triangle beneath it, so the result shares the surface plane exactly.
   pub fn drape (&self, tri: &Tri, lift: f64, prim: &mut Prim)
   {
      let b = [
         tri.iter ().map (|p| p.0).fold (f64::INFINITY, f64::min),
         tri.iter ().map (|p| p.0).fold (f64::NEG_INFINITY, f64::max),
         tri.iter ().map (|p| p.1).fold (f64::INFINITY, f64::min),
         tri.iter ().map (|p| p.1).fold (f64::NEG_INFINITY, f64::max),
      ];

      if b[1] < -self.half_w || b[0] > self.half_w || b[3] < -self.half_l || b[2] > self.half_l
      {
         return;
      }

      let subject: [Pt; 3] = *tri;

      for id in self.candidates (b)
      {
         let i = id as usize;
         let tb = self.bounds[i];

         if tb[1] < b[0] || tb[0] > b[1] || tb[3] < b[2] || tb[2] > b[3]
         {
            continue;
         }

         let [p0, p1, p2] = self.corners (i);
         let by: Tri = [(p0[0], p0[1]), (p1[0], p1[1]), (p2[0], p2[1])];
         let poly = tess::clip (&subject, &by);

         if poly.len () < 3 || tess::area (&poly).abs () < 1e-7
         {
            continue;
         }

         let nn = self.t[i].map (|j| self.n[j as usize]);
         let base = prim.count ();

         for p in &poly
         {
            let w = self.bary (i, p.0, p.1);
            let z = p0[2] * w[0] + p1[2] * w[1] + p2[2] * w[2];
            let mut n = [0.0; 3];

            for a in 0..3
            {
               n[a] = nn[0][a] * w[0] + nn[1][a] * w[1] + nn[2][a] * w[2];
            }

            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt ().max (1e-9);
            prim.vert ([p.0, p.1, z + lift], [n[0] / l, n[1] / l, n[2] / l]);
         }

         for j in 1..poly.len () - 1
         {
            prim.tri (base, base + j as u32, base + j as u32 + 1);
         }
      }
   }

   /// Every unique triangle edge as a thin flat ribbon just above the surface.
   pub fn wire (&self) -> Mesh
   {
      let mut mesh = Mesh::default ();
      let prim = mesh.prim ([20, 23, 26]);
      let mut seen = std::collections::HashSet::new ();

      for t in &self.t
      {
         for j in 0..3
         {
            let (a, b) = (t[j], t[(j + 1) % 3]);

            if !seen.insert ((a.min (b), a.max (b))) { continue; }

            let (pa, pb) = (self.v[a as usize], self.v[b as usize]);
            let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
            let len = dx.hypot (dy);

            if len < 1e-3 { continue; }

            let (px, py) = (-dy / len * WIRE_HALF_M, dx / len * WIRE_HALF_M);
            let up = [0.0, 0.0, 1.0];
            let i = prim.vert ([pa[0] + px, pa[1] + py, pa[2] + WIRE_LIFT_M], up);
            prim.vert ([pa[0] - px, pa[1] - py, pa[2] + WIRE_LIFT_M], up);
            prim.vert ([pb[0] - px, pb[1] - py, pb[2] + WIRE_LIFT_M], up);
            prim.vert ([pb[0] + px, pb[1] + py, pb[2] + WIRE_LIFT_M], up);
            prim.tri (i, i + 1, i + 2);
            prim.tri (i, i + 2, i + 3);
         }
      }

      mesh
   }
}

fn normal (h: &[f64], size: usize, gx: usize, gy: usize, sx: f64, sy: f64) -> [f64; 3]
{
   let last = size - 1;
   let (x0, x1) = (gx.saturating_sub (1), (gx + 1).min (last));
   let (y0, y1) = (gy.saturating_sub (1), (gy + 1).min (last));
   let de = (h[gy * size + x1] - h[gy * size + x0]) / ((x1 - x0) as f64 * sx);
   let dn = (h[y0 * size + gx] - h[y1 * size + gx]) / ((y1 - y0) as f64 * sy);
   let l = (de * de + dn * dn + 1.0).sqrt ();

   [-de / l, -dn / l, 1.0 / l]
}

// Martini: build the error pyramid bottom-up, then split top-down.
fn select (h: &[f64], size: usize, max_error: f64) -> Vec<[(usize, usize); 3]>
{
   let tile = (size - 1) as i32;
   let count = (tile * tile * 2 - 2) as usize;
   let parents = count - (tile * tile) as usize;
   let mut coords: Vec<[i32; 4]> = Vec::with_capacity (count);

   for i in 0..count
   {
      let mut id = i + 2;
      let (mut ax, mut ay, mut bx, mut by, mut cx, mut cy) = (0, 0, 0, 0, 0, 0);

      if id & 1 != 0
      {
         bx = tile;
         by = tile;
         cx = tile;
      }
      else
      {
         ax = tile;
         ay = tile;
         cy = tile;
      }

      loop
      {
         id >>= 1;

         if id <= 1 { break; }

         let (mx, my) = ((ax + bx) >> 1, (ay + by) >> 1);

         if id & 1 != 0
         {
            bx = ax;
            by = ay;
            ax = cx;
            ay = cy;
         }
         else
         {
            ax = bx;
            ay = by;
            bx = cx;
            by = cy;
         }

         cx = mx;
         cy = my;
      }

      coords.push ([ax, ay, bx, by]);
   }

   let s = size as i32;
   let at = |x: i32, y: i32| (y * s + x) as usize;
   let mut errors = vec![0.0f64; size * size];

   for i in (0..count).rev ()
   {
      let [ax, ay, bx, by] = coords[i];
      let (mx, my) = ((ax + bx) >> 1, (ay + by) >> 1);
      let (cx, cy) = (mx + my - ay, my + ax - mx);
      let interpolated = (h[at (ax, ay)] + h[at (bx, by)]) * 0.5;
      let mid = at (mx, my);
      let mut e = errors[mid].max ((interpolated - h[mid]).abs ());

      if i < parents
      {
         let left = at ((ax + cx) >> 1, (ay + cy) >> 1);
         let right = at ((bx + cx) >> 1, (by + cy) >> 1);
         e = e.max (errors[left]).max (errors[right]);
      }

      errors[mid] = e;
   }

   let mut out = Vec::new ();
   split (0, 0, tile, tile, tile, 0, &errors, s, max_error, &mut out);
   split (tile, tile, 0, 0, 0, tile, &errors, s, max_error, &mut out);
   out
}

#[allow(clippy::too_many_arguments)]
fn split (ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32, errors: &[f64], s: i32, max_error: f64, out: &mut Vec<[(usize, usize); 3]>)
{
   let (mx, my) = ((ax + bx) >> 1, (ay + by) >> 1);

   if (ax - cx).abs () + (ay - cy).abs () > 1 && errors[(my * s + mx) as usize] > max_error
   {
      split (cx, cy, ax, ay, mx, my, errors, s, max_error, out);
      split (bx, by, cx, cy, mx, my, errors, s, max_error, out);
   }
   else
   {
      let p = |x: i32, y: i32| (x as usize, y as usize);
      out.push ([p (ax, ay), p (bx, by), p (cx, cy)]);
   }
}

#[cfg(test)]
mod tests
{
   use super::*;

   fn window () -> Window
   {
      Window { lat: 32.78, lon: -96.80, width_m: 480.0, length_m: 480.0, px_w: 256, px_h: 256 }
   }

   fn grid (k: u32, f: impl Fn (f64, f64) -> f64) -> Vec<f64>
   {
      let size = (1usize << k) + 1;
      let mut out = Vec::new ();

      for gy in 0..size
      {
         for gx in 0..size
         {
            out.push (f (gx as f64, gy as f64));
         }
      }

      out
   }

   fn covered (tin: &Tin) -> f64
   {
      tin.t.iter ().map (|t| {
         let p = |i: u32| (tin.v[i as usize][0], tin.v[i as usize][1]);
         tess::tri_area (&[p (t[0]), p (t[1]), p (t[2])])
      }).sum ()
   }

   #[test]
   fn missing_patch_stays_five_metres ()
   {
      assert! ((min_patch_metres (None) - 5.0).abs () < 1e-9);
      assert! ((min_patch_metres (Some (f64::NAN)) - 5.0).abs () < 1e-9);
      assert! ((min_patch_metres (Some (0.0)) - 5.0).abs () < 1e-9);
      assert! ((min_patch_metres (Some (-4.0)) - 5.0).abs () < 1e-9);
      assert! ((max_error_metres (None) - 0.3).abs () < 1e-9);
   }

   #[test]
   fn finest_cell_respects_min_patch ()
   {
      let w = window ();

      assert_eq! (level (&w, 5.0), 6);
      assert! (w.width_m / f64::from (1u32 << level (&w, 5.0)) >= 5.0);
      assert! (level (&w, 20.0) < level (&w, 5.0));
      assert_eq! (level (&w, 1000.0), 1);
   }

   #[test]
   fn plane_collapses_to_two_triangles ()
   {
      let k = 6;
      let tin = Tin::from_grid (&window (), k, grid (k, |x, y| 100.0 + 0.1 * x - 0.05 * y), 0.3);

      assert_eq! (tin.t.len (), 2);
      assert! ((covered (&tin) - 480.0 * 480.0).abs () < 1e-6);
   }

   #[test]
   fn a_hill_adds_detail_only_where_needed ()
   {
      let k = 6;
      let hill = |x: f64, y: f64| 20.0 * (-((x - 40.0).powi (2) + (y - 20.0).powi (2)) / 60.0).exp ();
      let tin = Tin::from_grid (&window (), k, grid (k, hill), 0.3);
      let full = 2usize << (2 * k);

      assert! (tin.t.len () > 20, "{}", tin.t.len ());
      assert! (tin.t.len () < full / 4, "{} of {full}", tin.t.len ());
      assert! ((covered (&tin) - 480.0 * 480.0).abs () < 1e-6);

      // The TIN stays within the budget of the sampled surface.
      let (e, n) = (-240.0 + 40.0 * 7.5, 240.0 - 20.0 * 7.5);
      let truth = hill (40.0, 20.0);
      let got = tin.height_at (e, n) + tin.elev_min;
      assert! ((got - truth).abs () <= 0.3 + 1e-6, "{got} vs {truth}");
   }

   #[test]
   fn drape_conserves_planar_area ()
   {
      let k = 5;
      let hill = |x: f64, y: f64| 10.0 * (-((x - 16.0).powi (2) + (y - 16.0).powi (2)) / 40.0).exp ();
      let tin = Tin::from_grid (&window (), k, grid (k, hill), 0.2);
      let mut mesh = Mesh::default ();
      let tri: Tri = [(-100.0, -100.0), (120.0, -90.0), (10.0, 130.0)];
      tin.drape (&tri, 0.1, mesh.prim ([1, 2, 3]));

      let flat = mesh.flat_area ();

      assert! ((flat - tess::tri_area (&tess::ccw (tri))).abs () < 1e-2, "{flat}");
   }
}
