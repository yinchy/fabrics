// Mesh builder and binary glTF writer. One primitive per material colour, so a
// whole layer is one draw. Vertices are held as local (east, north, up) and
// stored in glTF Y-up as (east, up, -north); the engine's Rx(+90) maps that
// back to east/north/up. (e, n, u) -> (e, u, -n) is a proper rotation, so
// winding is preserved.

pub struct Prim
{
   pub rgb: [u8; 3],
   pos: Vec<[f32; 3]>,
   nrm: Vec<[f32; 3]>,
   idx: Vec<u32>,
}

impl Prim
{
   pub fn vert (&mut self, p: [f64; 3], n: [f64; 3]) -> u32
   {
      self.pos.push ([p[0] as f32, p[1] as f32, p[2] as f32]);
      self.nrm.push ([n[0] as f32, n[1] as f32, n[2] as f32]);
      (self.pos.len () - 1) as u32
   }

   pub fn tri (&mut self, a: u32, b: u32, c: u32)
   {
      self.idx.extend_from_slice (&[a, b, c]);
   }

   pub fn count (&self) -> u32 { self.pos.len () as u32 }
}

#[derive(Default)]
pub struct Mesh
{
   prims: Vec<Prim>,
}

impl Mesh
{
   pub fn prim (&mut self, rgb: [u8; 3]) -> &mut Prim
   {
      if let Some (i) = self.prims.iter ().position (|p| p.rgb == rgb)
      {
         return &mut self.prims[i];
      }

      self.prims.push (Prim { rgb, pos: Vec::new (), nrm: Vec::new (), idx: Vec::new () });
      self.prims.last_mut ().unwrap ()
   }

   pub fn triangles (&self) -> usize
   {
      self.prims.iter ().map (|p| p.idx.len () / 3).sum ()
   }

   #[cfg(test)]
   pub fn flat_area (&self) -> f64
   {
      let mut sum = 0.0;

      for p in &self.prims
      {
         for t in p.idx.chunks_exact (3)
         {
            let q = |i: u32| (f64::from (p.pos[i as usize][0]), f64::from (p.pos[i as usize][1]));
            sum += crate::tess::tri_area (&[q (t[0]), q (t[1]), q (t[2])]).abs ();
         }
      }

      sum
   }

   pub fn pack (&self) -> Vec<u8>
   {
      let prims: Vec<&Prim> = self.prims.iter ().filter (|p| !p.idx.is_empty ()).collect ();

      if prims.is_empty ()
      {
         return Vec::new ();
      }

      let mut bin: Vec<u8> = Vec::new ();
      let mut views: Vec<String> = Vec::new ();
      let mut accessors: Vec<String> = Vec::new ();
      let mut primitives: Vec<String> = Vec::new ();
      let mut materials: Vec<String> = Vec::new ();

      for (k, p) in prims.iter ().enumerate ()
      {
         let mut pmin = [f32::MAX; 3];
         let mut pmax = [f32::MIN; 3];
         let o_pos = align (&mut bin);

         for v in &p.pos
         {
            let g = [v[0], v[2], -v[1]];

            for a in 0..3
            {
               pmin[a] = pmin[a].min (g[a]);
               pmax[a] = pmax[a].max (g[a]);
               bin.extend_from_slice (&g[a].to_le_bytes ());
            }
         }

         let n_pos = bin.len () - o_pos;
         let o_nrm = align (&mut bin);

         for v in &p.nrm
         {
            for g in [v[0], v[2], -v[1]]
            {
               bin.extend_from_slice (&g.to_le_bytes ());
            }
         }

         let n_nrm = bin.len () - o_nrm;
         let o_idx = align (&mut bin);

         for i in &p.idx
         {
            bin.extend_from_slice (&i.to_le_bytes ());
         }

         let n_idx = bin.len () - o_idx;
         let v0 = views.len ();

         views.push (format! ("{{\"buffer\":0,\"byteOffset\":{o_pos},\"byteLength\":{n_pos},\"target\":34962}}"));
         views.push (format! ("{{\"buffer\":0,\"byteOffset\":{o_nrm},\"byteLength\":{n_nrm},\"target\":34962}}"));
         views.push (format! ("{{\"buffer\":0,\"byteOffset\":{o_idx},\"byteLength\":{n_idx},\"target\":34963}}"));

         let a0 = accessors.len ();

         accessors.push (format! (
            "{{\"bufferView\":{v0},\"componentType\":5126,\"count\":{},\"type\":\"VEC3\",\"min\":[{:.5},{:.5},{:.5}],\"max\":[{:.5},{:.5},{:.5}]}}",
            p.pos.len (), pmin[0], pmin[1], pmin[2], pmax[0], pmax[1], pmax[2]
         ));
         accessors.push (format! ("{{\"bufferView\":{},\"componentType\":5126,\"count\":{},\"type\":\"VEC3\"}}", v0 + 1, p.nrm.len ()));
         accessors.push (format! ("{{\"bufferView\":{},\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}", v0 + 2, p.idx.len ()));

         primitives.push (format! (
            "{{\"attributes\":{{\"POSITION\":{a0},\"NORMAL\":{}}},\"indices\":{},\"material\":{k}}}",
            a0 + 1, a0 + 2
         ));

         let [r, g, b] = [linear (p.rgb[0]), linear (p.rgb[1]), linear (p.rgb[2])];

         materials.push (format! (
            "{{\"doubleSided\":true,\"pbrMetallicRoughness\":{{\"baseColorFactor\":[{r:.4},{g:.4},{b:.4},1],\"metallicFactor\":0,\"roughnessFactor\":0.9}}}}"
         ));
      }

      align (&mut bin);

      let json = format! (
         "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"mesh\":0}}],\
          \"meshes\":[{{\"primitives\":[{}]}}],\"materials\":[{}],\"buffers\":[{{\"byteLength\":{}}}],\
          \"bufferViews\":[{}],\"accessors\":[{}]}}",
         primitives.join (","), materials.join (","), bin.len (), views.join (","), accessors.join (",")
      );

      pack_glb (&json, &bin)
   }
}

pub fn rgb (c: u32) -> [u8; 3]
{
   [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

fn linear (c: u8) -> f64
{
   let s = f64::from (c) / 255.0;

   if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf (2.4) }
}

fn align (bin: &mut Vec<u8>) -> usize
{
   while bin.len () % 4 != 0
   {
      bin.push (0);
   }

   bin.len ()
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
