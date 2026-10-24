#![allow(non_snake_case)]

use sneeze::*;

// Repaint check. The digits change once a second; the timer is finer so the
// shown second stays close to the wall clock.
const TICK_MS: i32 = 100;

// The panel texture is square, and the quad uses this same ratio so the
// digits are not stretched.
const FACE_ASPECT: f64 = 1.0;

struct CLOCK;

struct FACE
{
   twFabricIx: u64,
   pRoot: NODE,
   pPanel: NODE,
   nStamp: i32,
}

static mut FACES: Vec<FACE> = Vec::new ();

fn Faces () -> &'static mut Vec<FACE>
{
   unsafe { &mut *core::ptr::addr_of_mut! (FACES) }
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

fn Document (sTime: &str) -> String
{
   format! (
      "<rml><head><style>\
       body {{ display: block; width: 100%; height: 100%; margin: 0; font-family: Inter; color: #b7ffd4; }}\
       #face {{ display: block; position: absolute; left: 6%; top: 10%; width: 88%; height: 80%; \
                box-sizing: border-box; background-color: rgba(6, 14, 12, 232); \
                border-width: 2px; border-color: #1c7a58; border-radius: 22px; }}\
       #time {{ display: block; position: absolute; left: 0%; top: 28%; width: 100%; \
                text-align: center; font-size: 72px; font-weight: 600; color: #7dffb2; }}\
       #zone {{ display: block; position: absolute; left: 0%; top: 62%; width: 100%; \
                text-align: center; font-size: 18px; letter-spacing: 4px; color: #7ea892; }}\
       </style></head>\
       <body><div id='face'><div id='time'>{time}</div><div id='zone'>LOCAL</div></div></body></rml>",
      time = sTime
   )
}

// Opens a panel with a zero bound, then sets its RML. A zero bound is not
// rasterized, so this write lands before the panel has a document. Reloading
// RML on a panel that is already drawing closes that document on the timer
// thread and crashes the host.
fn OpenPanel (pHost: &HOST, pRoot: NODE, sTime: &str) -> NODE
{
   let mut pFace = SNEEZE_ABI_MAPOBJECT::Panel ();

   pFace.Name ("Clock")
        .Parent (pRoot.Class (), pRoot.ObjectIx ())
        .Bound (0.0, 0.0, 0.0);

   let pPanel = pHost.Fabric ().Node_Open (&pFace);

   if pPanel.IsValid ()
   {
      pPanel.Panel (&Document (sTime));
   }

   pPanel
}

fn Show (pPanel: NODE)
{
   pPanel.Bound (FACE_ASPECT, 1.0, 0.0);
}

fn Replace (pHost: &HOST)
{
   let twFabricIx = pHost.Index ();
   let (nNow, sTime) = Stamp (pHost);

   let pSlot = {
      let pFace = Faces ().iter ().find (|pFace| pFace.twFabricIx == twFabricIx);

      let Some (pFace) = pFace else { return };

      if pFace.nStamp == nNow
      {
         return;
      }

      (pFace.pRoot, pFace.pPanel)
   };

   let pPanel = OpenPanel (pHost, pSlot.0, &sTime);

   if !pPanel.IsValid ()
   {
      pHost.Console ().Error ("digital clock: replacement panel was not created");
      return;
   }

   // Show the new face before retiring the old one, so a frame in between
   // still has a clock-sized bound and the camera does not jump.
   Show (pPanel);

   if let Some (pFace) = Faces ().iter_mut ().find (|pFace| pFace.twFabricIx == twFabricIx)
   {
      pFace.pPanel = pPanel;
      pFace.nStamp = nNow;
   }

   if !pHost.Fabric ().Node_Close (pSlot.1)
   {
      pSlot.1.Bound (0.0, 0.0, 0.0);
   }
}

impl INSTANCE for CLOCK
{
   fn Open (pHost: &HOST)
   {
      let mut pRootObj = SNEEZE_ABI_MAPOBJECT::Root ();
      pRootObj.Name ("Digital Clock");
      let pRoot = pHost.Fabric ().Node_Root (&pRootObj);

      if !pRoot.IsValid ()
      {
         pHost.Console ().Error ("digital clock: root node was not created");
         return;
      }

      let (nStamp, sTime) = Stamp (pHost);
      let pPanel = OpenPanel (pHost, pRoot, &sTime);

      if !pPanel.IsValid ()
      {
         pHost.Console ().Error ("digital clock: panel was not created");
         return;
      }

      Show (pPanel);

      let twFabricIx = pHost.Index ();
      let pFaces = Faces ();

      pFaces.retain (|pFace| pFace.twFabricIx != twFabricIx);
      pFaces.push (FACE { twFabricIx, pRoot, pPanel, nStamp });

      let twTimerIx = pHost.Timer ().Interval (TICK_MS, eSNEEZE_ABI_TIMER_UNIT::kSNEEZE_ABI_TIMER_UNIT_MS, 0);

      if twTimerIx == 0
      {
         pHost.Console ().Error ("digital clock: timer was not armed");
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

sneeze::instance! (CLOCK);
