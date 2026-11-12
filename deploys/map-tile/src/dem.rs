// Terrarium RGB elevation: height = (R * 256 + G + B / 256) - 32768.
// Tiles are the public AWS set, one PNG per slippy tile, no key.

use std::io::Cursor;

pub struct Grid
{
   pub z: u32,
   pub x: u32,
   pub y: u32,
   pub size: u32,
   pub elev: Vec<f32>,
}

pub fn decode (z: u32, x: u32, y: u32, png_bytes: &[u8]) -> Option<Grid>
{
   let mut decoder = png::Decoder::new (Cursor::new (png_bytes));
   decoder.set_transformations (png::Transformations::EXPAND | png::Transformations::STRIP_16);

   let mut reader = decoder.read_info ().ok ()?;
   let mut buf = vec![0u8; reader.output_buffer_size ()];
   let info = reader.next_frame (&mut buf).ok ()?;
   let w = info.width;
   let h = info.height;

   if w == 0 || h == 0 || w != h || w > 2048
   {
      return None;
   }

   let bpp = match info.color_type
   {
      png::ColorType::Rgb => 3,
      png::ColorType::Rgba => 4,
      png::ColorType::Grayscale => 1,
      png::ColorType::GrayscaleAlpha => 2,
      _ => return None,
   };

   let n = (w as usize) * (h as usize);
   if buf.len () < n * bpp
   {
      return None;
   }

   let mut elev = Vec::with_capacity (n);

   for i in 0..n
   {
      let p = i * bpp;
      let (r, g, b) = if bpp == 1
      {
         (buf[p], buf[p], buf[p])
      }
      else
      {
         (buf[p], buf[p + 1], buf[p + 2])
      };

      let h = (f64::from (r) * 256.0 + f64::from (g) + f64::from (b) / 256.0) - 32768.0;
      // Terrarium voids and corrupt pixels (-32768 and the like) are not
      // elevations; mark them as no data rather than let them drag the mesh.
      elev.push (if (-500.0..=9000.0).contains (&h) { h as f32 } else { f32::NAN });
   }

   Some (Grid { z, x, y, size: w, elev })
}

pub struct DemSet
{
   grids: Vec<Grid>,
}

impl DemSet
{
   pub fn new (grids: Vec<Grid>) -> Self
   {
      Self { grids }
   }

   fn sample_pixel (&self, z: u32, px: i32, py: i32) -> Option<f32>
   {
      if self.grids.is_empty ()
      {
         return None;
      }

      let size = self.grids[0].size as i32;
      if size <= 0
      {
         return None;
      }

      let tx = px.div_euclid (size) as u32;
      let ty = py.div_euclid (size) as u32;
      let ix = px.rem_euclid (size) as u32;
      let iy = py.rem_euclid (size) as u32;

      for grid in &self.grids
      {
         if grid.z == z && grid.x == tx && grid.y == ty && grid.size == size as u32
         {
            let i = (iy as usize) * (grid.size as usize) + (ix as usize);
            return grid.elev.get (i).copied ();
         }
      }

      None
   }

   // Bilinear height in metres at a longitude and latitude. None if no tile covers it.
   pub fn sample (&self, lon: f64, lat: f64) -> Option<f64>
   {
      let z = self.grids.first ()?.z;
      let size = self.grids[0].size as f64;
      let (xf, yf) = crate::geom::lonlat_to_tile (lon, lat, z);
      let px = xf * size;
      let py = yf * size;
      let x0 = px.floor () as i32;
      let y0 = py.floor () as i32;
      let tx = px - f64::from (x0);
      let ty = py - f64::from (y0);

      let e00 = self.sample_pixel (z, x0,     y0    )? as f64;
      let e10 = self.sample_pixel (z, x0 + 1, y0    ).unwrap_or (e00 as f32) as f64;
      let e01 = self.sample_pixel (z, x0,     y0 + 1).unwrap_or (e00 as f32) as f64;
      let e11 = self.sample_pixel (z, x0 + 1, y0 + 1).unwrap_or (e00 as f32) as f64;
      let e0 = e00 * (1.0 - tx) + e10 * tx;
      let e1 = e01 * (1.0 - tx) + e11 * tx;

      Some (e0 * (1.0 - ty) + e1 * ty)
   }
}
