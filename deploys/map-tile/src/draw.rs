// Builds the 3D scene straight from the vector features.
//   ground:    adaptive terrain triangles, with land, water and road polygons
//              clipped onto them (one glb, one material per colour)
//   buildings: each footprint extruded as-is, roof earcut from the footprint
//   wire:      the terrain triangle edges, for the wireframe view

use crate::dem::DemSet;
use crate::geom::Window;
use crate::glb::{self, Mesh, Prim};
use crate::mvt::{self, Kind, Shape};
use crate::tess::{self, Poly};
use crate::tin::{self, Tin};

const BASE: [u8; 3] = [147, 154, 124];
const WATER: u32 = 0x3d7ea8;
const FOOT_M: f64 = 0.5;

const BIN_RGB: [[u8; 3]; 8] =
[
   [90, 86, 80],
   [132, 118, 104],
   [158, 144, 128],
   [176, 166, 154],
   [196, 190, 182],
   [210, 214, 216],
   [224, 228, 232],
   [236, 240, 244],
];

pub fn height_bin (height: f64) -> usize
{
   if height < 5.0 { 0 }
   else if height < 10.0 { 1 }
   else if height < 18.0 { 2 }
   else if height < 30.0 { 3 }
   else if height < 50.0 { 4 }
   else if height < 80.0 { 5 }
   else if height < 140.0 { 6 }
   else { 7 }
}

pub struct Built
{
   pub ground: Vec<u8>,
   pub buildings: Vec<u8>,
   pub wire: Vec<u8>,
   pub footprints: usize,
   pub terrain_tris: usize,
   pub ground_tris: usize,
   pub building_tris: usize,
   pub cell_m: f64,
   pub elev_min: f64,
   pub elev_max: f64,
   pub top: f64,
}

pub fn build (window: &Window, tiles: &[(u32, u32, u32, &[u8])], dem: &DemSet, min_patch_m: f64, max_error_m: f64) -> Built
{
   let mut shapes: Vec<Shape> = Vec::new ();

   for (z, x, y, bytes) in tiles
   {
      shapes.extend (mvt::decode (window, *z, *x, *y, bytes));
   }

   shapes.retain (|s| touches (window, &s.rings));

   let tin = tin::build (window, dem, min_patch_m, max_error_m);
   let mut ground = Mesh::default ();
   tin.surface (ground.prim (BASE));

   let mut landuse: Vec<(f64, Poly, [u8; 3])> = Vec::new ();
   let mut landcover: Vec<(f64, Poly, [u8; 3])> = Vec::new ();
   let mut water: Vec<(f64, Poly, [u8; 3])> = Vec::new ();
   let mut roads: Vec<(u32, f64, [u8; 3], Vec<tess::Pt>)> = Vec::new ();
   let mut buildings = Mesh::default ();
   let mut footprints = 0;
   let mut top = 0.0f64;

   for shape in &shapes
   {
      match shape.kind
      {
         Kind::Landuse (c) => collect (&mut landuse, &shape.rings, mix (BASE, glb::rgb (c), 0.5)),
         Kind::Landcover (c) => collect (&mut landcover, &shape.rings, mix (BASE, glb::rgb (c), 0.82)),
         Kind::Water => collect (&mut water, &shape.rings, glb::rgb (WATER)),
         Kind::Road { width_m, rank, color } =>
         {
            for ring in &shape.rings
            {
               roads.push ((rank, width_m, glb::rgb (color), ring.clone ()));
            }
         }
         Kind::Building { height, min_h, color } =>
         {
            let rgb = match color
            {
               Some (c) => quantise (glb::rgb (c)),
               None => BIN_RGB[height_bin (height)],
            };

            for poly in tess::polygons (&shape.rings)
            {
               // A feature can hold many footprints, and some run past the
               // ground; trim each one to the window.
               let Some (poly) = trim (window, poly) else { continue };

               footprints += 1;
               top = top.max (extrude (&tin, &poly, height, min_h, buildings.prim (rgb)));
            }
         }
      }
   }

   layer (&tin, &mut ground, landuse, 0.04);
   layer (&tin, &mut ground, landcover, 0.16);
   layer (&tin, &mut ground, water, 0.28);

   roads.sort_by_key (|r| r.0);

   for (rank, width_m, rgb, line) in &roads
   {
      let prim = ground.prim (*rgb);
      let lift = 0.42 + 0.06 * f64::from (*rank);

      for tri in tess::ribbon (line, width_m * 0.5)
      {
         tin.drape (&tri, lift, prim);
      }
   }

   let wire = tin.wire ();

   Built
   {
      ground: ground.pack (),
      buildings: buildings.pack (),
      wire: wire.pack (),
      footprints,
      terrain_tris: tin.t.len (),
      ground_tris: ground.triangles (),
      building_tris: buildings.triangles (),
      cell_m: tin.cell_m,
      elev_min: tin.elev_min,
      elev_max: tin.elev_max,
      top,
   }
}

fn collect (out: &mut Vec<(f64, Poly, [u8; 3])>, rings: &[Vec<tess::Pt>], rgb: [u8; 3])
{
   for poly in tess::polygons (rings)
   {
      out.push ((tess::poly_area (&poly), poly, rgb));
   }
}

// Big areas first, so a small patch sits a hair above the one that holds it.
fn layer (tin: &Tin, ground: &mut Mesh, mut polys: Vec<(f64, Poly, [u8; 3])>, base: f64)
{
   polys.sort_by (|a, b| b.0.partial_cmp (&a.0).unwrap_or (std::cmp::Ordering::Equal));

   for (k, (_, poly, rgb)) in polys.iter ().enumerate ()
   {
      let lift = base + 0.015 * k.min (6) as f64;
      let prim = ground.prim (*rgb);

      for tri in tess::triangulate (poly)
      {
         tin.drape (&tri, lift, prim);
      }
   }
}

// Walls rise from the terrain under every footprint vertex; the roof is flat
// at the lowest ground under the footprint plus the building height.
// Returns the roof height above the terrain minimum.
fn extrude (tin: &Tin, poly: &Poly, height: f64, min_h: f64, prim: &mut Prim) -> f64
{
   let ground_at = |p: &tess::Pt| tin.height_at (p.0, p.1);
   let base = poly.outer.iter ().map (ground_at).fold (f64::INFINITY, f64::min);
   let roof = base + height;
   let floating = min_h > 0.0;
   let floor = base + min_h;

   let (pts, tris) = tess::indexed (poly);
   let up = [0.0, 0.0, 1.0];
   let v0 = prim.count ();

   for p in &pts
   {
      prim.vert ([p.0, p.1, roof], up);
   }

   for t in &tris
   {
      prim.tri (v0 + t[0] as u32, v0 + t[1] as u32, v0 + t[2] as u32);
   }

   if floating && min_h > 0.5
   {
      let down = [0.0, 0.0, -1.0];
      let f0 = prim.count ();

      for p in &pts
      {
         prim.vert ([p.0, p.1, floor], down);
      }

      for t in &tris
      {
         prim.tri (f0 + t[0] as u32, f0 + t[2] as u32, f0 + t[1] as u32);
      }
   }

   let mut wall = |ring: &[tess::Pt]|
   {
      for i in 0..ring.len ()
      {
         let (a, b) = (ring[i], ring[(i + 1) % ring.len ()]);
         let (dx, dy) = (b.0 - a.0, b.1 - a.1);
         let len = dx.hypot (dy);

         if len < 0.05 { continue; }

         let n = [dy / len, -dx / len, 0.0];
         let (za, zb) = if floating { (floor, floor) } else { (ground_at (&a) - FOOT_M, ground_at (&b) - FOOT_M) };
         let w = prim.vert ([a.0, a.1, za], n);
         prim.vert ([b.0, b.1, zb], n);
         prim.vert ([b.0, b.1, roof], n);
         prim.vert ([a.0, a.1, roof], n);
         prim.tri (w, w + 1, w + 2);
         prim.tri (w, w + 2, w + 3);
      }
   };

   wall (&poly.outer);

   for h in &poly.holes
   {
      wall (h);
   }

   roof
}

fn mix (a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3]
{
   let f = |i: usize| (f64::from (a[i]) * (1.0 - t) + f64::from (b[i]) * t).round () as u8;

   [f (0), f (1), f (2)]
}

// Keeps the material count small when buildings carry their own colour.
fn quantise (c: [u8; 3]) -> [u8; 3]
{
   c.map (|v| (v & 0xe0) | 0x10)
}

// The outline cut to the window rectangle. A courtyard is kept only when it lies
// wholly inside, since one cut by the edge would no longer be a hole.
fn trim (window: &Window, poly: Poly) -> Option<Poly>
{
   let (hw, hl) = (window.width_m * 0.5, window.length_m * 0.5);
   let rect = [(-hw, -hl), (hw, -hl), (hw, hl), (-hw, hl)];
   let outer = tess::clip (&poly.outer, &rect);

   if outer.len () < 3 || tess::area (&outer) < 1.0
   {
      return None;
   }

   let inside = |h: &Vec<tess::Pt>| h.iter ().all (|p| p.0.abs () <= hw && p.1.abs () <= hl);
   let holes = poly.holes.into_iter ().filter (inside).collect ();

   Some (Poly { outer, holes })
}

fn touches (window: &Window, rings: &[Vec<tess::Pt>]) -> bool
{
   let (mut e0, mut e1, mut n0, mut n1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);

   for ring in rings
   {
      for (e, n) in ring
      {
         e0 = e0.min (*e);
         e1 = e1.max (*e);
         n0 = n0.min (*n);
         n1 = n1.max (*n);
      }
   }

   e1 >= -window.width_m * 0.5 && e0 <= window.width_m * 0.5 && n1 >= -window.length_m * 0.5 && n0 <= window.length_m * 0.5
}
