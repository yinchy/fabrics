// Terrain as one glTF mesh. Cells are never finer than the minimum patch
// (5 m unless Data.Grid.dMinPatchM says otherwise), and a large window uses a
// coarser grid so the mesh stays about 96 cells on its long side.
// The engine turns glTF Y-up into world Z-up with Rx(+90): (x, y, z) -> (x, -z, y).
// Stored vertices are (east, up, -north).

use crate::dem::DemSet;
use crate::geom::Window;

const MIN_CELL_M: f64 = 5.0;
const MAX_CELLS: i32 = 96;

/// Metres on a side for one ground patch. A missing, non-positive, or
/// non-finite `Data.Grid.dMinPatchM` keeps the 5 m default.
pub fn min_patch_metres (value: Option<f64>) -> f64
{
   match value
   {
      Some (v) if v.is_finite () && v > 0.0 => v,
      _ => MIN_CELL_M,
   }
}
const WIRE_HALF_M: f32 = 0.22;
const WIRE_LIFT_M: f32 = 0.35;

pub struct Meshes
{
   pub solid: Vec<u8>,
   pub wire: Vec<u8>,
   pub cell_m: f64,
}

pub fn terrain (window: &Window, dem: &DemSet, elev_min: f64, rgba: &[u8], tex_w: i32, tex_h: i32, min_patch_m: f64) -> Meshes
{
   let (cols, rows, cell_e, cell_n) = grid (window, min_patch_m);
   let cell_m = cell_e.min (cell_n);
   let nx = cols + 1;
   let ny = rows + 1;
   let nvert = (nx * ny) as usize;
   let mut east = vec![0.0f32; nvert];
   let mut north = vec![0.0f32; nvert];
   let mut up = vec![0.0f32; nvert];

   for r in 0..ny
   {
      for c in 0..nx
      {
         let e = -window.width_m * 0.5 + f64::from (c) * cell_e;
         let n = window.length_m * 0.5 - f64::from (r) * cell_n;
         let lat = window.lat + n / crate::geom::METRES_PER_DEG_LAT;
         let lon = window.lon + e / window.metres_per_deg_lon ();
         let h = dem.sample (lon, lat).unwrap_or (elev_min) - elev_min;
         let i = (r * nx + c) as usize;
         east[i] = e as f32;
         north[i] = n as f32;
         up[i] = h as f32;
      }
   }

   let png = encode_png (rgba, tex_w, tex_h).unwrap_or_default ();
   let solid = if png.is_empty () { Vec::new () } else { solid_glb (&east, &north, &up, nx, ny, &png) };
   let wire = wire_glb (&east, &north, &up, nx, ny);

   Meshes { solid, wire, cell_m }
}

fn grid (window: &Window, min_patch_m: f64) -> (i32, i32, f64, f64)
{
   let patch = min_patch_metres (Some (min_patch_m));
   let cols = ((window.width_m / patch).floor () as i32).clamp (1, MAX_CELLS);
   let rows = ((window.length_m / patch).floor () as i32).clamp (1, MAX_CELLS);
   let cell_e = window.width_m / f64::from (cols);
   let cell_n = window.length_m / f64::from (rows);

   (cols, rows, cell_e, cell_n)
}

fn encode_png (rgba: &[u8], w: i32, h: i32) -> Option<Vec<u8>>
{
   if w < 1 || h < 1 || rgba.len () < (w as usize) * (h as usize) * 4
   {
      return None;
   }

   let mut out = Vec::new ();
   let mut encoder = png::Encoder::new (&mut out, w as u32, h as u32);
   encoder.set_color (png::ColorType::Rgba);
   encoder.set_depth (png::BitDepth::Eight);
   encoder.write_header ().ok ()?.write_image_data (rgba).ok ()?;
   Some (out)
}

fn solid_glb (east: &[f32], north: &[f32], up: &[f32], nx: i32, ny: i32, png: &[u8]) -> Vec<u8>
{
   let nvert = east.len ();
   let cols = nx - 1;
   let rows = ny - 1;
   let mut pos = Vec::with_capacity (nvert * 12);
   let mut nrm = Vec::with_capacity (nvert * 12);
   let mut uv = Vec::with_capacity (nvert * 8);
   let mut idx = Vec::with_capacity ((cols * rows * 6) as usize * 2);
   let mut pmin = [f32::MAX; 3];
   let mut pmax = [f32::MIN; 3];

   for r in 0..ny
   {
      for c in 0..nx
      {
         let i = (r * nx + c) as usize;
         let (gx, gy, gz) = gltf_pos (east[i], north[i], up[i]);
         pmin[0] = pmin[0].min (gx);
         pmin[1] = pmin[1].min (gy);
         pmin[2] = pmin[2].min (gz);
         pmax[0] = pmax[0].max (gx);
         pmax[1] = pmax[1].max (gy);
         pmax[2] = pmax[2].max (gz);
         push_f32 (&mut pos, gx);
         push_f32 (&mut pos, gy);
         push_f32 (&mut pos, gz);

         let (wx, wy, wz) = normal_at (east, north, up, nx, ny, c, r);
         let (nx_, ny_, nz_) = gltf_pos (wx, wy, wz);
         push_f32 (&mut nrm, nx_);
         push_f32 (&mut nrm, ny_);
         push_f32 (&mut nrm, nz_);

         push_f32 (&mut uv, c as f32 / cols.max (1) as f32);
         push_f32 (&mut uv, r as f32 / rows.max (1) as f32);
      }
   }

   for r in 0..rows
   {
      for c in 0..cols
      {
         let v00 = (r * nx + c) as u32;
         let v10 = v00 + 1;
         let v01 = v00 + nx as u32;
         let v11 = v01 + 1;
         push_u16 (&mut idx, v00);
         push_u16 (&mut idx, v10);
         push_u16 (&mut idx, v11);
         push_u16 (&mut idx, v00);
         push_u16 (&mut idx, v11);
         push_u16 (&mut idx, v01);
      }
   }

   let mut bin = Vec::new ();
   let o_pos = align (&mut bin);
   bin.extend_from_slice (&pos);
   let o_nrm = align (&mut bin);
   bin.extend_from_slice (&nrm);
   let o_uv = align (&mut bin);
   bin.extend_from_slice (&uv);
   let o_idx = align (&mut bin);
   bin.extend_from_slice (&idx);
   let o_png = align (&mut bin);
   bin.extend_from_slice (png);
   align (&mut bin);

   let json = format! (
      "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"mesh\":0}}],\
       \"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1,\"TEXCOORD_0\":2}},\"indices\":3,\"material\":0}}]}}],\
       \"materials\":[{{\"doubleSided\":true,\"pbrMetallicRoughness\":{{\"baseColorTexture\":{{\"index\":0}},\"metallicFactor\":0,\"roughnessFactor\":0.84}}}}],\
       \"textures\":[{{\"sampler\":0,\"source\":0}}],\"samplers\":[{{\"magFilter\":9729,\"minFilter\":9987}}],\
       \"images\":[{{\"mimeType\":\"image/png\",\"bufferView\":4}}],\"buffers\":[{{\"byteLength\":{blen}}}],\
       \"bufferViews\":[{{\"buffer\":0,\"byteOffset\":{o_pos},\"byteLength\":{n_pos},\"target\":34962}},\
       {{\"buffer\":0,\"byteOffset\":{o_nrm},\"byteLength\":{n_nrm},\"target\":34962}},\
       {{\"buffer\":0,\"byteOffset\":{o_uv},\"byteLength\":{n_uv},\"target\":34962}},\
       {{\"buffer\":0,\"byteOffset\":{o_idx},\"byteLength\":{n_idx},\"target\":34963}},\
       {{\"buffer\":0,\"byteOffset\":{o_png},\"byteLength\":{n_png}}}],\
       \"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":{nvert},\"type\":\"VEC3\",\"min\":[{min0:.5},{min1:.5},{min2:.5}],\"max\":[{max0:.5},{max1:.5},{max2:.5}]}},
       {{\"bufferView\":1,\"componentType\":5126,\"count\":{nvert},\"type\":\"VEC3\"}},\
       {{\"bufferView\":2,\"componentType\":5126,\"count\":{nvert},\"type\":\"VEC2\"}},\
       {{\"bufferView\":3,\"componentType\":5123,\"count\":{nidx},\"type\":\"SCALAR\"}}]}}",
      blen = bin.len (),
      o_pos = o_pos, n_pos = pos.len (),
      o_nrm = o_nrm, n_nrm = nrm.len (),
      o_uv = o_uv, n_uv = uv.len (),
      o_idx = o_idx, n_idx = idx.len (),
      o_png = o_png, n_png = png.len (),
      nvert = nvert,
      nidx = idx.len () / 2,
      min0 = pmin[0], min1 = pmin[1], min2 = pmin[2],
      max0 = pmax[0], max1 = pmax[1], max2 = pmax[2],
   );

   pack_glb (&json, &bin)
}

fn wire_glb (east: &[f32], north: &[f32], up: &[f32], nx: i32, ny: i32) -> Vec<u8>
{
   let cols = nx - 1;
   let rows = ny - 1;
   let mut pos = Vec::new ();
   let mut idx = Vec::new ();

   let edge = |pos: &mut Vec<u8>, idx: &mut Vec<u8>, east: &[f32], north: &[f32], up: &[f32], nx: i32, c0: i32, r0: i32, c1: i32, r1: i32|
   {
      let i0 = (r0 * nx + c0) as usize;
      let i1 = (r1 * nx + c1) as usize;
      ribbon (pos, idx, east[i0], north[i0], up[i0], east[i1], north[i1], up[i1]);
   };

   for r in 0..=rows
   {
      for c in 0..cols
      {
         edge (&mut pos, &mut idx, east, north, up, nx, c, r, c + 1, r);
      }
   }

   for r in 0..rows
   {
      for c in 0..=cols
      {
         edge (&mut pos, &mut idx, east, north, up, nx, c, r, c, r + 1);
      }
   }

   for r in 0..rows
   {
      for c in 0..cols
      {
         edge (&mut pos, &mut idx, east, north, up, nx, c, r, c + 1, r + 1);
         edge (&mut pos, &mut idx, east, north, up, nx, c + 1, r, c, r + 1);
      }
   }

   if pos.is_empty ()
   {
      return Vec::new ();
   }

   let nvert = pos.len () / 12;
   let mut pmin = [f32::MAX; 3];
   let mut pmax = [f32::MIN; 3];

   for i in 0..nvert
   {
      let x = f32::from_le_bytes (pos[i * 12..i * 12 + 4].try_into ().unwrap ());
      let y = f32::from_le_bytes (pos[i * 12 + 4..i * 12 + 8].try_into ().unwrap ());
      let z = f32::from_le_bytes (pos[i * 12 + 8..i * 12 + 12].try_into ().unwrap ());
      pmin[0] = pmin[0].min (x);
      pmin[1] = pmin[1].min (y);
      pmin[2] = pmin[2].min (z);
      pmax[0] = pmax[0].max (x);
      pmax[1] = pmax[1].max (y);
      pmax[2] = pmax[2].max (z);
   }

   let mut bin = Vec::new ();
   let o_pos = align (&mut bin);
   bin.extend_from_slice (&pos);
   let o_idx = align (&mut bin);
   bin.extend_from_slice (&idx);
   align (&mut bin);

   let json = format! (
      "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"mesh\":0}}],\
       \"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1,\"material\":0}}]}}],\
       \"materials\":[{{\"doubleSided\":true,\"pbrMetallicRoughness\":{{\"baseColorFactor\":[0.08,0.09,0.1,1],\"metallicFactor\":0,\"roughnessFactor\":1}}}}],\
       \"buffers\":[{{\"byteLength\":{blen}}}],\
       \"bufferViews\":[{{\"buffer\":0,\"byteOffset\":{o_pos},\"byteLength\":{n_pos},\"target\":34962}},\
       {{\"buffer\":0,\"byteOffset\":{o_idx},\"byteLength\":{n_idx},\"target\":34963}}],\
       \"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":{nvert},\"type\":\"VEC3\",\"min\":[{min0:.5},{min1:.5},{min2:.5}],\"max\":[{max0:.5},{max1:.5},{max2:.5}]}},
       {{\"bufferView\":1,\"componentType\":5125,\"count\":{nidx},\"type\":\"SCALAR\"}}]}}",
      blen = bin.len (),
      o_pos = o_pos, n_pos = pos.len (),
      o_idx = o_idx, n_idx = idx.len (),
      nvert = nvert,
      nidx = idx.len () / 4,
      min0 = pmin[0], min1 = pmin[1], min2 = pmin[2],
      max0 = pmax[0], max1 = pmax[1], max2 = pmax[2],
   );

   pack_glb (&json, &bin)
}

fn ribbon (pos: &mut Vec<u8>, idx: &mut Vec<u8>, e0: f32, n0: f32, u0: f32, e1: f32, n1: f32, u1: f32)
{
   let dx = e1 - e0;
   let dz = -(n1 - n0);
   let len = (dx * dx + dz * dz).sqrt ();

   if len < 1.0e-3
   {
      return;
   }

   let px = -dz / len * WIRE_HALF_M;
   let pz = dx / len * WIRE_HALF_M;
   let base = pos.len () as u32 / 12;
   let y0 = u0 + WIRE_LIFT_M;
   let y1 = u1 + WIRE_LIFT_M;

   push_f32 (pos, e0 + px);
   push_f32 (pos, y0);
   push_f32 (pos, -n0 + pz);
   push_f32 (pos, e0 - px);
   push_f32 (pos, y0);
   push_f32 (pos, -n0 - pz);
   push_f32 (pos, e1 + px);
   push_f32 (pos, y1);
   push_f32 (pos, -n1 + pz);
   push_f32 (pos, e1 - px);
   push_f32 (pos, y1);
   push_f32 (pos, -n1 - pz);

   for v in [0, 1, 2, 0, 2, 3]
   {
      push_u32 (idx, base + v);
   }
}

fn gltf_pos (east: f32, north: f32, up: f32) -> (f32, f32, f32)
{
   (east, up, -north)
}

fn normal_at (east: &[f32], north: &[f32], up: &[f32], nx: i32, ny: i32, c: i32, r: i32) -> (f32, f32, f32)
{
   let at = |c: i32, r: i32| -> (f32, f32, f32)
   {
      let i = (r * nx + c) as usize;
      (east[i], north[i], up[i])
   };
   let (e1, n1, u1) = at ((c + 1).min (nx - 1), r);
   let (e0, n0, u0) = at ((c - 1).max (0), r);
   let (es, ns, us) = at (c, (r + 1).min (ny - 1));
   let (en, nn, un) = at (c, (r - 1).max (0));
   // tE runs toward +east. tN runs toward +north (decreasing row).
   let tex = e1 - e0;
   let tey = n1 - n0;
   let tez = u1 - u0;
   let tnx = en - es;
   let tny = nn - ns;
   let tnz = un - us;
   let cx = tey * tnz - tez * tny;
   let cy = tez * tnx - tex * tnz;
   let cz = tex * tny - tey * tnx;
   let len = (cx * cx + cy * cy + cz * cz).sqrt ();

   if len < 1.0e-6
   {
      (0.0, 0.0, 1.0)
   }
   else
   {
      (cx / len, cy / len, cz / len)
   }
}

fn align (bin: &mut Vec<u8>) -> usize
{
   while bin.len () % 4 != 0
   {
      bin.push (0);
   }

   bin.len ()
}

fn push_f32 (bin: &mut Vec<u8>, v: f32)
{
   bin.extend_from_slice (&v.to_le_bytes ());
}

fn push_u16 (bin: &mut Vec<u8>, v: u32)
{
   bin.extend_from_slice (&(v as u16).to_le_bytes ());
}

fn push_u32 (bin: &mut Vec<u8>, v: u32)
{
   bin.extend_from_slice (&v.to_le_bytes ());
}

fn pack_glb (json: &str, bin: &[u8]) -> Vec<u8>
{
   let mut js = json.as_bytes ().to_vec ();

   while js.len () % 4 != 0
   {
      js.push (b' ');
   }

   let total = 12 + 8 + js.len () + 8 + bin.len ();
   let mut out = Vec::with_capacity (total);
   out.extend_from_slice (&0x46546C67u32.to_le_bytes ());
   out.extend_from_slice (&2u32.to_le_bytes ());
   out.extend_from_slice (&(total as u32).to_le_bytes ());
   out.extend_from_slice (&(js.len () as u32).to_le_bytes ());
   out.extend_from_slice (&0x4E4F534Au32.to_le_bytes ());
   out.extend_from_slice (&js);
   out.extend_from_slice (&(bin.len () as u32).to_le_bytes ());
   out.extend_from_slice (&0x004E4942u32.to_le_bytes ());
   out.extend_from_slice (bin);
   out
}

#[cfg(test)]
mod tests
{
   use super::*;
   use crate::geom::Window;

   fn tile () -> Window
   {
      Window { lat: 32.78, lon: -96.80, width_m: 480.0, length_m: 480.0, px_w: 256, px_h: 256 }
   }

   fn cell (min_patch: Option<f64>) -> f64
   {
      let (_, _, cell_e, cell_n) = grid (&tile (), min_patch_metres (min_patch));
      cell_e.min (cell_n)
   }

   #[test]
   fn missing_patch_stays_five_metres ()
   {
      assert! ((min_patch_metres (None) - 5.0).abs () < 1e-9);
      assert! ((min_patch_metres (Some (f64::NAN)) - 5.0).abs () < 1e-9);
      assert! ((min_patch_metres (Some (0.0)) - 5.0).abs () < 1e-9);
      assert! ((min_patch_metres (Some (-4.0)) - 5.0).abs () < 1e-9);
      assert! ((cell (None) - 5.0).abs () < 1e-9);
   }

   #[test]
   fn larger_min_patch_coarsens_grid ()
   {
      let (cols, rows, cell_e, cell_n) = grid (&tile (), min_patch_metres (Some (20.0)));
      assert_eq! (cols, 24);
      assert_eq! (rows, 24);
      assert! ((cell_e - 20.0).abs () < 1e-9);
      assert! ((cell_n - 20.0).abs () < 1e-9);
      assert! (cell (Some (20.0)) > cell (None));
   }
}
