// A Mapbox vector tile, just enough of the protobuf to read polygons and lines.
// OpenFreeMap's `building` layer carries render_height and render_min_height,
// and sets hide_3d on a footprint that is already drawn as parts.

use crate::geom::{tile_to_local, Window};

#[derive(Clone)]
pub enum Kind
{
   Building { height: f64, min_h: f64, color: Option<u32> },
   Landuse (u32),
   Landcover (u32),
   Water,
   Road { width_px: i32, color: u32 },
}

pub struct Shape
{
   pub kind: Kind,
   pub rings: Vec<Vec<(f64, f64)>>,
}

struct Cursor<'a>
{
   b: &'a [u8],
   i: usize,
}

impl<'a> Cursor<'a>
{
   fn new (b: &'a [u8]) -> Self { Self { b, i: 0 } }

   fn varint (&mut self) -> Option<u64>
   {
      let mut n = 0u64;
      let mut s = 0;

      while self.i < self.b.len () && s <= 63
      {
         let c = self.b[self.i];
         self.i += 1;
         n |= u64::from (c & 0x7f) << s;

         if c < 128
         {
            return Some (n);
         }

         s += 7;
      }

      None
   }

   fn take (&mut self, n: usize) -> Option<&'a [u8]>
   {
      if self.i + n > self.b.len ()
      {
         return None;
      }

      let s = &self.b[self.i..self.i + n];
      self.i += n;
      Some (s)
   }

   fn skip (&mut self, wire: u32) -> Option<()>
   {
      match wire
      {
         0 => { self.varint ()?; }
         1 => { self.take (8)?; }
         2 => { let n = self.varint ()? as usize; self.take (n)?; }
         5 => { self.take (4)?; }
         _ => return None,
      }

      Some (())
   }
}

fn zigzag (n: u32) -> i32
{
   ((n >> 1) as i32) ^ -((n & 1) as i32)
}

fn packed_u32 (bytes: &[u8]) -> Option<Vec<u32>>
{
   let mut c = Cursor::new (bytes);
   let mut out = Vec::new ();

   while c.i < c.b.len ()
   {
      let n = c.varint ()?;
      out.push (n as u32);
   }

   Some (out)
}

#[derive(Clone)]
enum Value
{
   Str (String),
   Num (f64),
   Bool (bool),
}

fn parse_value (bytes: &[u8]) -> Option<Value>
{
   let mut c = Cursor::new (bytes);
   let mut value = None;

   while c.i < c.b.len ()
   {
      let key = c.varint ()?;
      let field = (key >> 3) as u32;
      let wire = (key & 7) as u32;

      value = Some (match (field, wire)
      {
         (1, 2) =>
         {
            let n = c.varint ()? as usize;
            let s = c.take (n)?;
            Value::Str (String::from_utf8_lossy (s).into_owned ())
         }
         (2, 5) =>
         {
            let b = c.take (4)?;
            Value::Num (f64::from (f32::from_le_bytes ([b[0], b[1], b[2], b[3]])))
         }
         (3, 1) =>
         {
            let b = c.take (8)?;
            let mut a = [0u8; 8];
            a.copy_from_slice (b);
            Value::Num (f64::from_le_bytes (a))
         }
         (4, 0) | (5, 0) => Value::Num (c.varint ()? as f64),
         (6, 0) =>
         {
            let n = c.varint ()?;
            Value::Num (f64::from (zigzag (n as u32)))
         }
         (7, 0) => Value::Bool (c.varint ()? != 0),
         _ =>
         {
            c.skip (wire)?;
            continue;
         }
      });
   }

   value
}

fn decode_rings (words: &[u32]) -> Vec<Vec<(i32, i32)>>
{
   let mut rings = Vec::new ();
   let mut ring: Vec<(i32, i32)> = Vec::new ();
   let mut x = 0i32;
   let mut y = 0i32;
   let mut i = 0;

   while i < words.len ()
   {
      let cmd = words[i] & 7;
      let count = (words[i] >> 3) as usize;
      i += 1;

      if cmd == 1 || cmd == 2
      {
         for _ in 0..count
         {
            if i + 1 >= words.len ()
            {
               return rings;
            }

            x += zigzag (words[i]);
            y += zigzag (words[i + 1]);
            i += 2;

            if cmd == 1 && !ring.is_empty ()
            {
               if ring.len () >= 2
               {
                  rings.push (std::mem::take (&mut ring));
               }
               else
               {
                  ring.clear ();
               }
            }

            ring.push ((x, y));

            if ring.len () > 8_000
            {
               return rings;
            }
         }
      }
      else if cmd == 7
      {
         if ring.len () >= 3
         {
            rings.push (std::mem::take (&mut ring));
         }
         else
         {
            ring.clear ();
         }
      }
      else
      {
         break;
      }
   }

   if ring.len () >= 2
   {
      rings.push (ring);
   }

   rings
}

fn hex_color (text: &str) -> Option<u32>
{
   let s = text.trim ();
   let s = s.strip_prefix ('#').unwrap_or (s);
   let n = u32::from_str_radix (s, 16).ok ()?;

   if s.len () == 6
   {
      Some (n)
   }
   else if s.len () == 3
   {
      let r = (n >> 8) & 0xf;
      let g = (n >> 4) & 0xf;
      let b = n & 0xf;
      Some ((r << 20) | (r << 16) | (g << 12) | (g << 8) | (b << 4) | b)
   }
   else
   {
      None
   }
}

fn land_color (class: &str, cover: bool) -> u32
{
   match class
   {
      "wood" | "forest" | "scrub" => 0x3f6b45,
      "grass" | "park" | "garden" | "pitch" | "meadow" | "recreation_ground" => 0x6e9a5a,
      "farmland" | "farm" | "orchard" | "vineyard" | "allotments" => 0xa3a05a,
      "sand" | "beach" | "dune" => 0xd9c99a,
      "wetland" | "marsh" | "mud" => 0x6d7d62,
      "ice" | "glacier" | "snow" => 0xe7eef2,
      "rock" | "bare" | "cliff" | "quarry" => 0x9a9288,
      "residential" => 0xc4beae,
      "commercial" | "retail" => 0xc8c2b6,
      "industrial" | "railway" | "brownfield" => 0xb3aea4,
      "cemetery" | "grave_yard" => 0x7f9474,
      _ if cover => 0x7d8f68,
      _ => 0xc2bbaa,
   }
}

fn road_style (class: &str) -> Option<(i32, u32)>
{
   match class
   {
      "motorway" | "trunk" => Some ((4, 0xe2c36a)),
      "primary" => Some ((3, 0xf0e0a0)),
      "secondary" | "tertiary" => Some ((2, 0xf4f0e4)),
      "minor" | "residential" | "unclassified" | "service" | "living_street" | "busway" | "raceway" => Some ((2, 0xded8cc)),
      "rail" | "transit" | "light_rail" => Some ((2, 0x8a8480)),
      "path" | "track" | "pedestrian" | "footway" | "steps" | "ferry" | "aerialway" | "corridor" | "platform" => None,
      _ => Some ((1, 0xd4cec2)),
   }
}

fn kind_of (layer: &str, props: &[(String, Value)]) -> Option<Kind>
{
   let get = |name: &str| props.iter ().find (|(k, _)| k == name).map (|(_, v)| v);

   match layer
   {
      "building" =>
      {
         if let Some (Value::Bool (true)) = get ("hide_3d")
         {
            return None;
         }

         if let Some (Value::Num (n)) = get ("hide_3d")
         {
            if *n != 0.0
            {
               return None;
            }
         }

         let height = match get ("render_height")
         {
            Some (Value::Num (n)) if *n > 0.3 => *n,
            _ => 6.0,
         };
         let min_h = match get ("render_min_height")
         {
            Some (Value::Num (n)) if *n > 0.0 && *n < height => *n,
            _ => 0.0,
         };
         let color = match get ("colour")
         {
            Some (Value::Str (s)) => hex_color (s),
            _ => None,
         };

         Some (Kind::Building { height, min_h, color })
      }
      "landuse" =>
      {
         let class = match get ("class") { Some (Value::Str (s)) => s.as_str (), _ => "" };
         Some (Kind::Landuse (land_color (class, false)))
      }
      "landcover" | "park" =>
      {
         let class = match get ("class") { Some (Value::Str (s)) => s.as_str (), _ => "" };
         Some (Kind::Landcover (land_color (class, true)))
      }
      "water" => Some (Kind::Water),
      "transportation" =>
      {
         let class = match get ("class") { Some (Value::Str (s)) => s.as_str (), _ => "" };
         let (width_px, color) = road_style (class)?;
         Some (Kind::Road { width_px, color })
      }
      _ => None,
   }
}

pub fn decode (window: &Window, z: u32, tx: u32, ty: u32, bytes: &[u8]) -> Vec<Shape>
{
   let bytes = inflate (bytes);
   let mut shapes = Vec::new ();
   let mut tile = Cursor::new (bytes.as_ref ());

   while tile.i < tile.b.len ()
   {
      let Some (key) = tile.varint () else { break };
      let field = (key >> 3) as u32;
      let wire = (key & 7) as u32;

      if field == 3 && wire == 2
      {
         let Some (n) = tile.varint () else { break };
         let Some (layer) = tile.take (n as usize) else { break };
         decode_layer (window, z, tx, ty, layer, &mut shapes);
      }
      else if tile.skip (wire).is_none ()
      {
         break;
      }
   }

   shapes
}

fn inflate (bytes: &[u8]) -> std::borrow::Cow<'_, [u8]>
{
   if bytes.len () >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
   {
      let mut dec = flate2::read::GzDecoder::new (bytes);
      let mut out = Vec::new ();

      if std::io::Read::read_to_end (&mut dec, &mut out).is_ok ()
      {
         return std::borrow::Cow::Owned (out);
      }
   }

   std::borrow::Cow::Borrowed (bytes)
}

fn decode_layer (window: &Window, z: u32, tx: u32, ty: u32, layer: &[u8], shapes: &mut Vec<Shape>)
{
   let mut name = String::new ();
   let mut extent = 4096.0f64;
   let mut keys: Vec<String> = Vec::new ();
   let mut values: Vec<Value> = Vec::new ();
   let mut features: Vec<Vec<u8>> = Vec::new ();
   let mut c = Cursor::new (layer);

   while c.i < c.b.len ()
   {
      let Some (key) = c.varint () else { return };
      let field = (key >> 3) as u32;
      let wire = (key & 7) as u32;

      match (field, wire)
      {
         (1, 2) =>
         {
            let Some (n) = c.varint () else { return };
            let Some (s) = c.take (n as usize) else { return };
            name = String::from_utf8_lossy (s).into_owned ();
         }
         (5, 0) =>
         {
            let Some (n) = c.varint () else { return };
            if n > 0 { extent = n as f64; }
         }
         (3, 2) =>
         {
            let Some (n) = c.varint () else { return };
            let Some (s) = c.take (n as usize) else { return };
            keys.push (String::from_utf8_lossy (s).into_owned ());
         }
         (4, 2) =>
         {
            let Some (n) = c.varint () else { return };
            let Some (s) = c.take (n as usize) else { return };
            if let Some (v) = parse_value (s) { values.push (v); }
            else { values.push (Value::Num (0.0)); }
         }
         (2, 2) =>
         {
            let Some (n) = c.varint () else { return };
            let Some (s) = c.take (n as usize) else { return };
            let keep = name.is_empty ()
               || name == "building" || name == "landuse" || name == "landcover"
               || name == "park" || name == "water" || name == "transportation";

            if keep
            {
               features.push (s.to_vec ());
            }
         }
         _ => { if c.skip (wire).is_none () { return; } }
      }
   }

   if name != "building" && name != "landuse" && name != "landcover" && name != "park" && name != "water" && name != "transportation"
   {
      return;
   }

   for feature in features
   {
      if let Some (shape) = decode_feature (window, z, tx, ty, extent, &name, &keys, &values, &feature)
      {
         shapes.push (shape);
      }
   }
}

fn decode_feature (window: &Window, z: u32, tx: u32, ty: u32, extent: f64, layer: &str, keys: &[String], values: &[Value], feature: &[u8]) -> Option<Shape>
{
   let mut tags: Vec<u32> = Vec::new ();
   let mut geom: Vec<u32> = Vec::new ();
   let mut gtype = 0u32;
   let mut c = Cursor::new (feature);

   while c.i < c.b.len ()
   {
      let key = c.varint ()?;
      let field = (key >> 3) as u32;
      let wire = (key & 7) as u32;

      match (field, wire)
      {
         (2, 2) =>
         {
            let n = c.varint ()? as usize;
            tags = packed_u32 (c.take (n)?)?;
         }
         (3, 0) => gtype = c.varint ()? as u32,
         (4, 2) =>
         {
            let n = c.varint ()? as usize;
            geom = packed_u32 (c.take (n)?)?;
         }
         _ => c.skip (wire)?,
      }
   }

   if gtype != 2 && gtype != 3
   {
      return None;
   }

   let mut props = Vec::new ();
   let mut i = 0;

   while i + 1 < tags.len ()
   {
      let k = tags[i] as usize;
      let v = tags[i + 1] as usize;
      i += 2;

      if let (Some (key), Some (val)) = (keys.get (k), values.get (v))
      {
         props.push ((key.clone (), val.clone ()));
      }
   }

   let kind = kind_of (layer, &props)?;
   let raw = decode_rings (&geom);

   if raw.is_empty ()
   {
      return None;
   }

   let mut rings = Vec::new ();

   for ring in raw
   {
      let mut pts = Vec::with_capacity (ring.len ());

      for (px, py) in ring
      {
         pts.push (tile_to_local (window, z, tx, ty, extent, f64::from (px), f64::from (py)));
      }

      rings.push (pts);
   }

   Some (Shape { kind, rings })
}
