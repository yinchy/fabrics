use crate::dem::{self, DemSet};
use crate::draw;
use crate::geom::{self, Window};

fn fetch (url: &str) -> Vec<u8>
{
   let path = std::env::temp_dir ().join ("map-tile-fetch.bin");
   let status = std::process::Command::new ("curl.exe")
      .args (["-fsS", "-L", "--compressed", "-m", "40", "-A", "fabrics-map-tile/0.1", "-o"])
      .arg (&path)
      .arg (url)
      .status ()
      .expect ("curl");

   assert! (status.success (), "fetch failed {url}");
   std::fs::read (&path).expect ("read")
}

#[test]
fn dallas_tile_has_buildings_and_relief ()
{
   let window = Window::new (32.778038, -96.800537, 480.0, 480.0, 256);
   let (west, south, east, north) = window.bounds ();
   let vectors = geom::tiles_covering (west, south, east, north, geom::VECTOR_Z);
   let dem_z = geom::dem_zoom (geom::tiles_covering (west, south, east, north, 15).len ());
   let dems = geom::tiles_covering (west, south, east, north, dem_z);

   assert! (vectors.len () <= 4, "vector tiles {}", vectors.len ());
   assert! (dems.len () <= 4, "dem tiles {}", dems.len ());

   let mut vector_bytes = Vec::new ();

   for (z, x, y) in &vectors
   {
      let url = format! ("https://tiles.openfreemap.org/planet/latest/{z}/{x}/{y}.pbf");
      vector_bytes.push ((*z, *x, *y, fetch (&url)));
   }

   let mut grids = Vec::new ();

   for (z, x, y) in &dems
   {
      let url = format! ("https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png");
      let bytes = fetch (&url);
      let grid = dem::decode (*z, *x, *y, &bytes).expect ("elevation png");
      grids.push (grid);
   }

   let refs: Vec<(u32, u32, u32, &[u8])> = vector_bytes.iter ().map (|(z, x, y, b)| (*z, *x, *y, b.as_slice ())).collect ();
   let dem = DemSet::new (grids);
   let built = draw::build (&window, &refs, &dem, crate::mesh::min_patch_metres (None));

   eprintln! (
      "buildings {} prisms {} relief {:.1}..{:.1} cell {:.2} m solid {} wire {} image {}x{}",
      built.buildings, built.prisms.len (), built.elev_min, built.elev_max, built.cell_m,
      built.solid.len (), built.wire.len (), built.width, built.height
   );

   assert! (built.buildings > 5, "expected downtown footprints");
   assert! (!built.prisms.is_empty ());
   assert! (built.elev_max > built.elev_min);
   assert! (built.elev_min > 50.0 && built.elev_max < 400.0);
   assert! (built.cell_m + 0.05 >= 5.0, "cell {}", built.cell_m);
   assert! (built.solid.starts_with (b"glTF"), "solid glb");
   assert! (built.wire.starts_with (b"glTF"), "wire glb");

   let path = std::env::temp_dir ().join ("map-tile-preview.png");
   let file = std::fs::File::create (&path).unwrap ();
   let mut encoder = png::Encoder::new (file, built.width as u32, built.height as u32);
   encoder.set_color (png::ColorType::Rgba);
   encoder.set_depth (png::BitDepth::Eight);
   encoder.write_header ().unwrap ().write_image_data (&built.rgba).unwrap ();
   eprintln! ("wrote {}", path.display ());
}
