// The tile window and the slippy-tile math shared by the map fetch and the draw.

pub const VECTOR_Z: u32 = 14;
pub const METRES_PER_DEG_LAT: f64 = 111_320.0;

#[derive(Clone, Copy)]
pub struct Window
{
   pub lat: f64,
   pub lon: f64,
   pub width_m: f64,
   pub length_m: f64,
   pub px_w: i32,
   pub px_h: i32,
}

impl Window
{
   pub fn new (lat: f64, lon: f64, width_m: f64, length_m: f64, long_px: i32) -> Self
   {
      let lat = lat.clamp (-80.0, 80.0);
      let lon = ((lon + 180.0).rem_euclid (360.0)) - 180.0;
      let width_m = width_m.clamp (20.0, 8_000.0);
      let length_m = length_m.clamp (20.0, 8_000.0);
      let long_px = long_px.clamp (128, 512);
      let aspect = width_m / length_m;

      let (px_w, px_h) = if aspect >= 1.0
      {
         (long_px, ((long_px as f64) / aspect).round () as i32)
      }
      else
      {
         (((long_px as f64) * aspect).round () as i32, long_px)
      };

      Self
      {
         lat,
         lon,
         width_m,
         length_m,
         px_w: px_w.clamp (32, 512),
         px_h: px_h.clamp (32, 512),
      }
   }

   pub fn metres_per_deg_lon (&self) -> f64
   {
      METRES_PER_DEG_LAT * self.lat.to_radians ().cos ().max (0.05)
   }

   // West, south, east, north of the window, degrees.
   pub fn bounds (&self) -> (f64, f64, f64, f64)
   {
      let half_n = self.length_m * 0.5;
      let half_e = self.width_m * 0.5;
      let north = self.lat + half_n / METRES_PER_DEG_LAT;
      let south = self.lat - half_n / METRES_PER_DEG_LAT;
      let east  = self.lon + half_e / self.metres_per_deg_lon ();
      let west  = self.lon - half_e / self.metres_per_deg_lon ();

      (west, south, east, north)
   }

   // East and north metres from the window centre.
   pub fn to_local (&self, lat: f64, lon: f64) -> (f64, f64)
   {
      let east  = (lon - self.lon) * self.metres_per_deg_lon ();
      let north = (lat - self.lat) * METRES_PER_DEG_LAT;

      (east, north)
   }

   // Image pixels. North is the top row, east is to the right.
   pub fn to_px (&self, east: f64, north: f64) -> (f64, f64)
   {
      let x = (east + self.width_m * 0.5) / self.width_m * (self.px_w as f64);
      let y = (self.length_m * 0.5 - north) / self.length_m * (self.px_h as f64);

      (x, y)
   }
}

pub fn lonlat_to_tile (lon: f64, lat: f64, z: u32) -> (f64, f64)
{
   let n = f64::from (1u32 << z);
   let x = (lon + 180.0) / 360.0 * n;
   let r = lat.clamp (-85.0, 85.0).to_radians ();
   let y = (1.0 - ((r.tan () + 1.0 / r.cos ()).ln () / std::f64::consts::PI)) / 2.0 * n;

   (x, y)
}

pub fn tiles_covering (west: f64, south: f64, east: f64, north: f64, z: u32) -> Vec<(u32, u32, u32)>
{
   let n = (1u32 << z) - 1;
   let (x0, y_north) = lonlat_to_tile (west, north, z);
   let (x1, y_south) = lonlat_to_tile (east, south, z);
   let xa = (x0.floor () as i64).clamp (0, n as i64) as u32;
   let xb = (x1.floor () as i64).clamp (0, n as i64) as u32;
   let ya = (y_north.floor () as i64).clamp (0, n as i64) as u32;
   let yb = (y_south.floor () as i64).clamp (0, n as i64) as u32;

   let mut out = Vec::new ();

   for y in ya.min (yb)..=ya.max (yb)
   {
      for x in xa.min (xb)..=xa.max (xb)
      {
         out.push ((z, x, y));
      }
   }

   out
}

// A point inside a vector tile, in that tile's 0..extent coordinates, as
// east/north metres from the window centre.
pub fn tile_to_local (window: &Window, z: u32, tx: u32, ty: u32, extent: f64, px: f64, py: f64) -> (f64, f64)
{
   let n = f64::from (1u32 << z);
   let xf = f64::from (tx) + px / extent;
   let yf = f64::from (ty) + py / extent;
   let lon = xf / n * 360.0 - 180.0;
   let merc = std::f64::consts::PI * (1.0 - 2.0 * yf / n);
   let lat = merc.sinh ().atan ().to_degrees ();

   window.to_local (lat, lon)
}

pub fn dem_zoom (tile_count_at_15: usize) -> u32
{
   if tile_count_at_15 <= 6 { 15 } else { 14 }
}
