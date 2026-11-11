#![allow(non_snake_case)]

use sneeze::*;

// Repaint check. The digits change once a second; the timer is finer so the
// shown second stays close to the wall clock.
const TICK_MS: i32 = 100;

// Used when Data.Panel omits a field. Bound is metres at scale 1. Width and
// height match so the square raster is not stretched. bScale 1 lets
// Transform.Scale and any parent scale multiply that bound. bBillboard 1 faces the
// camera; set it to 2 for Transform.Rotation to aim the panel.
const DEFAULT_POSITION: [f64; 3] = [0.0, 0.0, 0.0];
const DEFAULT_ROTATION: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
const DEFAULT_SCALE:    [f64; 3] = [1.0, 1.0, 1.0];
const DEFAULT_BOUND:    [f64; 3] = [100.0, 100.0, 0.0];
const DEFAULT_WIDTH:     u32 = 0;
const DEFAULT_HEIGHT:    u32 = 512;
const DEFAULT_DENSITY:   u32 = 2;
const DEFAULT_SCALE_MODE: u32 = 1;
const DEFAULT_BILLBOARD: u32 = 1;

struct CLOCK;

#[derive(Copy, Clone)]
struct PLACE
{
   aPosition:  [f64; 3],
   aRotation:  [f64; 4],
   aScale:     [f64; 3],
   aBound:     [f64; 3],
   nWidth:     u32,
   nHeight:    u32,
   nDensity:   u32,
   nScale:     u32,
   nBillboard: u32,
}

struct FACE
{
   twFabricIx: u64,
   pRoot: NODE,
   pPanel: NODE,
   nStamp: i32,
   Place: PLACE,
   bShown: bool,
   bDrawn: bool,
}

static mut FACES: Vec<FACE> = Vec::new ();

fn Faces () -> &'static mut Vec<FACE>
{
   unsafe { &mut *core::ptr::addr_of_mut! (FACES) }
}

fn Default_Place () -> PLACE
{
   PLACE
   {
      aPosition:  DEFAULT_POSITION,
      aRotation:  DEFAULT_ROTATION,
      aScale:     DEFAULT_SCALE,
      aBound:     DEFAULT_BOUND,
      nWidth:     DEFAULT_WIDTH,
      nHeight:    DEFAULT_HEIGHT,
      nDensity:   DEFAULT_DENSITY,
      nScale:     DEFAULT_SCALE_MODE,
      nBillboard: DEFAULT_BILLBOARD,
   }
}

fn Fill_Doubles (sText: &str, aOut: &mut [f64])
{
   let mut nIz = 0;
   let mut nAt = 0;
   let aByte = sText.as_bytes ();

   while nAt < aByte.len () && nIz < aOut.len ()
   {
      let c = aByte[nAt];

      if c != b'-' && c != b'+' && !c.is_ascii_digit ()
      {
         nAt += 1;
         continue;
      }

      let mut nEnd  = nAt + 1;
      let mut nGood = 0;

      while nEnd <= sText.len ()
      {
         if sText[nAt..nEnd].parse::<f64> ().is_ok ()
         {
            nGood = nEnd;
            nEnd += 1;
         }
         else if nGood == 0
         {
            nEnd += 1;
         }
         else
         {
            break;
         }
      }

      if nGood > nAt
      {
         if let Ok (dValue) = sText[nAt..nGood].parse::<f64> ()
         {
            aOut[nIz] = dValue;
            nIz += 1;
         }

         nAt = nGood;
      }
      else
      {
         nAt += 1;
      }
   }
}

fn Fill_Int (sText: &str, nDefault: u32) -> u32
{
   let sText = sText.trim ();

   if let Ok (nValue) = sText.parse::<u32> ()
   {
      nValue
   }
   else if let Ok (dValue) = sText.parse::<f64> ()
   {
      if dValue >= 0.0 { dValue as u32 } else { nDefault }
   }
   else
   {
      nDefault
   }
}

fn Load_Place (pHost: &HOST) -> PLACE
{
   let mut pPlace = Default_Place ();
   let pData = pHost.Data ();

   if !pData.Has ("Panel")
   {
      pHost.Console ().Warn ("clock toggle: Data.Panel is missing, using the built-in placement");
      return pPlace;
   }

   if let Some (sText) = pData.Get ("Panel.Transform.Position") { Fill_Doubles (&sText, &mut pPlace.aPosition); }
   if let Some (sText) = pData.Get ("Panel.Transform.Rotation") { Fill_Doubles (&sText, &mut pPlace.aRotation); }
   if let Some (sText) = pData.Get ("Panel.Transform.Scale")    { Fill_Doubles (&sText, &mut pPlace.aScale); }
   if let Some (sText) = pData.Get ("Panel.Bound.Max")          { Fill_Doubles (&sText, &mut pPlace.aBound); }

   if let Some (sText) = pData.Get ("Panel.Properties.nWidth")     { pPlace.nWidth     = Fill_Int (&sText, pPlace.nWidth); }
   if let Some (sText) = pData.Get ("Panel.Properties.nHeight")    { pPlace.nHeight    = Fill_Int (&sText, pPlace.nHeight); }
   if let Some (sText) = pData.Get ("Panel.Properties.bDensity")   { pPlace.nDensity   = Fill_Int (&sText, pPlace.nDensity); }
   if let Some (sText) = pData.Get ("Panel.Properties.bScale")     { pPlace.nScale     = Fill_Int (&sText, pPlace.nScale); }
   if let Some (sText) = pData.Get ("Panel.Properties.bBillboard") { pPlace.nBillboard = Fill_Int (&sText, pPlace.nBillboard); }

   pPlace
}

fn Stamp (pHost: &HOST) -> (i32, String)
{
   let mNow = pHost.Chrono ().Now ();
   let nHour = mNow.Hour ();
   let nMinute = mNow.Minute ();
   let nSecond = mNow.Second ();

   (
      nHour * 3600 + nMinute * 60 + nSecond,
      format! ("{:02}:{:02}:{:02}", nHour, nMinute, nSecond),
   )
}

// The button stays on the panel either way. Hiding removes the digits; the
// label flips so the same control brings them back. id 'toggle' is the hit
// target once a click reaches this module.
fn Document (sTime: &str, bShown: bool) -> String
{
   let sClock = if bShown
   {
      format! ("<div id='time'>{time}</div><div id='zone'>LOCAL</div>", time = sTime)
   }
   else
   {
      String::new ()
   };

   let sLabel = if bShown { "Hide clock" } else { "Show clock" };

   format! (
      "<rml><head><style>\
       body {{ display: block; width: 100%; height: 100%; margin: 0; font-family: Inter; color: #b7ffd4; }}\
       #face {{ display: block; position: absolute; left: 6%; top: 6%; width: 88%; height: 88%; \
                box-sizing: border-box; background-color: rgba(6, 14, 12, 232); \
                border-width: 2px; border-color: #1c7a58; border-radius: 22px; }}\
       #time {{ display: block; position: absolute; left: 0%; top: 14%; width: 100%; \
                text-align: center; font-size: 64dp; font-weight: 600; color: #7dffb2; }}\
       #zone {{ display: block; position: absolute; left: 0%; top: 42%; width: 100%; \
                text-align: center; font-size: 18dp; letter-spacing: 4dp; color: #7ea892; }}\
       #toggle {{ display: block; position: absolute; left: 10%; top: 70%; width: 80%; height: 18%; \
                  box-sizing: border-box; text-align: center; font-family: Inter; font-size: 22dp; \
                  font-weight: 600; color: #062018; background-color: #7dffb2; \
                  border-width: 0; border-radius: 14px; padding-top: 10dp; }}\
       #toggle:hover {{ background-color: #b9ffd6; }}\
       #toggle:active {{ background-color: #3dcc86; }}\
       </style></head>\
       <body><div id='face'>{clock}<button id='toggle'>{label}</button></div></body></rml>",
      clock = sClock,
      label = sLabel
   )
}

// Panel controls sit in the 32-byte properties region: u16 width, u16 height,
// u8 scale mode, u8 billboard, u8 density. sneeze 0.2.0 writes that region as
// two floats, so those integers go in through the float bits.
fn Configure (pObject: &mut SNEEZE_ABI_MAPOBJECT, pPlace: &PLACE)
{
   let nRaster = (pPlace.nWidth & 0xffff) | ((pPlace.nHeight & 0xffff) << 16);
   let nMode   = (pPlace.nScale & 0xff) | ((pPlace.nBillboard & 0xff) << 8) | ((pPlace.nDensity & 0xff) << 16);

   pObject.Properties_Celestial (f32::from_bits (nRaster), f32::from_bits (nMode), 0, 0.0, 0.0);
}

// Opens a panel with a zero bound, then sets its RML. A zero bound is not
// rasterized, so this write lands before the panel has a document. Reloading
// RML on a panel that is already drawing closes that document on the timer
// thread and crashes the host.
fn OpenPanel (pHost: &HOST, pRoot: NODE, sTime: &str, pPlace: &PLACE, bShown: bool) -> NODE
{
   let mut pFace = SNEEZE_ABI_MAPOBJECT::Panel ();

   pFace.Name ("Clock")
        .Parent (pRoot.Class (), pRoot.ObjectIx ())
        .Position (pPlace.aPosition[0], pPlace.aPosition[1], pPlace.aPosition[2])
        .Rotation (pPlace.aRotation[0], pPlace.aRotation[1], pPlace.aRotation[2], pPlace.aRotation[3])
        .Scale_Axes (pPlace.aScale[0], pPlace.aScale[1], pPlace.aScale[2])
        .Bound (0.0, 0.0, 0.0);

   Configure (&mut pFace, pPlace);

   let pPanel = pHost.Fabric ().Node_Open (&pFace);

   if pPanel.IsValid ()
   {
      pPanel.Panel (&Document (sTime, bShown));
   }

   pPanel
}

fn Show (pPanel: NODE, pPlace: &PLACE)
{
   pPanel.Bound (pPlace.aBound[0], pPlace.aBound[1], pPlace.aBound[2]);
}

fn Replace (pHost: &HOST)
{
   let twFabricIx = pHost.Index ();
   let (nNow, sTime) = Stamp (pHost);

   let pSlot = {
      let pFace = Faces ().iter ().find (|pFace| pFace.twFabricIx == twFabricIx);

      let Some (pFace) = pFace else { return };

      let bTime = pFace.bShown && pFace.nStamp != nNow;
      let bVis  = pFace.bShown != pFace.bDrawn;

      if !bTime && !bVis
      {
         return;
      }

      (pFace.pRoot, pFace.pPanel, pFace.Place, pFace.bShown)
   };

   let pPanel = OpenPanel (pHost, pSlot.0, &sTime, &pSlot.2, pSlot.3);

   if !pPanel.IsValid ()
   {
      pHost.Console ().Error ("clock toggle: replacement panel was not created");
      return;
   }

   // Show the new face before retiring the old one, so a frame in between
   // still has a clock-sized bound and the camera does not jump.
   Show (pPanel, &pSlot.2);

   if let Some (pFace) = Faces ().iter_mut ().find (|pFace| pFace.twFabricIx == twFabricIx)
   {
      pFace.pPanel = pPanel;
      pFace.bDrawn = pSlot.3;

      if pSlot.3
      {
         pFace.nStamp = nNow;
      }
   }

   if !pHost.Fabric ().Node_Close (pSlot.1)
   {
      pSlot.1.Bound (0.0, 0.0, 0.0);
   }
}

// Click on the button whose id is "toggle".
fn Toggle (pHost: &HOST)
{
   let twFabricIx = pHost.Index ();

   if let Some (pFace) = Faces ().iter_mut ().find (|pFace| pFace.twFabricIx == twFabricIx)
   {
      pFace.bShown = !pFace.bShown;
   }

   Replace (pHost);
}

impl INSTANCE for CLOCK
{
   fn Open (pHost: &HOST)
   {
      let mut pRootObj = SNEEZE_ABI_MAPOBJECT::Root ();
      pRootObj.Name ("Clock Toggle");
      let pRoot = pHost.Fabric ().Node_Root (&pRootObj);

      if !pRoot.IsValid ()
      {
         pHost.Console ().Error ("clock toggle: root node was not created");
         return;
      }

      let Place = Load_Place (pHost);
      let (nStamp, sTime) = Stamp (pHost);
      let bShown = true;
      let pPanel = OpenPanel (pHost, pRoot, &sTime, &Place, bShown);

      if !pPanel.IsValid ()
      {
         pHost.Console ().Error ("clock toggle: panel was not created");
         return;
      }

      Show (pPanel, &Place);

      let twFabricIx = pHost.Index ();
      let pFaces = Faces ();

      pFaces.retain (|pFace| pFace.twFabricIx != twFabricIx);
      pFaces.push (FACE { twFabricIx, pRoot, pPanel, nStamp, Place, bShown, bDrawn: bShown });

      let twTimerIx = pHost.Timer ().Interval (TICK_MS, eSNEEZE_ABI_TIMER_UNIT::kSNEEZE_ABI_TIMER_UNIT_MS, 0);

      if twTimerIx == 0
      {
         pHost.Console ().Error ("clock toggle: timer was not armed");
      }
   }

   fn Timer (pHost: &HOST, _twTimerIx: u64, _qwParam: u64)
   {
      Replace (pHost);
   }

   fn Close (pHost: &HOST)
   {
      let twFabricIx = pHost.Index ();
      Faces ().retain (|pFace| pFace.twFabricIx != twFabricIx);
   }
}

// Viewport pointer notify (method 5): header, fabric index, id byte count, UTF-8 id.
fn Click (nOffset: i32, nSize: i32) -> Option<(&'static HOST, String)>
{
   if nOffset == 0 || nSize < 24
   {
      return None;
   }

   let aByte = unsafe { core::slice::from_raw_parts (nOffset as u32 as *const u8, nSize as usize) };
   let wType   = u16::from_le_bytes ([aByte[0], aByte[1]]);
   let wMethod = u16::from_le_bytes ([aByte[2], aByte[3]]);

   if wType != 5 || wMethod != 5
   {
      return None;
   }

   let twFabricIx = u64::from_le_bytes (aByte[8..16].try_into ().ok ()?);
   let nLen = u64::from_le_bytes (aByte[16..24].try_into ().ok ()?) as usize;
   let nEnd = 24 + nLen;

   if nLen > 128 || nEnd > aByte.len ()
   {
      return None;
   }

   let sId = core::str::from_utf8 (&aByte[24..nEnd]).ok ()?;
   let pHost = Host_Find (twFabricIx)?;

   Some ((pHost, sId.to_string ()))
}

#[no_mangle]
pub extern "C" fn Init ()
{
   <CLOCK as INSTANCE>::Init ();
}

#[no_mangle]
pub extern "C" fn Open (twFabricIx: u64, nOffset: i32, nSize: i32)
{
   sneeze::Snapshot_Load (sneeze::SNAPSHOT::From_Raw (nOffset, nSize));
   <CLOCK as INSTANCE>::Open (sneeze::Host_Acquire (twFabricIx));
}

#[no_mangle]
pub extern "C" fn Close (twFabricIx: u64)
{
   if let Some (pHost) = Host_Find (twFabricIx)
   {
      <CLOCK as INSTANCE>::Close (pHost);
   }

   sneeze::Host_Release (twFabricIx);
}

#[no_mangle]
pub extern "C" fn Shutdown ()
{
   <CLOCK as INSTANCE>::Shutdown ();
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
   match sneeze::Event_Parse (nOffset, nSize)
   {
      sneeze::EVENT::Timer { pHost, twTimerIx, qwParam } =>
         <CLOCK as INSTANCE>::Timer (pHost, twTimerIx, qwParam),

      sneeze::EVENT::Request { pHost, pRequest, bSuccess } =>
         <CLOCK as INSTANCE>::Request (pHost, pRequest, bSuccess),

      sneeze::EVENT::Socket_Opened { pHost, pSocket } =>
         <CLOCK as INSTANCE>::Socket_Opened (pHost, pSocket),

      sneeze::EVENT::Socket_Received { pHost, pSocket, bBinary, nSize } =>
         <CLOCK as INSTANCE>::Socket_Received (pHost, pSocket, bBinary, nSize),

      sneeze::EVENT::Socket_Failed { pHost, pSocket } =>
         <CLOCK as INSTANCE>::Socket_Failed (pHost, pSocket),

      sneeze::EVENT::Socket_Closed { pHost, pSocket, wCode, bClean } =>
         <CLOCK as INSTANCE>::Socket_Closed (pHost, pSocket, wCode, bClean),

      sneeze::EVENT::Unknown =>
      {
         if let Some ((pHost, sId)) = Click (nOffset, nSize)
         {
            if sId == "toggle"
            {
               Toggle (pHost);
            }
         }
      }
   }

   0
}
