#![allow(non_snake_case)]

use sneeze::*;

use crate::dem::{self, DemSet};
use crate::draw::{self, Prism};
use crate::geom::{self, Window};

const TICK_MS: i32 = 16;
const MAX_TILES: usize = 16;
const SPAWN_PER_TICK: usize = 160;

const DEFAULT_LAT: f64 = 32.778038;
const DEFAULT_LON: f64 = -96.800537;
const DEFAULT_WIDTH: f64 = 480.0;
const DEFAULT_LENGTH: f64 = 480.0;
const DEFAULT_PIXELS: i32 = 256;

const TYPE_NODE: u16 = 8;
const METHOD_MODEL: u16 = 9;
const TYPE_VIEWPORT: u16 = 5;
const METHOD_KEY: u16 = 60;

#[link(wasm_import_module = "Sneeze")]
extern "C"
{
   fn Call (nOffset: i32, nSize: i32) -> i64;
}

struct SLOT
{
   id: u64,
   z: u32,
   x: u32,
   y: u32,
   body: Option<Vec<u8>>,
   done: bool,
}

struct JOB
{
   fabric: u64,
   root: NODE,
   window: Window,
   vector: Vec<SLOT>,
   dem: Vec<SLOT>,
   ready: bool,
   built: bool,
   prisms: Vec<Prism>,
   cursor: usize,
   solid: Option<NODE>,
   wire: Option<NODE>,
   wire_on: bool,
   toggle: u64,
}

struct MAP;

static mut JOBS: Vec<JOB> = Vec::new ();

fn Jobs () -> &'static mut Vec<JOB>
{
   unsafe { &mut *core::ptr::addr_of_mut! (JOBS) }
}

fn Number (sText: &str, nDefault: f64) -> f64
{
   let sText = sText.trim ().trim_matches ('"');
   sText.parse::<f64> ().unwrap_or (nDefault)
}

fn Whole (sText: &str, nDefault: i32) -> i32
{
   let sText = sText.trim ().trim_matches ('"');

   if let Ok (n) = sText.parse::<i32> ()
   {
      n
   }
   else if let Ok (d) = sText.parse::<f64> ()
   {
      d.round () as i32
   }
   else
   {
      nDefault
   }
}

fn Load (pHost: &HOST) -> Window
{
   let pData = pHost.Data ();
   let mut dLat = DEFAULT_LAT;
   let mut dLon = DEFAULT_LON;
   let mut dWidth = DEFAULT_WIDTH;
   let mut dLength = DEFAULT_LENGTH;
   let mut nPixels = DEFAULT_PIXELS;

   if let Some (s) = pData.Get ("Origin.dLat") { dLat = Number (&s, dLat); }
   if let Some (s) = pData.Get ("Origin.dLon") { dLon = Number (&s, dLon); }
   if let Some (s) = pData.Get ("Extent.dWidthM") { dWidth = Number (&s, dWidth); }
   if let Some (s) = pData.Get ("Extent.dLengthM") { dLength = Number (&s, dLength); }
   if let Some (s) = pData.Get ("Tile.nPixels") { nPixels = Whole (&s, nPixels); }

   Window::new (dLat, dLon, dWidth, dLength, nPixels)
}

fn MinPatch (pHost: &HOST) -> f64
{
   // Omitted Data.Grid.dMinPatchM stays at 5 m. Non-positive and non-finite too.
   let parsed = pHost.Data ().Get ("Grid.dMinPatchM").map (|s| Number (&s, f64::NAN));
   crate::mesh::min_patch_metres (parsed)
}

fn FoldKey (nCode: u64) -> u64
{
   if (b'a' as u64..=b'z' as u64).contains (&nCode) { nCode - 32 } else { nCode }
}

fn ToggleCode (pHost: &HOST) -> u64
{
   // Host codes: letters are ASCII uppercase, other bare keys are the unshifted
   // ASCII character, and digits are 0..9 (not '0'..'9'). Default is R.
   const DEFAULT: u64 = b'R' as u64;
   let Some (s) = pHost.Data ().Get ("Toggle.sKey") else { return DEFAULT };

   let s = s.trim ().trim_matches ('"').trim ();
   let mut aChar = s.chars ();
   let Some (c) = aChar.next () else { return DEFAULT };

   if aChar.next ().is_some ()
   {
      return DEFAULT;
   }

   if c.is_ascii_digit ()
   {
      return u64::from (c as u8 - b'0');
   }

   if c.is_ascii_alphabetic ()
   {
      return u64::from (c.to_ascii_uppercase () as u8);
   }

   if c.is_ascii ()
   {
      return u64::from (c as u8);
   }

   DEFAULT
}

fn KeyLabel (nCode: u64) -> String
{
   if nCode < 10
   {
      return nCode.to_string ();
   }

   match nCode
   {
      8 => "Backspace".to_string (),
      9 => "Tab".to_string (),
      13 => "Enter".to_string (),
      27 => "Escape".to_string (),
      32 => "Space".to_string (),
      n if (33..127).contains (&n) => (n as u8 as char).to_string (),
      n => n.to_string (),
   }
}

fn Issue (pHost: &HOST, sUrl: &str) -> u64
{
   let pRequest = pHost.Network ().Request_Open (eSNEEZE_ABI_REQUEST_VERB::kSNEEZE_ABI_REQUEST_VERB_GET, sUrl);

   if !pRequest.IsValid ()
   {
      return 0;
   }

   pRequest.Header_Set ("User-Agent", "fabrics-map-tile/0.1 (spatial fabric)");
   pRequest.Header_Set ("Accept", "*/*");
   pRequest.Timeout_Set (25_000);

   if !pRequest.Send ()
   {
      pRequest.Close ();
      return 0;
   }

   pRequest.Index ()
}

fn PushSlot (aSlot: &mut Vec<SLOT>, z: u32, x: u32, y: u32, id: u64)
{
   aSlot.push (SLOT { id, z, x, y, body: None, done: id == 0 });
}

fn AllDone (pJob: &JOB) -> bool
{
   pJob.vector.iter ().all (|p| p.done) && pJob.dem.iter ().all (|p| p.done)
}

fn Take (aSlot: &mut [SLOT], id: u64, aBody: Option<Vec<u8>>) -> bool
{
   if let Some (pSlot) = aSlot.iter_mut ().find (|p| p.id == id && !p.done)
   {
      pSlot.ok_body (aBody);
      return true;
   }

   false
}

impl SLOT
{
   fn ok_body (&mut self, aBody: Option<Vec<u8>>)
   {
      self.body = aBody;
      self.done = true;
   }
}

fn Attach (pNode: NODE, aByte: &[u8]) -> bool
{
   if aByte.is_empty () || !pNode.IsValid ()
   {
      return false;
   }

   let mut aPacket = Vec::with_capacity (24);
   aPacket.extend_from_slice (&TYPE_NODE.to_le_bytes ());
   aPacket.extend_from_slice (&METHOD_MODEL.to_le_bytes ());
   aPacket.extend_from_slice (&0u32.to_le_bytes ());
   aPacket.extend_from_slice (&pNode.Composed ().to_le_bytes ());
   aPacket.extend_from_slice (&(aByte.as_ptr () as u32 as i32).to_le_bytes ());
   aPacket.extend_from_slice (&(aByte.len () as i32).to_le_bytes ());

   let nSize = (aPacket.len () - 8) as u32;
   aPacket[4..8].copy_from_slice (&nSize.to_le_bytes ());

   unsafe { Call (aPacket.as_ptr () as i32, aPacket.len () as i32) != 0 }
}

fn Ground (pHost: &HOST, pRoot: NODE, sName: &str, aByte: &[u8], pWindow: &Window, dRelief: f64) -> Option<NODE>
{
   let mut pObject = SNEEZE_ABI_MAPOBJECT::Physical ();

   pObject.Name (sName)
          .Parent (pRoot.Class (), pRoot.ObjectIx ())
          .Position (0.0, 0.0, 0.0)
          .Rotation (0.0, 0.0, 0.0, 1.0)
          .Scale_Axes (1.0, 1.0, 1.0)
          .Bound (pWindow.width_m, pWindow.length_m, dRelief.max (1.0));

   let pNode = pHost.Fabric ().Node_Open (&pObject);

   if pNode.IsValid () && Attach (pNode, aByte)
   {
      Some (pNode)
   }
   else
   {
      None
   }
}

fn ShowMode (pJob: &JOB)
{
   let (dSolid, dWire) = if pJob.wire_on { (0.0, 1.0) } else { (1.0, 0.0) };

   if let Some (pNode) = pJob.solid
   {
      pNode.Scale_Axes (dSolid, dSolid, dSolid);
   }

   if let Some (pNode) = pJob.wire
   {
      pNode.Scale_Axes (dWire, dWire, dWire);
   }
}

fn Assemble (pHost: &HOST, pJob: &mut JOB)
{
   let (prisms, solid, wire, buildings, elev_min, elev_max, cell_m, nVector, nDem) = {
      let tiles: Vec<(u32, u32, u32, &[u8])> = pJob.vector.iter ().filter_map (|p| {
         let aBody = p.body.as_ref ().filter (|b| !b.is_empty ())?;
         Some ((p.z, p.x, p.y, aBody.as_slice ()))
      }).collect ();
      let nVector = tiles.len ();
      let grids: Vec<_> = pJob.dem.iter ().filter_map (|p| dem::decode (p.z, p.x, p.y, p.body.as_deref ()?)).collect ();
      let nDem = grids.len ();
      let dem = DemSet::new (grids);
      let built = draw::build (&pJob.window, &tiles, &dem, MinPatch (pHost));

      (built.prisms, built.solid, built.wire, built.buildings, built.elev_min, built.elev_max, built.cell_m, nVector, nDem)
   };

   pHost.Console ().Info (&format! (
      "map tile: {buildings} buildings, {} prisms, relief {elev_min:.0}–{elev_max:.0} m, cell {cell_m:.1} m, mesh {} / {} bytes, {nVector} vector tiles, {nDem} elevation tiles",
      prisms.len (),
      solid.len (),
      wire.len ()
   ));

   let dRelief = (elev_max - elev_min).max (1.0);
   pJob.solid = Ground (pHost, pJob.root, "Terrain", &solid, &pJob.window, dRelief);
   pJob.wire = Ground (pHost, pJob.root, "Terrain wire", &wire, &pJob.window, dRelief);
   pJob.wire_on = false;
   ShowMode (pJob);

   if pJob.solid.is_none ()
   {
      pHost.Console ().Error ("map tile: terrain mesh was not created");
   }

   pJob.prisms = prisms;

   for p in &mut pJob.vector { p.body = None; }
   for p in &mut pJob.dem { p.body = None; }

   pJob.built = true;
   pHost.Console ().Info (&format! ("map tile: press {} to switch solid and wireframe", KeyLabel (pJob.toggle)));
}

fn Spawn (pHost: &HOST, pJob: &mut JOB)
{
   let nEnd = (pJob.cursor + SPAWN_PER_TICK).min (pJob.prisms.len ());

   while pJob.cursor < nEnd
   {
      let p = pJob.prisms[pJob.cursor];
      pJob.cursor += 1;

      let mut pObject = SNEEZE_ABI_MAPOBJECT::Physical ();
      let sUrl = format! ("res/b{}.glb", p.bin);

      pObject.Name ("Building")
             .Parent (pJob.root.Class (), pJob.root.ObjectIx ())
             .Position (p.east, p.north, p.z)
             .Rotation (0.0, 0.0, p.qz, p.qw)
             .Scale_Axes (p.sx, p.sy, p.sz)
             .Bound (p.sx, p.sy, p.sz)
             .Reference (&sUrl);

      pHost.Fabric ().Node_Open (&pObject);
   }
}

fn OnKey (pHost: &HOST, nCode: u64)
{
   let Some (pJob) = Jobs ().iter_mut ().find (|p| p.fabric == pHost.Index ()) else { return };

   if FoldKey (nCode) != FoldKey (pJob.toggle)
   {
      return;
   }

   if pJob.solid.is_none () || pJob.wire.is_none ()
   {
      return;
   }

   pJob.wire_on = !pJob.wire_on;
   ShowMode (pJob);
   pHost.Console ().Info (if pJob.wire_on { "map tile: wireframe" } else { "map tile: solid" });
}

impl INSTANCE for MAP
{
   fn Open (pHost: &HOST)
   {
      let window = Load (pHost);
      let mut pRootObj = SNEEZE_ABI_MAPOBJECT::Root ();
      pRootObj.Name ("Map Tile");
      let pRoot = pHost.Fabric ().Node_Root (&pRootObj);

      if !pRoot.IsValid ()
      {
         pHost.Console ().Error ("map tile: root node was not created");
         return;
      }

      let (west, south, east, north) = window.bounds ();
      let aVector = geom::tiles_covering (west, south, east, north, geom::VECTOR_Z);
      let nDem15 = geom::tiles_covering (west, south, east, north, 15).len ();
      let nDemZ = geom::dem_zoom (nDem15);
      let aDem = geom::tiles_covering (west, south, east, north, nDemZ);

      let mut job = JOB
      {
         fabric: pHost.Index (),
         root: pRoot,
         window,
         vector: Vec::new (),
         dem: Vec::new (),
         ready: false,
         built: false,
         prisms: Vec::new (),
         cursor: 0,
         solid: None,
         wire: None,
         wire_on: false,
         toggle: ToggleCode (pHost),
      };

      if aVector.len () > MAX_TILES || aDem.len () > MAX_TILES
      {
         pHost.Console ().Error (&format! (
            "map tile: this area needs {} tiles. Shrink the width and length.",
            aVector.len ().max (aDem.len ())
         ));
      }
      else
      {
         for (z, x, y) in aVector
         {
            let sUrl = format! ("https://tiles.openfreemap.org/planet/latest/{z}/{x}/{y}.pbf");
            PushSlot (&mut job.vector, z, x, y, Issue (pHost, &sUrl));
         }

         for (z, x, y) in aDem
         {
            let sUrl = format! ("https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png");
            PushSlot (&mut job.dem, z, x, y, Issue (pHost, &sUrl));
         }

         pHost.Console ().Info (&format! (
            "map tile: {:.5},{:.5}  {:.0} m × {:.0} m  ({} vector, {} elevation)",
            window.lat, window.lon, window.width_m, window.length_m, job.vector.len (), job.dem.len ()
         ));
      }

      job.ready = AllDone (&job);
      let twFabric = job.fabric;
      Jobs ().retain (|p| p.fabric != twFabric);
      Jobs ().push (job);

      if pHost.Timer ().Interval (TICK_MS, eSNEEZE_ABI_TIMER_UNIT::kSNEEZE_ABI_TIMER_UNIT_MS, 0) == 0
      {
         pHost.Console ().Error ("map tile: timer was not armed");
      }
   }

   fn Request (pHost: &HOST, pRequest: REQUEST, bSuccess: bool)
   {
      let id = pRequest.Index ();
      let aBody = if bSuccess && pRequest.Status () == 200
      {
         let a = pRequest.Body ();
         if a.is_empty () { None } else { Some (a) }
      }
      else
      {
         None
      };

      pRequest.Close ();

      let Some (pJob) = Jobs ().iter_mut ().find (|p| p.fabric == pHost.Index ()) else { return };

      if !Take (&mut pJob.vector, id, aBody.clone ())
      {
         Take (&mut pJob.dem, id, aBody);
      }

      if AllDone (pJob)
      {
         pJob.ready = true;
      }
   }

   fn Timer (pHost: &HOST, _twTimerIx: u64, _qwParam: u64)
   {
      let Some (pJob) = Jobs ().iter_mut ().find (|p| p.fabric == pHost.Index ()) else { return };

      if pJob.ready && !pJob.built
      {
         Assemble (pHost, pJob);
      }
      else if pJob.built && pJob.cursor < pJob.prisms.len ()
      {
         Spawn (pHost, pJob);
      }
   }

   fn Close (pHost: &HOST)
   {
      let twFabric = pHost.Index ();
      Jobs ().retain (|p| p.fabric != twFabric);
   }
}

fn KeyCode (nOffset: i32, nSize: i32) -> Option<(&'static HOST, u64)>
{
   if nOffset == 0 || nSize < 24
   {
      return None;
   }

   let aByte = unsafe { core::slice::from_raw_parts (nOffset as u32 as *const u8, nSize as usize) };
   let wType = u16::from_le_bytes ([aByte[0], aByte[1]]);
   let wMethod = u16::from_le_bytes ([aByte[2], aByte[3]]);

   if wType != TYPE_VIEWPORT || wMethod != METHOD_KEY || aByte.len () < 24
   {
      return None;
   }

   let twFabric = u64::from_le_bytes (aByte[8..16].try_into ().unwrap ());
   let nCode = u64::from_le_bytes (aByte[16..24].try_into ().unwrap ());

   Host_Find (twFabric).map (|pHost| (pHost, nCode))
}

#[no_mangle]
pub extern "C" fn Init ()
{
   <MAP as INSTANCE>::Init ();
}

#[no_mangle]
pub extern "C" fn Open (twFabricIx: u64, nOffset: i32, nSize: i32)
{
   Snapshot_Load (SNAPSHOT::From_Raw (nOffset, nSize));
   <MAP as INSTANCE>::Open (Host_Acquire (twFabricIx));
}

#[no_mangle]
pub extern "C" fn Close (twFabricIx: u64)
{
   if let Some (pHost) = Host_Find (twFabricIx)
   {
      <MAP as INSTANCE>::Close (pHost);
   }

   Host_Release (twFabricIx);
}

#[no_mangle]
pub extern "C" fn Shutdown ()
{
   <MAP as INSTANCE>::Shutdown ();
}

#[no_mangle]
pub extern "C" fn Alloc (nSize: i32) -> i32
{
   sneeze::Alloc (nSize)
}

#[no_mangle]
pub extern "C" fn Free (nOffset: i32, nSize: i32)
{
   sneeze::Free (nOffset, nSize);
}

#[no_mangle]
pub extern "C" fn Notify (nOffset: i32, nSize: i32) -> i64
{
   if let Some ((pHost, nCode)) = KeyCode (nOffset, nSize)
   {
      OnKey (pHost, nCode);
      return 0;
   }

   match Event_Parse (nOffset, nSize)
   {
      EVENT::Timer { pHost, twTimerIx, qwParam } =>
         <MAP as INSTANCE>::Timer (pHost, twTimerIx, qwParam),

      EVENT::Request { pHost, pRequest, bSuccess } =>
         <MAP as INSTANCE>::Request (pHost, pRequest, bSuccess),

      EVENT::Socket_Opened { pHost, pSocket } =>
         <MAP as INSTANCE>::Socket_Opened (pHost, pSocket),

      EVENT::Socket_Received { pHost, pSocket, bBinary, nSize } =>
         <MAP as INSTANCE>::Socket_Received (pHost, pSocket, bBinary, nSize),

      EVENT::Socket_Failed { pHost, pSocket } =>
         <MAP as INSTANCE>::Socket_Failed (pHost, pSocket),

      EVENT::Socket_Closed { pHost, pSocket, wCode, bClean } =>
         <MAP as INSTANCE>::Socket_Closed (pHost, pSocket, wCode, bClean),

      EVENT::Unknown => {},
   }

   0
}
