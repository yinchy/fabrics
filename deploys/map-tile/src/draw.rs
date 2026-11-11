// Raster of one map tile, then the same footprints as extruded boxes.
// A nearly rectangular footprint becomes one oriented box. Anything with a
// courtyard or a bent plan is split into axis-aligned prisms on a metre grid,
// so the volume follows the outline instead of a single bounding box.

use crate::dem::DemSet;
use crate::geom::Window;
use crate::mvt::{self, Kind, Shape};

pub const BIN_RGB: [[u8; 3]; 8] =
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

pub fn height_bin (height: f64) -> u8
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

#[derive(Clone, Copy)]
pub struct Prism
{
   pub east: f64,
   pub north: f64,
   pub z: f64,
   pub sx: f64,
   pub sy: f64,
   pub sz: f64,
   pub qz: f64,
   pub qw: f64,
   pub bin: u8,
}

pub struct Built
{
   #[allow(dead_code)]
   pub rgba: Vec<u8>,
   #[allow(dead_code)]
   pub width: i32,
   #[allow(dead_code)]
   pub height: i32,
   pub solid: Vec<u8>,
   pub wire: Vec<u8>,
   pub cell_m: f64,
   pub prisms: Vec<Prism>,
   pub buildings: usize,
   pub elev_min: f64,
   pub elev_max: f64,
}

const POSTER_EDGE: i32 = 256;

fn poster_dims (window: &Window) -> (i32, i32)
{
   let long = window.px_w.max (window.px_h).max (1);
   let edge = long.min (POSTER_EDGE);
   let pw = (i64::from (window.px_w) * i64::from (edge) / i64::from (long)).max (1) as i32;
   let ph = (i64::from (window.px_h) * i64::from (edge) / i64::from (long)).max (1) as i32;

   (pw, ph)
}

struct Paint<'a>
{
   window: &'a Window,
   w: i32,
   h: i32,
   rgba: Vec<u8>,
}

pub fn build (window: &Window, tiles: &[(u32, u32, u32, &[u8])], dem: &DemSet, min_patch_m: f64) -> Built
{
   let mut shapes = Vec::new ();

   for (z, x, y, bytes) in tiles
   {
      shapes.extend (mvt::decode (window, *z, *x, *y, bytes));
   }

   let (elev_min, elev_max) = relief (window, dem);
   let (poster_w, poster_h) = poster_dims (window);
   let mut paint = Paint::new (window, poster_w, poster_h);
   paint.ground (dem, elev_min, elev_max);

   let mut buildings: Vec<Shape> = Vec::new ();

   for shape in shapes
   {
      if !bbox_hits (window, &shape.rings)
      {
         continue;
      }

      match shape.kind
      {
         Kind::Landuse (rgb) => paint.fill (&shape.rings, rgb, 0.5),
         Kind::Landcover (rgb) => paint.fill (&shape.rings, rgb, 0.82),
         Kind::Water => paint.fill (&shape.rings, 0x3d7ea8, 1.0),
         Kind::Road { width_px, color } => paint.stroke (&shape.rings, width_px, color),
         Kind::Building { .. } => buildings.push (shape),
      }
   }

   buildings.sort_by (|a, b| building_key (a).partial_cmp (&building_key (b)).unwrap_or (std::cmp::Ordering::Equal));

   let mut prisms = Vec::new ();

   for shape in &buildings
   {
      let Kind::Building { height, min_h, color } = shape.kind else { continue };
      let rgb = color.unwrap_or_else (|| rgb_of (BIN_RGB[height_bin (height) as usize]));
      paint.fill (&shape.rings, rgb, 1.0);

      if prisms.len () < 700
      {
         extrude (window, dem, &shape.rings, height, min_h, elev_min, false, &mut prisms);
      }
   }

   let meshes = crate::mesh::terrain (window, dem, elev_min, &paint.rgba, paint.w, paint.h, min_patch_m);

   Built
   {
      rgba: paint.rgba,
      width: paint.w,
      height: paint.h,
      solid: meshes.solid,
      wire: meshes.wire,
      cell_m: meshes.cell_m,
      prisms,
      buildings: buildings.len (),
      elev_min,
      elev_max,
   }
}

fn building_key (shape: &Shape) -> f64
{
   if let Kind::Building { min_h, height, .. } = shape.kind { min_h * 1000.0 + height } else { 0.0 }
}

fn rgb_of (c: [u8; 3]) -> u32
{
   (u32::from (c[0]) << 16) | (u32::from (c[1]) << 8) | u32::from (c[2])
}

fn rgb_bytes (n: u32) -> [u8; 3]
{
   [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

fn relief (window: &Window, dem: &DemSet) -> (f64, f64)
{
   let mut lo = f64::MAX;
   let mut hi = f64::MIN;
   let step = (window.px_w.min (window.px_h) / 24).max (4);

   for y in (0..window.px_h).step_by (step as usize)
   {
      for x in (0..window.px_w).step_by (step as usize)
      {
         let (east, north) = px_local (window, f64::from (x), f64::from (y));
         let lat = window.lat + north / crate::geom::METRES_PER_DEG_LAT;
         let lon = window.lon + east / window.metres_per_deg_lon ();

         if let Some (h) = dem.sample (lon, lat)
         {
            lo = lo.min (h);
            hi = hi.max (h);
         }
      }
   }

   if lo > hi { (0.0, 0.0) } else { (lo, hi) }
}

fn px_local (window: &Window, x: f64, y: f64) -> (f64, f64)
{
   let east = -window.width_m * 0.5 + x / f64::from (window.px_w) * window.width_m;
   let north = window.length_m * 0.5 - y / f64::from (window.px_h) * window.length_m;

   (east, north)
}

fn bbox_hits (window: &Window, rings: &[Vec<(f64, f64)>]) -> bool
{
   let mut min_e = f64::MAX;
   let mut max_e = f64::MIN;
   let mut min_n = f64::MAX;
   let mut max_n = f64::MIN;

   for ring in rings
   {
      for (e, n) in ring
      {
         min_e = min_e.min (*e);
         max_e = max_e.max (*e);
         min_n = min_n.min (*n);
         max_n = max_n.max (*n);
      }
   }

   max_e >= -window.width_m * 0.5 && min_e <= window.width_m * 0.5
      && max_n >= -window.length_m * 0.5 && min_n <= window.length_m * 0.5
}

impl<'a> Paint<'a>
{
   fn new (window: &'a Window, w: i32, h: i32) -> Self
   {
      let n = (w as usize) * (h as usize);

      Self
      {
         window,
         w,
         h,
         rgba: vec![0u8; n * 4],
      }
   }

   fn at (&self, east: f64, north: f64) -> (f64, f64)
   {
      let x = (east + self.window.width_m * 0.5) / self.window.width_m * f64::from (self.w);
      let y = (self.window.length_m * 0.5 - north) / self.window.length_m * f64::from (self.h);

      (x, y)
   }

   fn metres (&self, x: f64, y: f64) -> (f64, f64)
   {
      let east = -self.window.width_m * 0.5 + x / f64::from (self.w) * self.window.width_m;
      let north = self.window.length_m * 0.5 - y / f64::from (self.h) * self.window.length_m;

      (east, north)
   }

   fn ground (&mut self, dem: &DemSet, lo: f64, hi: f64)
   {
      let span = (hi - lo).max (0.0);
      let calm = span < 4.0;

      for y in 0..self.h
      {
         for x in 0..self.w
         {
            let (east, north) = self.metres (f64::from (x) + 0.5, f64::from (y) + 0.5);
            let lat = self.window.lat + north / crate::geom::METRES_PER_DEG_LAT;
            let lon = self.window.lon + east / self.window.metres_per_deg_lon ();
            let t = if calm
            {
               0.35
            }
            else if let Some (h) = dem.sample (lon, lat)
            {
               ((h - lo) / span).clamp (0.0, 1.0)
            }
            else
            {
               0.35
            };
            let t = (t * 5.0).round () / 5.0;
            let rgb = lerp ([106, 122, 86], [186, 176, 154], t);
            self.put (x, y, rgb, 255);
         }
      }
   }

   fn put (&mut self, x: i32, y: i32, rgb: [u8; 3], a: u8)
   {
      if x < 0 || y < 0 || x >= self.w || y >= self.h
      {
         return;
      }

      let i = ((y as usize) * (self.w as usize) + (x as usize)) * 4;
      self.rgba[i] = rgb[0];
      self.rgba[i + 1] = rgb[1];
      self.rgba[i + 2] = rgb[2];
      self.rgba[i + 3] = a;
   }

   fn blend (&mut self, x: i32, y: i32, rgb: [u8; 3], cover: f64)
   {
      if x < 0 || y < 0 || x >= self.w || y >= self.h
      {
         return;
      }

      let i = ((y as usize) * (self.w as usize) + (x as usize)) * 4;
      for c in 0..3
      {
         let v = f64::from (self.rgba[i + c]) * (1.0 - cover) + f64::from (rgb[c]) * cover;
         self.rgba[i + c] = v.round () as u8;
      }
      self.rgba[i + 3] = 255;
   }

   fn fill (&mut self, rings: &[Vec<(f64, f64)>], color: u32, cover: f64)
   {
      let rgb = rgb_bytes (color);
      let w = self.w;
      let h = self.h;
      let mut xs: Vec<f64> = Vec::new ();

      for y in 0..h
      {
         xs.clear ();
         let yf = f64::from (y) + 0.5;

         for ring in rings
         {
            if ring.len () < 3
            {
               continue;
            }

            let n = ring.len ();

            for i in 0..n
            {
               let (e0, n0) = ring[i];
               let (e1, n1) = ring[(i + 1) % n];
               let (x0, y0) = self.at (e0, n0);
               let (x1, y1) = self.at (e1, n1);

               if (y0 <= yf && y1 > yf) || (y1 <= yf && y0 > yf)
               {
                  let t = (yf - y0) / (y1 - y0);
                  xs.push (x0 + t * (x1 - x0));
               }
            }
         }

         xs.sort_by (|a, b| a.partial_cmp (b).unwrap_or (std::cmp::Ordering::Equal));
         let mut i = 0;

         while i + 1 < xs.len ()
         {
            let mut a = xs[i].ceil () as i32;
            let mut b = xs[i + 1].floor () as i32;
            i += 2;

            if b < a { continue; }
            if a < 0 { a = 0; }
            if b > w { b = w; }

            for x in a..b
            {
               if cover >= 0.99
               {
                  self.put (x, y, rgb, 255);
               }
               else
               {
                  self.blend (x, y, rgb, cover);
               }
            }
         }
      }
   }

   fn stroke (&mut self, lines: &[Vec<(f64, f64)>], radius: i32, color: u32)
   {
      let rgb = rgb_bytes (color);
      let r = radius.max (1);

      for line in lines
      {
         if line.len () < 2
         {
            continue;
         }

         for i in 0..line.len () - 1
         {
            let (x0, y0) = self.at (line[i].0, line[i].1);
            let (x1, y1) = self.at (line[i + 1].0, line[i + 1].1);
            let dx = x1 - x0;
            let dy = y1 - y0;
            let steps = (dx * dx + dy * dy).sqrt ().ceil ().max (1.0) as i32;

            for s in 0..=steps
            {
               let t = f64::from (s) / f64::from (steps);
               let x = (x0 + dx * t).round () as i32;
               let y = (y0 + dy * t).round () as i32;

               for oy in -(r - 1)..=(r - 1)
               {
                  for ox in -(r - 1)..=(r - 1)
                  {
                     self.put (x + ox, y + oy, rgb, 255);
                  }
               }
            }
         }
      }
   }
}

fn lerp (a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3]
{
   let t = t.clamp (0.0, 1.0);
   [
      (f64::from (a[0]) + (f64::from (b[0]) - f64::from (a[0])) * t).round () as u8,
      (f64::from (a[1]) + (f64::from (b[1]) - f64::from (a[1])) * t).round () as u8,
      (f64::from (a[2]) + (f64::from (b[2]) - f64::from (a[2])) * t).round () as u8,
   ]
}

fn extrude (window: &Window, dem: &DemSet, rings: &[Vec<(f64, f64)>], height: f64, min_h: f64, elev_min: f64, flat: bool, prisms: &mut Vec<Prism>)
{
   let outer: Vec<&[(f64, f64)]> = rings.iter ().filter (|r| r.len () >= 3).map (|r| r.as_slice ()).collect ();

   if outer.is_empty () || prisms.len () >= 700
   {
      return;
   }

   let bin = height_bin (height);
   let sz = (height - min_h).max (0.4);

   if outer.len () == 1 && outer[0].len () <= 24
   {
      if let Some (box_) = oriented_box (outer[0])
      {
         let area = ring_area (outer[0]).abs ();

         if area > 1.0 && area / (box_.2 * box_.3) >= 0.86
         {
            push_prism (window, dem, box_.0, box_.1, box_.2, box_.3, box_.4, sz, min_h, elev_min, flat, bin, prisms);
            return;
         }
      }
   }

   let cell = (window.width_m.max (window.length_m) / 110.0).max (2.4);
   let cols = ((window.width_m / cell).ceil () as i32).clamp (1, 400);
   let rows = ((window.length_m / cell).ceil () as i32).clamp (1, 400);
   let mut min_c = cols;
   let mut min_r = rows;
   let mut max_c = 0i32;
   let mut max_r = 0i32;

   for ring in &outer
   {
      for (e, n) in *ring
      {
         let c = (((e + window.width_m * 0.5) / cell).floor () as i32).clamp (0, cols - 1);
         let r = (((window.length_m * 0.5 - n) / cell).floor () as i32).clamp (0, rows - 1);
         min_c = min_c.min (c);
         max_c = max_c.max (c + 1);
         min_r = min_r.min (r);
         max_r = max_r.max (r + 1);
      }
   }

   if max_c <= min_c || max_r <= min_r
   {
      return;
   }

   let bw = max_c - min_c;
   let bh = max_r - min_r;

   if bw * bh > 80_000
   {
      return;
   }

   let mut mask = vec![false; (bw * bh) as usize];
   let mut xs: Vec<f64> = Vec::new ();

   for row in 0..bh
   {
      xs.clear ();
      let north = window.length_m * 0.5 - (f64::from (min_r + row) + 0.5) * cell;

      for ring in &outer
      {
         let n = ring.len ();

         for i in 0..n
         {
            let (e0, n0) = ring[i];
            let (e1, n1) = ring[(i + 1) % n];

            if (n0 <= north && n1 > north) || (n1 <= north && n0 > north)
            {
               let t = (north - n0) / (n1 - n0);
               xs.push (e0 + t * (e1 - e0));
            }
         }
      }

      xs.sort_by (|a, b| a.partial_cmp (b).unwrap_or (std::cmp::Ordering::Equal));
      let mut i = 0;

      while i + 1 < xs.len ()
      {
         let a = (((xs[i] + window.width_m * 0.5) / cell).ceil () as i32) - min_c;
         let b = (((xs[i + 1] + window.width_m * 0.5) / cell).floor () as i32) - min_c;
         i += 2;

         let a = a.max (0);
         let b = b.min (bw);

         for c in a..b
         {
            mask[(row * bw + c) as usize] = true;
         }
      }
   }

   for run in merge_rects (&mask, bw, bh)
   {
      let east0 = -window.width_m * 0.5 + f64::from (min_c + run.x0) * cell;
      let east1 = -window.width_m * 0.5 + f64::from (min_c + run.x1) * cell;
      let north1 = window.length_m * 0.5 - f64::from (min_r + run.y) * cell;
      let north0 = window.length_m * 0.5 - f64::from (min_r + run.y + run.h) * cell;

      push_prism (window, dem, (east0 + east1) * 0.5, (north0 + north1) * 0.5, east1 - east0, north1 - north0, 0.0, sz, min_h, elev_min, flat, bin, prisms);

      if prisms.len () >= 700
      {
         break;
      }
   }
}

fn push_prism (window: &Window, dem: &DemSet, east: f64, north: f64, sx: f64, sy: f64, angle: f64, sz: f64, min_h: f64, elev_min: f64, flat: bool, bin: u8, prisms: &mut Vec<Prism>)
{
   if sx < 0.6 || sy < 0.6 || prisms.len () >= 700
   {
      return;
   }

   let lat = window.lat + north / crate::geom::METRES_PER_DEG_LAT;
   let lon = window.lon + east / window.metres_per_deg_lon ();
   let elev = dem.sample (lon, lat).unwrap_or (elev_min);
   let base = if flat { 0.0 } else { (elev - elev_min).max (0.0) };
   let half = angle * 0.5;

   prisms.push (Prism
   {
      east,
      north,
      z: base + min_h + sz * 0.5 + 0.2,
      sx,
      sy,
      sz,
      qz: half.sin (),
      qw: half.cos (),
      bin,
   });
}

// Centre, width along the rotated X axis, depth along Y, yaw from east toward north.
fn oriented_box (ring: &[(f64, f64)]) -> Option<(f64, f64, f64, f64, f64)>
{
   if ring.len () < 3
   {
      return None;
   }

   let mut best_area = f64::MAX;
   let mut best = None;

   for i in 0..ring.len ()
   {
      let (x0, y0) = ring[i];
      let (x1, y1) = ring[(i + 1) % ring.len ()];
      let dx = x1 - x0;
      let dy = y1 - y0;
      let len = (dx * dx + dy * dy).sqrt ();

      if len < 1.0e-3
      {
         continue;
      }

      let c = dx / len;
      let s = dy / len;
      let mut min_x = f64::MAX;
      let mut max_x = f64::MIN;
      let mut min_y = f64::MAX;
      let mut max_y = f64::MIN;

      for (x, y) in ring
      {
         let px = x * c + y * s;
         let py = -x * s + y * c;
         min_x = min_x.min (px);
         max_x = max_x.max (px);
         min_y = min_y.min (py);
         max_y = max_y.max (py);
      }

      let w = max_x - min_x;
      let d = max_y - min_y;
      let area = w * d;

      if area < best_area && w > 0.4 && d > 0.4
      {
         let rcx = (min_x + max_x) * 0.5;
         let rcy = (min_y + max_y) * 0.5;
         best_area = area;
         best = Some ((rcx * c - rcy * s, rcx * s + rcy * c, w, d, s.atan2 (c)));
      }
   }

   best
}

fn ring_area (ring: &[(f64, f64)]) -> f64
{
   let mut a = 0.0;
   let n = ring.len ();

   for i in 0..n
   {
      let (x0, y0) = ring[i];
      let (x1, y1) = ring[(i + 1) % n];
      a += x0 * y1 - x1 * y0;
   }

   a * 0.5
}

struct Run
{
   x0: i32,
   x1: i32,
   y: i32,
   h: i32,
}

fn merge_rects (mask: &[bool], w: i32, h: i32) -> Vec<Run>
{
   let mut open: Vec<Run> = Vec::new ();
   let mut done: Vec<Run> = Vec::new ();

   for y in 0..h
   {
      let mut row: Vec<(i32, i32)> = Vec::new ();
      let mut x = 0;

      while x < w
      {
         if !mask[(y * w + x) as usize]
         {
            x += 1;
            continue;
         }

         let x0 = x;
         while x < w && mask[(y * w + x) as usize] { x += 1; }
         row.push ((x0, x));
      }

      let mut keep = vec![false; open.len ()];
      let mut next: Vec<Run> = Vec::new ();

      for (x0, x1) in row
      {
         if let Some (i) = open.iter ().position (|r| r.x0 == x0 && r.x1 == x1)
         {
            open[i].h += 1;
            keep[i] = true;
         }
         else
         {
            next.push (Run { x0, x1, y, h: 1 });
         }
      }

      for (i, run) in open.into_iter ().enumerate ()
      {
         if keep[i] { next.push (run); }
         else { done.push (run); }
      }

      open = next;
   }

   done.extend (open);
   done
}
