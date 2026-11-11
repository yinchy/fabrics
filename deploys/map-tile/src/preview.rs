use crate::dem::{self, DemSet};
use crate::draw;
use crate::geom::{self, Window};
use crate::tin;

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
   let started = std::time::Instant::now ();
   let built = draw::build (&window, &refs, &dem, tin::min_patch_metres (None), tin::max_error_metres (None));
   let took = started.elapsed ();

   eprintln! (
      "footprints {} ({} tris) terrain {} tris, ground {} tris, relief {:.1}..{:.1} finest {:.2} m top {:.1} | ground {} bldg {} wire {} bytes | {:?}",
      built.footprints, built.building_tris, built.terrain_tris, built.ground_tris, built.elev_min, built.elev_max,
      built.cell_m, built.top, built.ground.len (), built.buildings.len (), built.wire.len (), took
   );

   assert! (built.footprints > 5, "expected downtown footprints");
   assert! (built.elev_max > built.elev_min);
   assert! (built.elev_min > 50.0 && built.elev_max < 400.0);
   assert! (built.cell_m + 0.05 >= 5.0, "cell {}", built.cell_m);
   assert! (built.terrain_tris >= 2 && built.terrain_tris < 8192, "terrain {}", built.terrain_tris);
   assert! (built.ground_tris > built.terrain_tris, "overlays should add triangles");
   assert! (built.ground.starts_with (b"glTF"), "ground glb");
   assert! (built.buildings.starts_with (b"glTF"), "buildings glb");
   assert! (built.wire.starts_with (b"glTF"), "wire glb");

   let dir = std::env::temp_dir ();
   std::fs::write (dir.join ("map-tile-ground.glb"), &built.ground).unwrap ();
   std::fs::write (dir.join ("map-tile-buildings.glb"), &built.buildings).unwrap ();
   std::fs::write (dir.join ("map-tile-wire.glb"), &built.wire).unwrap ();
   eprintln! ("wrote glbs to {}", dir.display ());
}
