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
fn lax_two_km_tile_builds ()
{
   let (built, nv, nd) = build_at (33.942791, -118.410042, 2000.0, 2000.0);

   eprintln! ("LAX 2 km: {nv} vector tiles, {nd} elevation tiles, {} footprints, terrain {} tris, ground {} tris, buildings {} tris, ground {} B, buildings {} B, wire {} B, relief {:.1}..{:.1}, finest {:.2} m",
      built.footprints, built.terrain_tris, built.ground_tris, built.building_tris,
      built.ground.len (), built.buildings.len (), built.wire.len (), built.elev_min, built.elev_max, built.cell_m);

   assert! (built.ground.starts_with (b"glTF"));
}

#[test]
fn lax_wide_window_stacks_without_hitting_the_limit ()
{
   let (built, nv, nd) = build_at (33.942791, -118.410042, 3000.0, 7000.0);

   eprintln! ("LAX 3x7 km: {nv} vector tiles, {nd} elevation tiles, {} footprints, relief {:.1}..{:.1}, {} overlaps, {} levels of {:.3} m, {} capped, {} roof overlaps",
      built.footprints, built.elev_min, built.elev_max, built.overlaps, built.levels, built.gap_m, built.capped, built.roof_overlaps);

   assert! (built.gap_m > 0.25, "gap {}", built.gap_m);
   assert! (built.capped == 0, "{} patches held at the level limit", built.capped);
   assert! (built.levels < crate::zorder::MAX_LEVEL, "levels {}", built.levels);
}

fn build_at (lat: f64, lon: f64, width: f64, length: f64) -> (draw::Built, usize, usize)
{
   let window = Window::new (lat, lon, width, length, 256);
   let (west, south, east, north) = window.bounds ();
   let vectors = geom::tiles_covering (west, south, east, north, geom::VECTOR_Z);
   let dem_z = geom::dem_zoom (geom::tiles_covering (west, south, east, north, 15).len ());
   let dems = geom::tiles_covering (west, south, east, north, dem_z);

   assert! (vectors.len () <= 16, "vector tiles {}", vectors.len ());
   assert! (dems.len () <= 16, "dem tiles {}", dems.len ());

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
   let built = draw::build (&window, &refs, &dem, tin::min_patch_metres (None), tin::max_error_metres (None), None);
   eprintln! ("z-fight guard: {} overlaps, {} levels of {:.3} m, {} capped, {} roof overlaps",
      built.overlaps, built.levels, built.gap_m, built.capped, built.roof_overlaps);
   eprintln! ("build took {:?}", started.elapsed ());

   (built, vectors.len (), dems.len ())
}

#[test]
fn dallas_tile_has_buildings_and_relief ()
{
   let (built, _, _) = build_at (32.778038, -96.800537, 480.0, 480.0);

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
