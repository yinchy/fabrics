// Copyright 2026 Metaversal Corporation

// ===========================================================================
// DFW_ADSBfi.c - ADSB.fi traffic around Dallas/Fort Worth, as a WASM module.
//
// Sibling of DFW_ADSB. On Open the module GETs ADSB.fi v3 for the
// configured radius, then polls (ADSB.nIntervalSec, default 5). Meshes
// stream in on a timer: first the DFW pad, then directional aircraft icons.
// Nothing is planted during Open — a GLB there aborts Halogen while chrome
// UnlitBlend is still binding. Later polls merge by icao24: each fix is
// appended as a waypoint. Every icon is sampled on the same lagging UTC
// clock (Chrono_Date minus ADSB.nLagSec, default one poll). If the playhead
// catches the newest fix, lag grows by one poll and the head hops back so
// interpolation never runs out of a next waypoint. New aircraft spawn;
// departed ones close. One summary card is created after the first icons, RML set once (never
// rewritten — a live UnlitBlend panel rewrite aborts Artemis). Each aircraft also
// carries a billboard above it (callsign, type, speed). That card is opened at a
// zero bound, given its RML, then shown — the same order as the clock. A text
// change opens a replacement before the old card is closed. The card is closed
// when the aircraft leaves. Digit 0-9
// also re-fetches. The GLB is treated as 1 m long. Guest scale is airframe
// length in metres (JSON IconLengthMetres, default 40). ADSB.fi `t`
// typecodes override that when the lookup table knows them — no second
// metadata GET. Icons stay 1:1 — no distance LOD. Reload calls Close before
// a second Open so in-flight GLBs are not leaked into a second spawn wave.
// ===========================================================================

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "sneeze.h"

// --- Design constants ------------------------------------------------------

#define MAX_AIRCRAFT        128
#define MAX_WAYPOINTS       32
#define WP_MIN_MS           500
#define WP_MAX_MS           180000
#define CRAFTS_PER_TICK     1
#define SPAWN_HZ            30
#define HOLD_TICKS_OPEN     24        // ~0.8 s after Open; lets Halogen finish a reload
#define HOLD_TICKS_CLOSE    8         // ~0.25 s after any Node_Close before the next Open
#define MAX_STATE_ROWS      512
#define METRES_PER_MILE     1609.344
#define METRES_PER_NM       1852.0
#define METRES_PER_DEG_LAT  111320.0
#define METRES_PER_FOOT     0.3048
#define ORIGIN_PAD_METRES   64.0      // unit disk in origin.glb, metres
#define REQUEST_TIMEOUT_MS  15000
#define LEGEND_ROWS         10
#define ICON_LEN_DEFAULT    40.0      // typical narrowbody; override per type
#define POLL_DEFAULT_SEC    5
#define POLL_MIN_SEC        1
#define POLL_MAX_SEC        120
#define LAG_PAD_MS          2000      // extra delay so a late poll still has a next waypoint
#define ADSB_URL_DEFAULT    "https://opendata.adsb.fi/api/v3"

#define DFW_LAT_DEFAULT     32.896944
#define DFW_LON_DEFAULT     -97.038056
#define RADIUS_MI_DEFAULT   10.0
#define PANEL_X_DEFAULT     0.0
#define PANEL_Y_DEFAULT     0.0
#define PANEL_Z_DEFAULT     1500.0
#define PANEL_SCALE_DEFAULT 0.375     // painted card as a fraction of the fixed billboard quad
#define PANEL_W_DEFAULT     1.0
#define PANEL_H_DEFAULT     1.0
#define LABEL_W_M           50.0      // billboard width, metres at scale 1
#define LABEL_H_M           16.0
#define LABEL_ABOVE_M       12.0      // extra metres above the icon
#define LABEL_RASTER_H      160
#define LABEL_DENSITY       2
#define LABEL_SCALE_MODE    1         // bound times node scale, in metres
#define LABEL_BILLBOARD     1         // face the camera, camera up
#define LABELS_PER_TICK     4

static const double PI = 3.14159265358979323846;

// --- Per-aircraft nodes ----------------------------------------------------

typedef struct
{
   double  dLat;
   double  dLon;
   double  dAltM;
   double  dHeading;
   int64_t tmMs;
}
WAYPOINT;

typedef struct
{
   char     sId[16];
   char     sIcao[8];
   char     sType[8];
   double   dLat;
   double   dLon;
   double   dAltM;
   double   dHeading;
   double   dLengthM;
   double   dGsKt;
   bool     bGround;
   bool     bLive;
   bool     bTried;
   HNODE    qwIcon;
   HNODE    qwLabel;
   char     sSign[80];
   WAYPOINT aWp[MAX_WAYPOINTS];
   int      nWp;
}
CRAFT;

static HOST*    s_pHost      = 0;
static HREQUEST s_hRequest   = 0;
static uint64_t s_twTimer    = 0;
static CRAFT    s_aCraft[MAX_AIRCRAFT];
static int      s_nCraft     = 0;
static uint16_t s_wRootClass = 0;
static uint64_t s_twRootIx   = 0;
static HNODE    s_qwPad      = 0;
static HNODE    s_qwLegend   = 0;
static bool     s_bPad       = false;
static bool     s_bListed    = false;
static bool     s_bLegend      = false;
static bool     s_bPanelWanted = true;
static int      s_nPollTick  = 0;
static int      s_nPollSec   = POLL_DEFAULT_SEC;
static int      s_nLagSec    = POLL_DEFAULT_SEC;
static int      s_nHoldTicks = 0;

static double   s_dOriginLat  = DFW_LAT_DEFAULT;
static double   s_dOriginLon  = DFW_LON_DEFAULT;
static double   s_dRadiusMi   = RADIUS_MI_DEFAULT;
static double   s_dIconLen    = ICON_LEN_DEFAULT;
static double   s_dPanelX     = PANEL_X_DEFAULT;
static double   s_dPanelY     = PANEL_Y_DEFAULT;
static double   s_dPanelZ     = PANEL_Z_DEFAULT;
static double   s_dPanelScale = PANEL_SCALE_DEFAULT;
static double   s_dPanelW     = PANEL_W_DEFAULT;
static double   s_dPanelH     = PANEL_H_DEFAULT;
static double   s_dLabelW     = LABEL_W_M;
static double   s_dLabelH     = LABEL_H_M;
static double   s_dLabelSX    = 1.0;
static double   s_dLabelSY    = 1.0;
static double   s_dLabelAbove = LABEL_ABOVE_M;
static int      s_nLabelPixW  = 0;
static int      s_nLabelPixH  = LABEL_RASTER_H;
static int      s_nLabelDen   = LABEL_DENSITY;
static int      s_nLabelScale = LABEL_SCALE_MODE;
static int      s_nLabelBill  = LABEL_BILLBOARD;
static bool     s_bLabelLogged = false;

static void Fetch_ADSB (void);
static void Craft_Label (const CRAFT* p, char* sOut, int nCap);
static void Scene_Hold (int nTicks);
static void Label_Service (void);

// --- Tiny JSON cursor (ADSB.fi /v3 ac[]) ----------------------------------

typedef struct
{
   const char* s;
   const char* sEnd;
}
JSON;

static void Json_Skip (JSON* p)
{
   while (p->s < p->sEnd)
   {
      char c = *p->s;
      if (c != ' '  &&  c != '\t'  &&  c != '\n'  &&  c != '\r')
         break;
      p->s++;
   }
}

static char Json_Peek (JSON* p)
{
   Json_Skip (p);
   return (p->s < p->sEnd) ? *p->s : 0;
}

static bool Json_Take (JSON* p, char c)
{
   if (Json_Peek (p) != c)
      return false;
   p->s++;
   return true;
}

static bool Json_SkipString (JSON* p)
{
   if (!Json_Take (p, '"'))
      return false;

   while (p->s < p->sEnd)
   {
      char c = *p->s++;
      if (c == '"')
         return true;
      if (c == '\\'  &&  p->s < p->sEnd)
         p->s++;
   }
   return false;
}

static bool Json_SkipLiteral (JSON* p)
{
   Json_Skip (p);
   if (p->s + 4 <= p->sEnd  &&  memcmp (p->s, "true",  4) == 0) { p->s += 4; return true; }
   if (p->s + 5 <= p->sEnd  &&  memcmp (p->s, "false", 5) == 0) { p->s += 5; return true; }
   if (p->s + 4 <= p->sEnd  &&  memcmp (p->s, "null",  4) == 0) { p->s += 4; return true; }
   return false;
}

static bool Json_SkipNumber (JSON* p)
{
   Json_Skip (p);
   if (p->s < p->sEnd  &&  (*p->s == '-' || *p->s == '+'))
      p->s++;
   int nDigit = 0;
   while (p->s < p->sEnd  &&  *p->s >= '0'  &&  *p->s <= '9') { p->s++; nDigit++; }
   if (p->s < p->sEnd  &&  *p->s == '.')
   {
      p->s++;
      while (p->s < p->sEnd  &&  *p->s >= '0'  &&  *p->s <= '9') { p->s++; nDigit++; }
   }
   if (p->s < p->sEnd  &&  (*p->s == 'e' || *p->s == 'E'))
   {
      p->s++;
      if (p->s < p->sEnd  &&  (*p->s == '-' || *p->s == '+'))
         p->s++;
      while (p->s < p->sEnd  &&  *p->s >= '0'  &&  *p->s <= '9') p->s++;
   }
   return nDigit > 0;
}

static bool Json_SkipValue (JSON* p);

static bool Json_SkipObject (JSON* p)
{
   if (!Json_Take (p, '{'))
      return false;
   if (Json_Take (p, '}'))
      return true;

   for (;;)
   {
      if (!Json_SkipString (p)  ||  !Json_Take (p, ':')  ||  !Json_SkipValue (p))
         return false;
      if (Json_Take (p, '}'))
         return true;
      if (!Json_Take (p, ','))
         return false;
   }
}

static bool Json_SkipArray (JSON* p)
{
   if (!Json_Take (p, '['))
      return false;
   if (Json_Take (p, ']'))
      return true;

   for (;;)
   {
      if (!Json_SkipValue (p))
         return false;
      if (Json_Take (p, ']'))
         return true;
      if (!Json_Take (p, ','))
         return false;
   }
}

static bool Json_SkipValue (JSON* p)
{
   char c = Json_Peek (p);
   if (c == '"') return Json_SkipString (p);
   if (c == '{') return Json_SkipObject (p);
   if (c == '[') return Json_SkipArray  (p);
   if (c == 't'  ||  c == 'f'  ||  c == 'n') return Json_SkipLiteral (p);
   return Json_SkipNumber (p);
}

// Never stall the cursor: a failed field parse must still consume something
// or Artemis freezes in the WASM loop for the rest of the session.
static void Json_Bump (JSON* p, const char* s0)
{
   if (p->s != s0)
      return;
   Json_SkipValue (p);
   if (p->s == s0  &&  p->s < p->sEnd)
      p->s++;
}

static bool Json_FindKey (JSON* p, const char* sKey)
{
   if (!Json_Take (p, '{'))
      return false;

   size_t nKey = strlen (sKey);

   while (Json_Peek (p) != '}'  &&  Json_Peek (p) != 0)
   {
      const char* s0 = p->s;

      if (!Json_Take (p, '"'))
         return false;

      const char* sName = p->s;
      while (p->s < p->sEnd  &&  *p->s != '"')
      {
         if (*p->s == '\\'  &&  p->s + 1 < p->sEnd)
            p->s += 2;
         else
            p->s++;
      }
      size_t nName = (size_t) (p->s - sName);
      if (p->s >= p->sEnd)
         return false;
      p->s++;

      if (!Json_Take (p, ':'))
         return false;

      if (nName == nKey  &&  memcmp (sName, sKey, nKey) == 0)
         return true;

      if (!Json_SkipValue (p))
         return false;
      Json_Take (p, ',');
      if (p->s == s0)
         return false;
   }
   return false;
}

static bool Json_IsNull (JSON* p)
{
   Json_Skip (p);
   return (p->s + 4 <= p->sEnd  &&  memcmp (p->s, "null", 4) == 0);
}

static bool Json_Bool (JSON* p, bool* pb)
{
   Json_Skip (p);
   if (p->s + 4 <= p->sEnd  &&  memcmp (p->s, "true", 4) == 0)
   {
      p->s += 4;
      *pb = true;
      return true;
   }
   if (p->s + 5 <= p->sEnd  &&  memcmp (p->s, "false", 5) == 0)
   {
      p->s += 5;
      *pb = false;
      return true;
   }
   return false;
}

static bool Json_Number (JSON* p, double* pd)
{
   Json_Skip (p);
   if (Json_IsNull (p))
   {
      p->s += 4;
      return false;
   }

   char  aBuf[64];
   int   n = 0;
   const char* s0 = p->s;

   if (!Json_SkipNumber (p))
      return false;

   n = (int) (p->s - s0);
   if (n <= 0  ||  n >= (int) sizeof (aBuf))
      return false;

   memcpy (aBuf, s0, (size_t) n);
   aBuf[n] = 0;
   *pd = atof (aBuf);
   return true;
}

static bool Json_String (JSON* p, char* sOut, int nCap)
{
   Json_Skip (p);
   if (Json_IsNull (p))
   {
      p->s += 4;
      if (nCap > 0)
         sOut[0] = 0;
      return false;
   }
   if (!Json_Take (p, '"'))
      return false;

   int n = 0;
   while (p->s < p->sEnd)
   {
      char c = *p->s++;
      if (c == '"')
         break;
      if (c == '\\'  &&  p->s < p->sEnd)
      {
         char e = *p->s++;
         if (e == 'n') c = '\n';
         else if (e == 't') c = '\t';
         else c = e;
      }
      if (n + 1 < nCap)
         sOut[n++] = c;
   }
   if (nCap > 0)
      sOut[n] = 0;
   return true;
}

// --- Text helpers ----------------------------------------------------------

static void Trim (char* s)
{
   char* p = s;
   while (*p == ' '  ||  *p == '\t')
      p++;
   if (p != s)
      memmove (s, p, strlen (p) + 1);

   size_t n = strlen (s);
   while (n > 0  &&  (s[n - 1] == ' '  ||  s[n - 1] == '\t'))
      s[--n] = 0;
}

static void Strip_Json_String (char* s)
{
   size_t n = strlen (s);
   if (n >= 2  &&  s[0] == '"'  &&  s[n - 1] == '"')
   {
      s[n - 1] = 0;
      memmove (s, s + 1, n - 1);
   }
}

static bool Data_Number (HOST* pHost, const char* sPath, double dDefault, double* pd)
{
   char* s = Data_Get (pHost, sPath);

   *pd = dDefault;
   if (s == NULL)
      return false;

   Strip_Json_String (s);
   *pd = atof (s);
   free (s);
   return true;
}

static int Data_Int (HOST* pHost, const char* sPath, int nDefault)
{
   double d = (double) nDefault;

   Data_Number (pHost, sPath, d, &d);
   if (d <= -0.5  ||  d >= 1000000.0)
      return nDefault;
   return (int) (d + 0.5);
}

static int Fill_Doubles (const char* sText, double* aOut, int nCap)
{
   int         nIz = 0;
   const char* s   = sText;

   if (s == 0)
      return 0;

   while (*s  &&  nIz < nCap)
   {
      char*  sEnd;
      double d;

      while (*s  &&  *s != '-'  &&  *s != '+'  &&  (*s < '0'  ||  *s > '9'))
         s++;
      if (*s == 0)
         break;

      d = strtod (s, &sEnd);
      if (sEnd == s)
      {
         s++;
         continue;
      }

      aOut[nIz++] = d;
      s = sEnd;
   }

   return nIz;
}

static void Label_Load (HOST* pHost)
{
   char*  sBound;
   char*  sScale;
   double aBound[3];
   double aScale[3];

   s_dLabelW     = LABEL_W_M;
   s_dLabelH     = LABEL_H_M;
   s_dLabelSX    = 1.0;
   s_dLabelSY    = 1.0;
   s_dLabelAbove = LABEL_ABOVE_M;
   s_nLabelPixW  = 0;
   s_nLabelPixH  = LABEL_RASTER_H;
   s_nLabelDen   = LABEL_DENSITY;
   s_nLabelScale = LABEL_SCALE_MODE;
   s_nLabelBill  = LABEL_BILLBOARD;

   sBound = Data_Get (pHost, "Label.Bound.Max");
   if (sBound != 0)
   {
      aBound[0] = s_dLabelW;
      aBound[1] = s_dLabelH;
      aBound[2] = 0.0;
      if (Fill_Doubles (sBound, aBound, 3) >= 2)
      {
         s_dLabelW = aBound[0];
         s_dLabelH = aBound[1];
      }
      free (sBound);
   }

   sScale = Data_Get (pHost, "Label.Transform.Scale");
   if (sScale != 0)
   {
      aScale[0] = 1.0;
      aScale[1] = 1.0;
      aScale[2] = 1.0;
      if (Fill_Doubles (sScale, aScale, 3) >= 2)
      {
         s_dLabelSX = aScale[0];
         s_dLabelSY = aScale[1];
      }
      free (sScale);
   }

   Data_Number (pHost, "Label.dAbove", LABEL_ABOVE_M, &s_dLabelAbove);
   s_nLabelPixW  = Data_Int (pHost, "Label.Properties.nWidth",     s_nLabelPixW);
   s_nLabelPixH  = Data_Int (pHost, "Label.Properties.nHeight",    s_nLabelPixH);
   s_nLabelDen   = Data_Int (pHost, "Label.Properties.bDensity",   s_nLabelDen);
   s_nLabelScale = Data_Int (pHost, "Label.Properties.bScale",     s_nLabelScale);
   s_nLabelBill  = Data_Int (pHost, "Label.Properties.bBillboard", s_nLabelBill);

   if (s_dLabelSX <= 0.0)
      s_dLabelSX = 1.0;
   if (s_dLabelSY <= 0.0)
      s_dLabelSY = 1.0;
}

static bool Data_String (HOST* pHost, const char* sPath, char* sOut, int nCap)
{
   char* s = Data_Get (pHost, sPath);

   if (nCap > 0)
      sOut[0] = 0;
   if (s == NULL)
      return false;

   Strip_Json_String (s);
   strncpy (sOut, s, (size_t) nCap - 1);
   sOut[nCap - 1] = 0;
   free (s);
   return sOut[0] != 0;
}

static bool Text_Means_Off (const char* s)
{
   char a[16];
   int  i;

   if (s == 0  ||  s[0] == 0)
      return true;

   for (i = 0; i < 15  &&  s[i]; i++)
   {
      char c = s[i];
      if (c >= 'A'  &&  c <= 'Z')
         c = (char) (c + 32);
      a[i] = c;
   }
   a[i] = 0;

   return strcmp (a, "0") == 0
       ||  strcmp (a, "false") == 0
       ||  strcmp (a, "off") == 0
       ||  strcmp (a, "no") == 0;
}

static bool Query_Param (const char* sUrl, const char* sKey, char* sOut, int nCap)
{
   const char* q;
   int         nKey;

   if (nCap > 0)
      sOut[0] = 0;
   if (sUrl == 0  ||  sKey == 0  ||  sKey[0] == 0)
      return false;

   q = strchr (sUrl, '?');
   if (q == 0)
      return false;
   q++;
   nKey = (int) strlen (sKey);

   while (*q)
   {
      const char* sAmp = q;
      const char* sEq;
      int         nPair;
      int         nName;

      while (*sAmp  &&  *sAmp != '&')
         sAmp++;
      nPair = (int) (sAmp - q);
      sEq   = q;
      while (sEq < sAmp  &&  *sEq != '=')
         sEq++;
      nName = (int) (sEq - q);

      if (nName == nKey  &&  memcmp (q, sKey, (size_t) nKey) == 0)
      {
         if (sEq < sAmp  &&  nCap > 0)
         {
            int nVal = (int) (sAmp - sEq - 1);
            if (nVal >= nCap)
               nVal = nCap - 1;
            memcpy (sOut, sEq + 1, (size_t) nVal);
            sOut[nVal] = 0;
         }
         return true;
      }

      q = *sAmp ? sAmp + 1 : sAmp;
   }

   return false;
}

static bool Url_Panel_Off (const char* sUrl)
{
   char aVal[16];

   aVal[0] = 0;
   if (Query_Param (sUrl, "nopanel", aVal, (int) sizeof (aVal)))
      return aVal[0] == 0  ||  Text_Means_Off (aVal)  ||  strcmp (aVal, "1") == 0;
   aVal[0] = 0;
   if (Query_Param (sUrl, "panel", aVal, (int) sizeof (aVal)))
      return Text_Means_Off (aVal);
   return false;
}

static void Panel_Wanted_Init (HOST* pHost)
{
   char aVis[16];

   s_bPanelWanted = true;
   aVis[0] = 0;
   if (Data_String (pHost, "Panel.bVisible", aVis, (int) sizeof (aVis))
    &&  Text_Means_Off (aVis))
      s_bPanelWanted = false;

   if (Url_Panel_Off (Location_Href (pHost))
    ||  Url_Panel_Off (Location_Pathname (pHost)))
      s_bPanelWanted = false;

   if (!s_bPanelWanted)
      Console_Log (pHost, "dfw-adsbfi: panel disabled");
}

// Heading 0 is north (+Y); the GLB's identity forward is +X (east).
// A +Z rotation of (90 - heading) degrees aims the nose along true track.
static void Heading_Quat (double dHeadingDeg, double* aQ)
{
   double dYaw = (90.0 - dHeadingDeg) * PI / 180.0;
   double dH   = dYaw * 0.5;

   aQ[0] = 0.0;
   aQ[1] = 0.0;
   aQ[2] = sin (dH);
   aQ[3] = cos (dH);
}

static void Geodetic_To_Local (double dLat, double dLon, double dAltM,
                               double* pdX, double* pdY, double* pdZ)
{
   double dLat0 = s_dOriginLat * PI / 180.0;
   double dMPerDegLon = METRES_PER_DEG_LAT * cos (dLat0);

   if (dMPerDegLon < 1.0)
      dMPerDegLon = 1.0;

   *pdX = (dLon - s_dOriginLon) * dMPerDegLon;
   *pdY = (dLat - s_dOriginLat) * METRES_PER_DEG_LAT;
   *pdZ = dAltM;
}

static double Distance_M (double dLat, double dLon)
{
   double dX, dY, dZ;
   Geodetic_To_Local (dLat, dLon, 0.0, &dX, &dY, &dZ);
   return sqrt (dX * dX + dY * dY);
}

// ICAO typecode → published overall length, metres. Exact match first, then
// a 3-character family (B73, A32, B77). Unknown types keep IconLengthMetres.
typedef struct
{
   const char* sCode;
   float       fLen;
}
TYPELEN;

static const TYPELEN s_aTypeLen[] =
{
   { "A318", 31.4f }, { "A319", 33.8f }, { "A31N", 33.8f },
   { "A320", 37.6f }, { "A20N", 37.6f }, { "A321", 44.5f }, { "A21N", 44.5f },
   { "A332", 58.8f }, { "A333", 63.7f }, { "A338", 58.8f }, { "A339", 63.7f },
   { "A342", 59.4f }, { "A343", 63.7f }, { "A345", 67.9f }, { "A346", 75.3f },
   { "A359", 64.8f }, { "A35K", 73.8f }, { "A388", 72.7f },
   { "A306", 53.6f }, { "A30B", 53.6f }, { "A310", 46.7f },
   { "BCS1", 35.0f }, { "BCS3", 38.7f },
   { "B712", 37.8f },
   { "B732", 30.5f }, { "B733", 33.4f }, { "B734", 36.4f }, { "B735", 31.0f },
   { "B736", 31.2f }, { "B737", 33.6f }, { "B738", 39.5f }, { "B739", 42.1f },
   { "B37M", 35.6f }, { "B38M", 39.5f }, { "B39M", 42.2f }, { "B3XM", 43.8f },
   { "B741", 70.7f }, { "B742", 70.7f }, { "B743", 70.7f }, { "B744", 70.7f },
   { "B748", 76.3f },
   { "B752", 47.3f }, { "B753", 54.4f },
   { "B762", 48.5f }, { "B763", 54.9f }, { "B764", 61.4f },
   { "B772", 63.7f }, { "B773", 73.9f }, { "B77L", 63.7f }, { "B77W", 73.9f },
   { "B788", 56.7f }, { "B789", 62.8f }, { "B78X", 68.3f },
   { "MD11", 61.6f }, { "MD82", 45.1f }, { "MD83", 45.1f }, { "MD88", 45.1f },
   { "E135", 26.3f }, { "E145", 29.9f }, { "E170", 29.9f },
   { "E75L", 31.6f }, { "E75S", 31.6f }, { "E190", 36.2f }, { "E195", 41.5f },
   { "E290", 36.2f }, { "E295", 41.5f },
   { "CRJ1", 26.8f }, { "CRJ2", 26.8f }, { "CRJ7", 32.3f }, { "CRJ9", 36.4f },
   { "CRJX", 39.1f },
   { "DH8A", 22.3f }, { "DH8B", 25.7f }, { "DH8C", 25.7f }, { "DH8D", 32.8f },
   { "AT43", 22.7f }, { "AT45", 22.7f }, { "AT72", 27.2f }, { "AT76", 27.2f },
   { "C172",  8.3f }, { "C182",  8.8f }, { "C208", 11.5f },
   { "C25A", 12.8f }, { "C25B", 14.4f }, { "C56X", 16.0f }, { "C680", 19.4f },
   { "C68A", 22.0f }, { "C700", 21.0f }, { "C750", 21.9f },
   { "BE20", 13.4f }, { "B350", 14.2f }, { "PC12", 14.4f },
   { "GLF4", 26.9f }, { "GLF5", 29.4f }, { "GLF6", 30.7f }, { "G280", 20.0f },
   { "CL30", 20.9f }, { "CL35", 20.9f }, { "CL60", 20.9f }, { "GLEX", 29.7f },
   { "FA7X", 23.2f }, { "FA8X", 26.2f },
   { "A139", 13.8f }, { "EC35", 12.9f }, { "B412", 12.7f },
   { "B73",  39.5f }, { "B77",  70.0f }, { "B78",  63.0f }, { "B74",  71.0f },
   { "B75",  48.0f }, { "B76",  55.0f },
   { "A31",  33.8f }, { "A32",  40.0f }, { "A33",  61.0f }, { "A34",  67.0f },
   { "A35",  67.0f }, { "A38",  73.0f },
   { "E17",  30.0f }, { "E19",  38.0f }, { "E75",  32.0f },
   { "CRJ",  32.0f }, { "DH8",  28.0f }, { "AT7",  27.0f }, { "GLF",  29.0f },
   { "BCS",  37.0f }
};

static void Str_Upper (char* s)
{
   for (; *s; s++)
      if (*s >= 'a'  &&  *s <= 'z')
         *s = (char) (*s - 32);
}

static void Str_Lower (char* s)
{
   for (; *s; s++)
      if (*s >= 'A'  &&  *s <= 'Z')
         *s = (char) (*s + 32);
}

static double Type_Length (const char* sType)
{
   int i;
   int nType;

   if (sType == 0  ||  sType[0] == 0)
      return 0.0;

   nType = (int) strlen (sType);

   for (i = 0; i < (int) (sizeof (s_aTypeLen) / sizeof (s_aTypeLen[0])); i++)
   {
      if (strcmp (sType, s_aTypeLen[i].sCode) == 0)
         return (double) s_aTypeLen[i].fLen;
   }

   for (i = 0; i < (int) (sizeof (s_aTypeLen) / sizeof (s_aTypeLen[0])); i++)
   {
      int nCode = (int) strlen (s_aTypeLen[i].sCode);

      if (nCode >= 3  &&  nType >= nCode  &&  strncmp (sType, s_aTypeLen[i].sCode, (size_t) nCode) == 0)
         return (double) s_aTypeLen[i].fLen;
   }

   return 0.0;
}

static double Class_Length (const char* sClass)
{
   if (sClass == 0  ||  sClass[0] == 0)
      return 0.0;
   if (sClass[0] == 'L'  &&  sClass[2] == 'J')
   {
      if (sClass[1] == '4') return 70.0;
      if (sClass[1] == '3') return 60.0;
      if (sClass[1] == '2') return 42.0;
      if (sClass[1] == '1') return 18.0;
   }
   if (sClass[0] == 'L'  &&  sClass[2] == 'T')
      return (sClass[1] == '4'  ||  sClass[1] == '2') ? 28.0 : 14.0;
   if (sClass[0] == 'L'  &&  sClass[2] == 'P')
      return 10.0;
   if (sClass[0] == 'H')
      return 16.0;
   return 0.0;
}

static double Craft_World_Scale (const CRAFT* p)
{
   double dLen = p->dLengthM;

   if (dLen < 5.0)
      dLen = 5.0;
   return dLen;
}

static void Craft_Apply_Scale (CRAFT* p)
{
   if (p != 0  &&  Node_IsValid (p->qwIcon))
      Node_Scale (p->qwIcon, Craft_World_Scale (p));
}

static int Craft_Find_Icao (const char* sIcao)
{
   int i;

   if (sIcao == 0  ||  sIcao[0] == 0)
      return -1;

   for (i = 0; i < s_nCraft; i++)
   {
      if (strcmp (s_aCraft[i].sIcao, sIcao) == 0)
         return i;
   }

   return -1;
}

// The icon's top sits about a quarter of its length above the node. dAbove
// is further clearance. The panel is centered, so half its height is added
// and the card's bottom clears the airframe.
static double Label_Lift (const CRAFT* p)
{
   double dLen = (p != 0  &&  p->dLengthM > 0.0) ? p->dLengthM : 0.0;

   return s_dLabelAbove + 0.25 * dLen + s_dLabelH * s_dLabelSY * 0.5;
}

static void Craft_Update_Pose (CRAFT* p)
{
   double dX, dY, dZ;
   double aQ[4];
   char   aName[48];

   if (p == 0  ||  !Node_IsValid (p->qwIcon))
      return;

   Geodetic_To_Local (p->dLat, p->dLon, p->dAltM, &dX, &dY, &dZ);
   Heading_Quat (p->dHeading, aQ);
   Craft_Label (p, aName, (int) sizeof (aName));

   Node_Position (p->qwIcon, dX, dY, dZ);
   Node_Rotation (p->qwIcon, aQ[0], aQ[1], aQ[2], aQ[3]);
   Node_Name     (p->qwIcon, aName);

   if (Node_IsValid (p->qwLabel))
      Node_Position (p->qwLabel, dX, dY, dZ + Label_Lift (p));
}

static double Lerp (double dA, double dB, double dT)
{
   return dA + (dB - dA) * dT;
}

// Ground-track degrees clockwise from north (+Y), or -1 if the span is too small.
static double Course_Deg (double dLat0, double dLon0, double dLat1, double dLon1)
{
   double dX0, dY0, dZ0;
   double dX1, dY1, dZ1;
   double dEast;
   double dNorth;
   double dH;
   double dDeg;

   Geodetic_To_Local (dLat0, dLon0, 0.0, &dX0, &dY0, &dZ0);
   Geodetic_To_Local (dLat1, dLon1, 0.0, &dX1, &dY1, &dZ1);
   dEast  = dX1 - dX0;
   dNorth = dY1 - dY0;
   dH     = sqrt (dEast * dEast + dNorth * dNorth);
   if (dH < 0.5)
      return -1.0;

   dDeg = atan2 (dEast, dNorth) * 180.0 / PI;
   if (dDeg < 0.0)
      dDeg += 360.0;
   if (dDeg >= 360.0)
      dDeg -= 360.0;
   return dDeg;
}

static void Craft_Heading_Seg (CRAFT* p, int nA, int nB)
{
   double dCourse;

   if (p == 0  ||  nA < 0  ||  nB < 0  ||  nA >= p->nWp  ||  nB >= p->nWp)
      return;

   dCourse = Course_Deg (p->aWp[nA].dLat, p->aWp[nA].dLon,
                         p->aWp[nB].dLat, p->aWp[nB].dLon);
   if (dCourse >= 0.0)
      p->dHeading = dCourse;
   else
      p->dHeading = p->aWp[nB].dHeading;
}

static void Craft_Sit (CRAFT* p, int nIz)
{
   if (p == 0  ||  nIz < 0  ||  nIz >= p->nWp)
      return;

   p->dLat  = p->aWp[nIz].dLat;
   p->dLon  = p->aWp[nIz].dLon;
   p->dAltM = p->aWp[nIz].dAltM;
   if (nIz > 0)
      Craft_Heading_Seg (p, nIz - 1, nIz);
   else
      p->dHeading = p->aWp[nIz].dHeading;
   Craft_Update_Pose (p);
}

static void Craft_Wp_Push (CRAFT* p, double dLat, double dLon, double dAlt,
                           double dHeading, int64_t tmMs)
{
   WAYPOINT* q;

   if (p == 0)
      return;

   if (tmMs <= 0  &&  s_pHost != 0)
      tmMs = Chrono_Date (s_pHost);

   if (p->nWp > 0)
   {
      q = &p->aWp[p->nWp - 1];
      if (fabs (q->dLat - dLat) < 1e-6
       &&  fabs (q->dLon - dLon) < 1e-6
       &&  fabs (q->dAltM - dAlt) < 1.0)
         return;
      if (tmMs > 0  &&  tmMs < q->tmMs)
         tmMs = q->tmMs + WP_MIN_MS;
   }

   if (p->nWp >= MAX_WAYPOINTS)
   {
      memmove (&p->aWp[0], &p->aWp[1],
         (size_t) (MAX_WAYPOINTS - 1) * sizeof (WAYPOINT));
      p->nWp = MAX_WAYPOINTS - 1;
   }

   q = &p->aWp[p->nWp];
   q->dLat     = dLat;
   q->dLon     = dLon;
   q->dAltM    = dAlt;
   q->dHeading = dHeading;
   q->tmMs     = tmMs;
   if (p->nWp >= 1)
   {
      double dCourse = Course_Deg (p->aWp[p->nWp - 1].dLat, p->aWp[p->nWp - 1].dLon,
                                   dLat, dLon);
      if (dCourse >= 0.0)
         q->dHeading = dCourse;
   }
   p->nWp++;

   if (p->nWp == 1)
   {
      p->dLat     = dLat;
      p->dLon     = dLon;
      p->dAltM    = dAlt;
      p->dHeading = dHeading;
      Craft_Update_Pose (p);
   }
}

static void Craft_Animate (CRAFT* p, int64_t tmNow)
{
   int    i;
   int    nB;
   double dT;
   int64_t nMs;

   if (p == 0  ||  p->nWp <= 0)
      return;

   if (p->nWp == 1  ||  tmNow <= p->aWp[0].tmMs)
   {
      Craft_Sit (p, 0);
      return;
   }

   if (tmNow >= p->aWp[p->nWp - 1].tmMs)
   {
      Craft_Sit (p, p->nWp - 1);
      return;
   }

   for (i = 0; i < p->nWp - 1; i++)
   {
      nB = i + 1;
      if (tmNow > p->aWp[nB].tmMs)
         continue;

      nMs = p->aWp[nB].tmMs - p->aWp[i].tmMs;
      dT  = (nMs > 0) ? ((double) (tmNow - p->aWp[i].tmMs) / (double) nMs) : 1.0;
      if (dT < 0.0) dT = 0.0;
      if (dT > 1.0) dT = 1.0;

      p->dLat  = Lerp (p->aWp[i].dLat,  p->aWp[nB].dLat,  dT);
      p->dLon  = Lerp (p->aWp[i].dLon,  p->aWp[nB].dLon,  dT);
      p->dAltM = Lerp (p->aWp[i].dAltM, p->aWp[nB].dAltM, dT);
      Craft_Heading_Seg (p, i, nB);
      Craft_Update_Pose (p);
      return;
   }

   Craft_Sit (p, p->nWp - 1);
}

static int64_t Clock_Play (int64_t tmNow)
{
   int64_t nLagMs = (int64_t) s_nLagSec * 1000 + LAG_PAD_MS;

   if (tmNow > nLagMs)
      return tmNow - nLagMs;
   return 0;
}

static void Trail_Span (int64_t* ptmTail, int64_t* ptmFront)
{
   int     i;
   bool    b      = false;
   int64_t tmTail = 0;
   int64_t tmFront = 0;

   for (i = 0; i < s_nCraft; i++)
   {
      const CRAFT* p = &s_aCraft[i];
      int64_t tm0;
      int64_t tm1;

      if (p->nWp < 2)
         continue;

      tm0 = p->aWp[0].tmMs;
      tm1 = p->aWp[p->nWp - 1].tmMs;
      if (!b)
      {
         tmTail  = tm0;
         tmFront = tm1;
         b       = true;
      }
      else
      {
         if (tm0 < tmTail)
            tmTail = tm0;
         if (tm1 > tmFront)
            tmFront = tm1;
      }
   }

   if (ptmTail)
      *ptmTail = b ? tmTail : 0;
   if (ptmFront)
      *ptmFront = b ? tmFront : 0;
}

static int Clock_Lag_From_Play (int64_t tmNow, int64_t tmPlay, int nLagMax)
{
   int nLag;

   if (tmNow <= tmPlay + LAG_PAD_MS)
      return 1;

   nLag = (int) ((tmNow - tmPlay - LAG_PAD_MS) / 1000);
   if (nLag < 1)
      nLag = 1;
   if (nLag > nLagMax)
      nLag = nLagMax;
   return nLag;
}

static int64_t Clock_Play_Sync (int64_t tmNow)
{
   int     nGuard  = 0;
   int     nLag0   = s_nLagSec;
   int     nLagMax;
   int64_t tmPlay  = Clock_Play (tmNow);
   int64_t tmTail  = 0;
   int64_t tmFront = 0;

   Trail_Span (&tmTail, &tmFront);
   if (tmFront <= 0)
      return tmPlay;

   nLagMax = s_nPollSec * MAX_WAYPOINTS;
   if (nLagMax < 60)
      nLagMax = 60;
   if (nLagMax > 600)
      nLagMax = 600;

   while (tmPlay >= tmFront  &&  nGuard < MAX_WAYPOINTS)
   {
      int nLag = s_nLagSec + s_nPollSec;

      if (nLag > nLagMax)
         nLag = nLagMax;

      if (nLag > s_nLagSec)
      {
         s_nLagSec = nLag;
         tmPlay    = Clock_Play (tmNow);
         if (tmTail > 0  &&  tmPlay < tmTail)
         {
            tmPlay    = tmTail;
            s_nLagSec = Clock_Lag_From_Play (tmNow, tmPlay, nLagMax);
            break;
         }
      }
      else
      {
         if (tmTail > 0)
         {
            tmPlay    = tmTail;
            s_nLagSec = Clock_Lag_From_Play (tmNow, tmPlay, nLagMax);
         }
         break;
      }

      nGuard++;
   }

   if (s_nLagSec != nLag0  &&  s_pHost != 0)
   {
      char aLog[96];
      snprintf (aLog, sizeof (aLog),
         "dfw-adsbfi: playhead caught front, lag %d s -> %d s",
         nLag0, s_nLagSec);
      Console_Log (s_pHost, aLog);
   }

   return tmPlay;
}

static void Craft_Remove_At (int nIz)
{
   if (nIz < 0  ||  nIz >= s_nCraft)
      return;

   if (Node_IsValid (s_aCraft[nIz].qwLabel))
   {
      if (!Fabric_Node_Close (s_pHost, s_aCraft[nIz].qwLabel))
         Node_Bound (s_aCraft[nIz].qwLabel, 0.0, 0.0, 0.0);
      Scene_Hold (HOLD_TICKS_CLOSE);
   }

   if (Node_IsValid (s_aCraft[nIz].qwIcon))
   {
      Fabric_Node_Close (s_pHost, s_aCraft[nIz].qwIcon);
      Scene_Hold (HOLD_TICKS_CLOSE);
   }

   if (nIz < s_nCraft - 1)
      memmove (&s_aCraft[nIz], &s_aCraft[nIz + 1],
         (size_t) (s_nCraft - 1 - nIz) * sizeof (CRAFT));

   s_nCraft--;
}

static int Crafts_Need_Icon (void)
{
   int i;
   int n = 0;

   for (i = 0; i < s_nCraft; i++)
   {
      if (!s_aCraft[i].bTried)
         n++;
   }

   return n;
}

static void Scene_Hold (int nTicks)
{
   if (nTicks > s_nHoldTicks)
      s_nHoldTicks = nTicks;
}

static void Xml_Escape (const char* sIn, char* sOut, int nCap)
{
   int n = 0;

   for (; *sIn  &&  n + 6 < nCap; sIn++)
   {
      if (*sIn == '<')      { memcpy (sOut + n, "&lt;",  4); n += 4; }
      else if (*sIn == '>') { memcpy (sOut + n, "&gt;",  4); n += 4; }
      else if (*sIn == '&') { memcpy (sOut + n, "&amp;", 5); n += 5; }
      else                  { sOut[n++] = *sIn; }
   }
   sOut[n] = 0;
}

static void Craft_Label (const CRAFT* p, char* sOut, int nCap)
{
   if (p->bGround)
      snprintf (sOut, (size_t) nCap, "%s HDG %03.0f GND", p->sId, p->dHeading);
   else
      snprintf (sOut, (size_t) nCap, "%s HDG %03.0f %.0f ft",
         p->sId, p->dHeading, p->dAltM / METRES_PER_FOOT);
}

// Identification on the first line, type and speed on the second. Speed is
// bucketed to 5 kt so a jittering ground-speed does not rebuild the card.
static void Craft_Sign (const CRAFT* p, char* sOut, int nCap)
{
   char        aDetail[48];
   const char* sName = (p->sId[0] != 0) ? p->sId : p->sIcao;

   aDetail[0] = 0;

   if (p->dGsKt >= 0.0)
   {
      int nKt = (int) (p->dGsKt / 5.0 + 0.5) * 5;

      if (p->bGround)
      {
         if (p->sType[0] != 0)
            snprintf (aDetail, sizeof (aDetail), "%s  GND  %d kt", p->sType, nKt);
         else
            snprintf (aDetail, sizeof (aDetail), "GND  %d kt", nKt);
      }
      else if (p->sType[0] != 0)
         snprintf (aDetail, sizeof (aDetail), "%s  %d kt", p->sType, nKt);
      else
         snprintf (aDetail, sizeof (aDetail), "%d kt", nKt);
   }
   else if (p->bGround)
   {
      if (p->sType[0] != 0)
         snprintf (aDetail, sizeof (aDetail), "%s  GND", p->sType);
      else
         snprintf (aDetail, sizeof (aDetail), "GND");
   }
   else if (p->sType[0] != 0)
      snprintf (aDetail, sizeof (aDetail), "%s", p->sType);

   snprintf (sOut, (size_t) nCap, "%s\n%s", sName, aDetail);
}

static void Label_Document (const char* sSign, char* sOut, int nCap)
{
   char        aId[40];
   char        aDetail[48];
   char        aEscId[64];
   char        aEscDetail[80];
   const char* sNl = strchr (sSign, '\n');
   int         nId;

   aId[0]     = 0;
   aDetail[0] = 0;

   if (sNl != 0)
   {
      nId = (int) (sNl - sSign);
      if (nId >= (int) sizeof (aId))
         nId = (int) sizeof (aId) - 1;
      if (nId < 0)
         nId = 0;
      memcpy (aId, sSign, (size_t) nId);
      aId[nId] = 0;
      strncpy (aDetail, sNl + 1, sizeof (aDetail) - 1);
      aDetail[sizeof (aDetail) - 1] = 0;
   }
   else
   {
      strncpy (aId, sSign, sizeof (aId) - 1);
      aId[sizeof (aId) - 1] = 0;
   }

   Xml_Escape (aId, aEscId, (int) sizeof (aEscId));
   Xml_Escape (aDetail, aEscDetail, (int) sizeof (aEscDetail));

   snprintf (sOut, (size_t) nCap,
      "<rml><head><style>"
      "body { display:block; width:100%%; height:100%%; margin:0; font-family:Inter;"
      "  color:#e9eef6; background-color:transparent; }"
      "#card { display:block; position:absolute; left:1%%; top:6%%; width:98%%; height:88%%;"
      "  box-sizing:border-box; background-color:rgba(8,16,28,226);"
      "  border-width:2px; border-color:#3d8fd4; border-radius:16px; }"
      "#id { display:block; position:absolute; left:0%%; top:12%%; width:100%%;"
      "  text-align:center; font-size:36dp; font-weight:600; color:#d7ecff; }"
      "#meta { display:block; position:absolute; left:0%%; top:54%%; width:100%%;"
      "  text-align:center; font-size:24dp; color:#9fd0ff; }"
      "</style></head><body><div id='card'>"
      "<div id='id'>%s</div><div id='meta'>%s</div>"
      "</div></body></rml>",
      aEscId, aEscDetail);
}

// Panel controls sit in the 32-byte properties region: u16 width, u16 height,
// u8 scale mode, u8 billboard, u8 density. The C SDK writes that region as
// two floats, so the integers go in through the float bits. Same packing as
// the clock.
static void Label_Configure (HMAPOBJECT pPanel)
{
   uint32_t nRaster = ((uint32_t) s_nLabelPixW & 0xffffu)
                    | (((uint32_t) s_nLabelPixH & 0xffffu) << 16);
   uint32_t nMode   = ((uint32_t) s_nLabelScale & 0xffu)
                    | (((uint32_t) s_nLabelBill & 0xffu) << 8)
                    | (((uint32_t) s_nLabelDen & 0xffu) << 16);
   float    fRaster;
   float    fMode;

   memcpy (&fRaster, &nRaster, sizeof (fRaster));
   memcpy (&fMode, &nMode, sizeof (fMode));
   MapObject_Properties_Celestial (pPanel, fRaster, fMode, 0, 0.0f, 0.0f);
}

// Opens with a zero bound so the host does not rasterize before the document
// exists, then sets the RML, then the real metre bound. A live rewrite of RML
// closes the document on the timer thread and aborts the host.
static HNODE Label_Open (const CRAFT* p, const char* sSign)
{
   HMAPOBJECT pPanel;
   HNODE      qw;
   double     dX, dY, dZ;
   char       aDoc[1536];
   char       aName[40];
   const char* sNl;
   int        nName;

   if (p == 0  ||  s_pHost == 0)
      return 0;

   Label_Document (sSign, aDoc, (int) sizeof (aDoc));
   Geodetic_To_Local (p->dLat, p->dLon, p->dAltM, &dX, &dY, &dZ);
   dZ += Label_Lift (p);

   aName[0] = 0;
   sNl = strchr (sSign, '\n');
   nName = (sNl != 0) ? (int) (sNl - sSign) : (int) strlen (sSign);
   if (nName >= (int) sizeof (aName))
      nName = (int) sizeof (aName) - 1;
   if (nName < 0)
      nName = 0;
   memcpy (aName, sSign, (size_t) nName);
   aName[nName] = 0;
   if (aName[0] == 0)
      strncpy (aName, "Aircraft", sizeof (aName) - 1);

   pPanel = MapObject_Panel ();
   MapObject_Parent   (pPanel, s_wRootClass, s_twRootIx);
   MapObject_Name     (pPanel, aName);
   MapObject_Position  (pPanel, dX, dY, dZ);
   MapObject_Scale_Axes (pPanel, s_dLabelSX, s_dLabelSY, 1.0);
   MapObject_Bound     (pPanel, 0.0, 0.0, 0.0);
   Label_Configure (pPanel);

   qw = Fabric_Node_Open (s_pHost, pPanel);
   MapObject_Free (pPanel);

   if (!Node_IsValid (qw))
      return 0;

   Node_Panel (qw, aDoc);
   Node_Bound (qw, s_dLabelW, s_dLabelH, 0.0);

   if (!s_bLabelLogged)
   {
      char aLog[128];

      snprintf (aLog, sizeof (aLog),
         "dfw-adsbfi: label %.0f x %.0f m billboard, %.0f m above the icon",
         s_dLabelW, s_dLabelH, s_dLabelAbove);
      Console_Log (s_pHost, aLog);
      s_bLabelLogged = true;
   }

   return qw;
}

static void Label_Service (void)
{
   HNODE aOld[LABELS_PER_TICK];
   int   nOld    = 0;
   int   nOpened = 0;
   int   i;

   if (s_pHost == 0)
      return;

   for (i = 0; i < s_nCraft  &&  nOpened < LABELS_PER_TICK; i++)
   {
      CRAFT* p = &s_aCraft[i];
      char   aSign[80];
      HNODE  qwNew;

      if (!Node_IsValid (p->qwIcon))
         continue;

      Craft_Sign (p, aSign, (int) sizeof (aSign));
      if (Node_IsValid (p->qwLabel)  &&  strcmp (p->sSign, aSign) == 0)
         continue;

      qwNew = Label_Open (p, aSign);
      if (!Node_IsValid (qwNew))
         continue;

      if (Node_IsValid (p->qwLabel)  &&  nOld < LABELS_PER_TICK)
         aOld[nOld++] = p->qwLabel;

      p->qwLabel = qwNew;
      strncpy (p->sSign, aSign, sizeof (p->sSign) - 1);
      p->sSign[sizeof (p->sSign) - 1] = 0;
      nOpened++;
   }

   for (i = 0; i < nOld; i++)
   {
      if (!Fabric_Node_Close (s_pHost, aOld[i]))
         Node_Bound (aOld[i], 0.0, 0.0, 0.0);
   }

   if (nOld > 0)
      Scene_Hold (HOLD_TICKS_CLOSE);
}

static void Scene_Origin (void)
{
   HMAPOBJECT pPad = MapObject_Physical ();

   MapObject_Parent    (pPad, s_wRootClass, s_twRootIx);
   MapObject_Name      (pPad, "DFW");
   MapObject_Reference (pPad, "res/origin.glb");
   MapObject_Position  (pPad, 0.0, 0.0, 0.0);
   MapObject_Scale     (pPad, ORIGIN_PAD_METRES);

   s_qwPad = Fabric_Node_Open (s_pHost, pPad);
   MapObject_Free (pPad);
   s_bPad  = true;
}

static void Craft_Spawn (int nIz)
{
   CRAFT*     p;
   HMAPOBJECT pIcon;
   double     dX, dY, dZ;
   double     aQ[4];
   char       aName[48];

   if (nIz < 0  ||  nIz >= s_nCraft)
      return;

   p = &s_aCraft[nIz];
   if (p->bTried  ||  Node_IsValid (p->qwIcon))
      return;

   p->bTried = true;
   pIcon = MapObject_Physical ();

   Geodetic_To_Local (p->dLat, p->dLon, p->dAltM, &dX, &dY, &dZ);
   Heading_Quat (p->dHeading, aQ);
   Craft_Label (p, aName, (int) sizeof (aName));

   MapObject_Parent    (pIcon, s_wRootClass, s_twRootIx);
   MapObject_Name      (pIcon, aName);
   MapObject_Reference (pIcon, "res/aircraft.glb");
   MapObject_Position  (pIcon, dX, dY, dZ);
   MapObject_Rotation  (pIcon, aQ[0], aQ[1], aQ[2], aQ[3]);
   MapObject_Scale     (pIcon, Craft_World_Scale (p));

   p->qwIcon = Fabric_Node_Open (s_pHost, pIcon);
   MapObject_Free (pIcon);
}

static void Pad_Close (void)
{
   if (Node_IsValid (s_qwPad))
      Fabric_Node_Close (s_pHost, s_qwPad);

   s_qwPad = 0;
   s_bPad  = false;
}

static void Legend_Close (void)
{
   if (Node_IsValid (s_qwLegend))
   {
      Fabric_Node_Close (s_pHost, s_qwLegend);
      Scene_Hold (HOLD_TICKS_CLOSE);
   }

   s_qwLegend = 0;
   s_bLegend  = false;
}

// One UnlitBlend card, RML set in the same turn as Fabric_Node_Open and never
// rewritten. The compositor must not see this node until after the aircraft
// GLBs have already been committed.
static void Legend_Open (void)
{
   HMAPOBJECT pPanel;
   char       aDoc[5120];
   char       aRows[2800];
   char       aTitle[80];
   char       aEscTitle[96];
   int        nRows = s_nCraft < LEGEND_ROWS ? s_nCraft : LEGEND_ROWS;
   int        nOff  = 0;
   int        i;

   if (s_bLegend  ||  Node_IsValid (s_qwLegend))
      return;

   aRows[0] = 0;

   if (s_nCraft <= 0)
   {
      snprintf (aRows, sizeof (aRows),
         "<span class='row'>No aircraft in range</span>");
   }
   else
   {
      for (i = 0; i < nRows; i++)
      {
         char aLine[64];
         char aEsc[80];

         Craft_Label (&s_aCraft[i], aLine, (int) sizeof (aLine));
         Xml_Escape (aLine, aEsc, (int) sizeof (aEsc));
         nOff += snprintf (aRows + nOff, sizeof (aRows) - (size_t) nOff,
            "<span class='row'>%s</span>", aEsc);
         if (nOff < 0  ||  nOff >= (int) sizeof (aRows))
            break;
      }
   }

   snprintf (aTitle, sizeof (aTitle), "DFW ADSB.fi - %d aircraft", s_nCraft);
   Xml_Escape (aTitle, aEscTitle, (int) sizeof (aEscTitle));

   // The engine billboard quad is a fixed fraction of the framed view. Node
   // Scale does not change it. dScale sizes the painted card inside that quad;
   // the rest of the 512 texture stays transparent.
   {
      int  nPct   = (int) (s_dPanelScale * 100.0 + 0.5);
      int  nTitle;
      int  nBtn;
      int  nRow;
      int  nNote;
      int  nPad;
      char aLog[112];

      if (nPct < 12)
         nPct = 12;
      if (nPct > 92)
         nPct = 92;

      nTitle = 17 * nPct / 90;
      nBtn   = 12 * nPct / 90;
      nRow   = 12 * nPct / 90;
      nNote  =  9 * nPct / 90;
      nPad   = 12 * nPct / 90;
      if (nTitle < 9) nTitle = 9;
      if (nBtn   < 8) nBtn   = 8;
      if (nRow   < 8) nRow   = 8;
      if (nNote  < 6) nNote  = 6;
      if (nPad   < 5) nPad   = 5;

      snprintf (aDoc, sizeof (aDoc),
         "<rml><head><style>"
         "body { display:block; width:100%%; height:100%%; font-family:Inter; color:#e9eef6;"
         "  background-color:transparent; }"
         "#card { display:block; position:absolute; left:4%%; top:4%%; width:%d%%; height:%d%%;"
         "  box-sizing:border-box; padding:%dpx %dpx; overflow:hidden;"
         "  background-color:rgba(20,22,28,232);"
         "  border-width:1px; border-color:rgba(120,190,255,60); border-radius:16px; }"
         ".title { display:block; font-size:%dpx; font-weight:600; color:#d7ecff; margin:0 0 4px 0; }"
         ".btn { display:block; margin:0 0 5px 0; padding:4px 0; text-align:center;"
         "  color:#dbe2ec; font-size:%dpx; font-weight:500;"
         "  background-color:rgba(120,190,255,36);"
         "  border-width:1px; border-color:rgba(120,190,255,80); border-radius:10px; }"
         ".row { display:block; font-size:%dpx; color:#c4ccd8; margin:0 0 1px 0; }"
         ".note { display:block; font-size:%dpx; color:#8b94a3; margin:4px 0 0 0; }"
         "</style></head><body><div id='card'>"
         "<span class='title'>%s</span>"
         "<div class='btn'>1 toggles panel · other digits refresh</div>"
         "%s"
         "<span class='note'>click the 3D view first, then a digit</span>"
         "</div></body></rml>",
         nPct, nPct, nPad, nPad, nTitle, nBtn, nRow, nNote, aEscTitle, aRows);

      snprintf (aLog, sizeof (aLog),
         "dfw-adsbfi: legend card %d%% of fixed billboard (dScale %.4f)",
         nPct, s_dPanelScale);
      Console_Log (s_pHost, aLog);
   }

   pPanel = MapObject_Panel ();
   MapObject_Parent   (pPanel, s_wRootClass, s_twRootIx);
   MapObject_Name     (pPanel, "Legend");
   MapObject_Position (pPanel, s_dPanelX, s_dPanelY, s_dPanelZ);
   MapObject_Bound    (pPanel, s_dPanelW, s_dPanelH, 0.0);
   MapObject_Scale    (pPanel, 1.0);

   s_qwLegend = Fabric_Node_Open (s_pHost, pPanel);
   MapObject_Free (pPanel);

   if (Node_IsValid (s_qwLegend))
      Node_Panel (s_qwLegend, aDoc);

   s_bLegend = true;
}

static void Legend_Toggle (void)
{
   char aLog[128];

   snprintf (aLog, sizeof (aLog),
      "dfw-adsbfi: toggle triggered  wanted=%d  live=%d  node=%s",
      s_bPanelWanted ? 1 : 0,
      s_bLegend ? 1 : 0,
      Node_IsValid (s_qwLegend) ? "valid" : "none");
   Console_Warn (s_pHost, aLog);

   s_bPanelWanted = !s_bPanelWanted;

   if (s_bPanelWanted)
   {
      Console_Warn (s_pHost, "dfw-adsbfi: panel ON");
      Legend_Open ();
   }
   else
   {
      Console_Warn (s_pHost, "dfw-adsbfi: panel OFF");
      Legend_Close ();
   }
}

static void Scene_Clear_Crafts (void)
{
   int i;

   for (i = s_nCraft - 1; i >= 0; i--)
   {
      if (Node_IsValid (s_aCraft[i].qwLabel))
      {
         if (!Fabric_Node_Close (s_pHost, s_aCraft[i].qwLabel))
            Node_Bound (s_aCraft[i].qwLabel, 0.0, 0.0, 0.0);
         Scene_Hold (HOLD_TICKS_CLOSE);
      }
      s_aCraft[i].qwLabel = 0;
      s_aCraft[i].sSign[0] = 0;

      if (Node_IsValid (s_aCraft[i].qwIcon))
      {
         Fabric_Node_Close (s_pHost, s_aCraft[i].qwIcon);
         Scene_Hold (HOLD_TICKS_CLOSE);
      }
      s_aCraft[i].qwIcon = 0;
   }

   s_nCraft = 0;
}

static void Craft_Set_Type (CRAFT* p, const char* sType)
{
   double dLen;

   if (p == 0  ||  sType == 0  ||  sType[0] == 0)
      return;

   strncpy (p->sType, sType, sizeof (p->sType) - 1);
   p->sType[sizeof (p->sType) - 1] = 0;

   dLen = Type_Length (sType);
   if (dLen >= 5.0  &&  dLen <= 90.0)
   {
      p->dLengthM = dLen;
      Craft_Apply_Scale (p);
   }
}

static void Timer_Ensure (void)
{
   if (s_twTimer == 0  &&  s_pHost != 0)
      s_twTimer = Timer_Interval (s_pHost, SPAWN_HZ, kSNEEZE_ABI_TIMER_UNIT_HZ, 0);
}

static void Traffic_Refresh (void)
{
   if (s_pHost == 0)
      return;

   if (s_hRequest != 0)
   {
      Request_Abort (s_hRequest);
      Request_Close (s_hRequest);
      s_hRequest = 0;
   }

   s_nPollTick = 0;
   Fetch_ADSB ();
   Timer_Ensure ();
}

static bool Ac_Read (JSON* p, char* sIcao, int nIcao, char* sCall, int nCall,
                    char* sType, int nType, double* pdLat, double* pdLon,
                    double* pdAltFt, double* pdTrack, double* pdSeen, double* pdGs,
                    bool* pbLat, bool* pbLon, bool* pbAlt, bool* pbTrack,
                    bool* pbSeen, bool* pbGround, bool* pbGs)
{
   if (!Json_Take (p, '{'))
      return false;

   while (Json_Peek (p) != 0  &&  Json_Peek (p) != '}')
   {
      char        aKey[32] = { 0 };
      const char* sField   = p->s;

      if (!Json_String (p, aKey, (int) sizeof (aKey))  ||  !Json_Take (p, ':'))
      {
         Json_Bump (p, sField);
         Json_Take (p, ',');
         if (p->s == sField)
            break;
         continue;
      }

      if (strcmp (aKey, "hex") == 0)
         Json_String (p, sIcao, nIcao);
      else if (strcmp (aKey, "flight") == 0)
         Json_String (p, sCall, nCall);
      else if (strcmp (aKey, "t") == 0)
         Json_String (p, sType, nType);
      else if (strcmp (aKey, "lat") == 0)
         *pbLat = Json_Number (p, pdLat);
      else if (strcmp (aKey, "lon") == 0)
         *pbLon = Json_Number (p, pdLon);
      else if (strcmp (aKey, "alt_geom") == 0)
      {
         double dFt = 0.0;
         if (Json_Number (p, &dFt))
         {
            *pdAltFt = dFt;
            *pbAlt   = true;
         }
      }
      else if (strcmp (aKey, "alt_baro") == 0)
      {
         if (Json_Peek (p) == '"')
         {
            char aAlt[16] = { 0 };
            Json_String (p, aAlt, (int) sizeof (aAlt));
            if (strcmp (aAlt, "ground") == 0)
            {
               *pbGround = true;
               if (!*pbAlt)
               {
                  *pdAltFt = 0.0;
                  *pbAlt   = true;
               }
            }
         }
         else
         {
            double dFt = 0.0;
            if (Json_Number (p, &dFt)  &&  !*pbAlt)
            {
               *pdAltFt = dFt;
               *pbAlt   = true;
            }
         }
      }
      else if (strcmp (aKey, "track") == 0)
         *pbTrack = Json_Number (p, pdTrack);
      else if (strcmp (aKey, "gs") == 0)
         *pbGs = Json_Number (p, pdGs);
      else if (strcmp (aKey, "true_heading") == 0)
      {
         double dHdg = 0.0;
         if (Json_Number (p, &dHdg)  &&  !*pbTrack)
         {
            *pdTrack = dHdg;
            *pbTrack = true;
         }
      }
      else if (strcmp (aKey, "seen_pos") == 0)
         *pbSeen = Json_Number (p, pdSeen);
      else
         Json_SkipValue (p);

      Json_Bump (p, sField);
      Json_Take (p, ',');
      if (p->s == sField)
         break;
   }

   Json_Take (p, '}');
   return true;
}

static int Ac_Load (const char* sJson, int nLen)
{
   JSON   j;
   JSON   jNow;
   int    nPlaced = 0;
   int    nMoved  = 0;
   int    nGone   = 0;
   int    nSeen   = 0;
   int    i;
   char   aLog[192];
   double dRadiusM = s_dRadiusMi * METRES_PER_MILE;
   double dNow     = 0.0;
   bool   bNow     = false;
   bool   bParsed  = false;
   int64_t tmNowMs;

   for (i = 0; i < s_nCraft; i++)
      s_aCraft[i].bLive = false;

   j.s    = sJson;
   j.sEnd = sJson + nLen;
   jNow   = j;

   if (Json_FindKey (&jNow, "now"))
      bNow = Json_Number (&jNow, &dNow);

   if (bNow  &&  dNow > 1.0e12)
      tmNowMs = (int64_t) dNow;
   else if (bNow  &&  dNow > 1.0e9)
      tmNowMs = (int64_t) (dNow * 1000.0);
   else
      tmNowMs = Chrono_Date (s_pHost);

   if (!Json_FindKey (&j, "ac"))
   {
      Console_Error (s_pHost, "dfw-adsbfi: no ac array");
      return 0;
   }

   if (Json_IsNull (&j))
   {
      bParsed = true;
   }
   else if (!Json_Take (&j, '['))
   {
      Console_Error (s_pHost, "dfw-adsbfi: ac is not an array");
      return 0;
   }
   else
   {
      bParsed = true;

      if (!Json_Take (&j, ']'))
      {
         while (Json_Peek (&j) != 0  &&  Json_Peek (&j) != ']'  &&  nSeen < MAX_STATE_ROWS)
         {
            char   aIcao[16] = { 0 };
            char   aCall[32] = { 0 };
            char   aType[12] = { 0 };
            double dLon = 0, dLat = 0, dAltFt = 0, dTrack = 0, dSeenPos = 0, dGs = 0;
            bool   bLon = false, bLat = false, bAlt = false, bTrack = false;
            bool   bSeenPos = false, bGround = false, bGs = false;
            int    nIz;
            const char* sRow = j.s;

            nSeen++;

            if (!Ac_Read (&j, aIcao, (int) sizeof (aIcao), aCall, (int) sizeof (aCall),
                          aType, (int) sizeof (aType), &dLat, &dLon, &dAltFt, &dTrack,
                          &dSeenPos, &dGs, &bLat, &bLon, &bAlt, &bTrack, &bSeenPos,
                          &bGround, &bGs))
            {
               Json_Bump (&j, sRow);
               Json_Take (&j, ',');
               if (j.s == sRow)
                  break;
               continue;
            }

            Json_Take (&j, ',');
            if (j.s == sRow)
               break;

            Trim (aIcao);
            Trim (aCall);
            Trim (aType);
            Str_Lower (aIcao);
            Str_Upper (aType);

            if (!bLon  ||  !bLat  ||  aIcao[0] == 0)
               continue;
            if (aIcao[0] == '~')
               continue;
            if (Distance_M (dLat, dLon) > dRadiusM)
               continue;

            nIz = Craft_Find_Icao (aIcao);
            if (nIz < 0)
            {
               if (s_nCraft >= MAX_AIRCRAFT)
                  continue;

               nIz = s_nCraft;
               memset (&s_aCraft[nIz], 0, sizeof (CRAFT));
               strncpy (s_aCraft[nIz].sIcao, aIcao, sizeof (s_aCraft[nIz].sIcao) - 1);
               s_aCraft[nIz].dLengthM = s_dIconLen;
               s_aCraft[nIz].dGsKt    = -1.0;
               s_nCraft++;
               nPlaced++;
            }
            else
            {
               nMoved++;
            }

            {
               CRAFT*      p    = &s_aCraft[nIz];
               const char* sId  = (aCall[0] != 0) ? aCall : aIcao;
               double      dAlt = 0.0;
               int64_t     tmMs = tmNowMs;

               if (bGround)
                  dAlt = 0.0;
               else if (bAlt)
                  dAlt = dAltFt * METRES_PER_FOOT;

               if (!bTrack)
                  dTrack = 0.0;

               if (bSeenPos)
               {
                  tmMs = tmNowMs - (int64_t) (dSeenPos * 1000.0);
                  if (tmMs <= 0)
                     tmMs = tmNowMs;
               }

               strncpy (p->sId, sId, sizeof (p->sId) - 1);
               p->sId[sizeof (p->sId) - 1] = 0;
               p->bGround = bGround;
               p->bLive   = true;
               if (bGs  &&  dGs >= 0.0)
                  p->dGsKt = dGs;
               Craft_Set_Type (p, aType);
               Craft_Wp_Push (p, dLat, dLon, dAlt, dTrack, tmMs);
            }
         }
      }
   }

   if (bParsed)
   {
      for (i = s_nCraft - 1; i >= 0; i--)
      {
         if (!s_aCraft[i].bLive)
         {
            Craft_Remove_At (i);
            nGone++;
         }
      }
   }

   snprintf (aLog, sizeof (aLog),
      "dfw-adsbfi: live %d  (+%d moved %d gone %d of %d reports)",
      s_nCraft, nPlaced, nMoved, nGone, nSeen);
   Console_Log (s_pHost, aLog);

   return s_nCraft;
}

static void Fetch_ADSB (void)
{
   char   aUrl[512];
   char   aApi[384];
   int    nLen;
   double dNm;

   if (s_pHost == 0  ||  s_hRequest != 0)
      return;

   if (!Data_String (s_pHost, "ADSB.sUrl", aApi, (int) sizeof (aApi)))
      strncpy (aApi, ADSB_URL_DEFAULT, sizeof (aApi) - 1);
   aApi[sizeof (aApi) - 1] = 0;

   nLen = (int) strlen (aApi);
   while (nLen > 0  &&  aApi[nLen - 1] == '/')
      aApi[--nLen] = 0;

   dNm = s_dRadiusMi * METRES_PER_MILE / METRES_PER_NM;
   if (dNm < 1.0)
      dNm = 1.0;
   if (dNm > 250.0)
      dNm = 250.0;

   snprintf (aUrl, sizeof (aUrl),
      "%s/lat/%.6f/lon/%.6f/dist/%.1f?_=%u",
      aApi, s_dOriginLat, s_dOriginLon, dNm,
      (unsigned) (Chrono_Date (s_pHost) / 1000));

   s_hRequest = Network_Request_Open (s_pHost, kSNEEZE_ABI_REQUEST_VERB_GET, aUrl);

   if (!Request_IsValid (s_hRequest))
   {
      Console_Error (s_pHost, "dfw-adsbfi: Request_Open failed");
      s_hRequest = 0;
      return;
   }

   Request_Header_Set  (s_hRequest, "Accept", "application/json");
   Request_Header_Set  (s_hRequest, "User-Agent", "DFW_ADSBfi/1.0");
   Request_Timeout_Set (s_hRequest, REQUEST_TIMEOUT_MS);

   if (!Request_Send (s_hRequest))
   {
      Console_Error (s_pHost, "dfw-adsbfi: Request_Send failed");
      Request_Close (s_hRequest);
      s_hRequest = 0;
   }
   else
   {
      Console_Log (s_pHost, "dfw-adsbfi: ADSB.fi request sent");
   }
}

// --- Lifecycle -------------------------------------------------------------

void Instance_Open (HOST* pHost)
{
   HMAPOBJECT pRoot;
   HNODE      qwRoot;
   char       aLog[128];

   Console_Log (pHost, "dfw-adsbfi: opening");
   Console_Warn (pHost, "dfw-adsbfi: press 1 to toggle panel (this wasm logs every digit)");

   if (s_pHost != 0)
   {
      Console_Log (pHost, "dfw-adsbfi: reload - closing the live instance first");
      Instance_Close (s_pHost);
   }

   pRoot  = MapObject_Root ();
   qwRoot = Fabric_Node_Root (pHost, pRoot);
   MapObject_Free (pRoot);

   if (!Node_IsValid (qwRoot))
   {
      Console_Error (pHost, "dfw-adsbfi: root node failed");
      return;
   }

   s_pHost      = pHost;
   s_nCraft     = 0;
   s_hRequest   = 0;
   s_twTimer    = 0;
   s_nPollTick  = 0;
   s_qwPad      = 0;
   s_qwLegend   = 0;
   s_bPad       = false;
   s_bListed    = false;
   s_bLegend      = false;
   s_bPanelWanted = true;
   s_bLabelLogged = false;
   Scene_Hold (HOLD_TICKS_OPEN);
   s_wRootClass = Node_Class    (qwRoot);
   s_twRootIx   = Node_ObjectIx (qwRoot);

   Data_Number (pHost, "Origin.dLat", DFW_LAT_DEFAULT, &s_dOriginLat);
   Data_Number (pHost, "Origin.dLon", DFW_LON_DEFAULT, &s_dOriginLon);
   Data_Number (pHost, "RadiusMiles", RADIUS_MI_DEFAULT, &s_dRadiusMi);
   Data_Number (pHost, "IconLengthMetres", ICON_LEN_DEFAULT, &s_dIconLen);
   Data_Number (pHost, "Panel.dX",     PANEL_X_DEFAULT,     &s_dPanelX);
   Data_Number (pHost, "Panel.dY",     PANEL_Y_DEFAULT,     &s_dPanelY);
   Data_Number (pHost, "Panel.dZ",     PANEL_Z_DEFAULT,     &s_dPanelZ);
   Data_Number (pHost, "Panel.dScale", PANEL_SCALE_DEFAULT, &s_dPanelScale);
   Data_Number (pHost, "Panel.dWidth",  PANEL_W_DEFAULT,    &s_dPanelW);
   Data_Number (pHost, "Panel.dHeight", PANEL_H_DEFAULT,    &s_dPanelH);
   Label_Load (pHost);
   Panel_Wanted_Init (pHost);
   {
      char* sScale = Data_Get (pHost, "Panel.dScale");
      char  aRaw[96];

      if (sScale != 0)
      {
         snprintf (aRaw, sizeof (aRaw), "dfw-adsbfi: Data Panel.dScale = %s", sScale);
         Console_Log (pHost, aRaw);
         free (sScale);
      }
      else
         Console_Log (pHost, "dfw-adsbfi: Data Panel.dScale missing");
   }
   {
      double dPoll = (double) POLL_DEFAULT_SEC;
      Data_Number (pHost, "ADSB.nIntervalSec", (double) POLL_DEFAULT_SEC, &dPoll);
      s_nPollSec = (int) (dPoll + 0.5);
      if (s_nPollSec < POLL_MIN_SEC)
         s_nPollSec = POLL_MIN_SEC;
      if (s_nPollSec > POLL_MAX_SEC)
         s_nPollSec = POLL_MAX_SEC;
      {
         double dLag = (double) s_nPollSec;
         Data_Number (pHost, "ADSB.nLagSec", (double) s_nPollSec, &dLag);
         s_nLagSec = (int) (dLag + 0.5);
         if (s_nLagSec < 1)
            s_nLagSec = 1;
         if (s_nLagSec > 180)
            s_nLagSec = 180;
      }
   }

   if (s_dRadiusMi < 1.0)
      s_dRadiusMi = 1.0;
   if (s_dIconLen < 5.0)
      s_dIconLen = 5.0;
   if (s_dIconLen > 90.0)
      s_dIconLen = 90.0;
   if (s_dPanelScale <= 0.0)
      s_dPanelScale = PANEL_SCALE_DEFAULT;
   if (s_dPanelW <= 0.0)
      s_dPanelW = PANEL_W_DEFAULT;
   if (s_dPanelH <= 0.0)
      s_dPanelH = PANEL_H_DEFAULT;
   if (s_dLabelW < 5.0)
      s_dLabelW = LABEL_W_M;
   if (s_dLabelW > 500.0)
      s_dLabelW = 500.0;
   if (s_dLabelH < 4.0)
      s_dLabelH = LABEL_H_M;
   if (s_dLabelH > 200.0)
      s_dLabelH = 200.0;
   if (s_dLabelAbove < 0.0)
      s_dLabelAbove = 0.0;
   if (s_dLabelAbove > 500.0)
      s_dLabelAbove = 500.0;
   if (s_nLabelPixW < 0)
      s_nLabelPixW = 0;
   if (s_nLabelPixW > 2048)
      s_nLabelPixW = 2048;
   if (s_nLabelPixH < 32)
      s_nLabelPixH = LABEL_RASTER_H;
   if (s_nLabelPixH > 2048)
      s_nLabelPixH = 2048;
   if (s_nLabelDen < 1)
      s_nLabelDen = 1;
   if (s_nLabelDen > 4)
      s_nLabelDen = 4;
   if (s_nLabelScale != 0  &&  s_nLabelScale != 1)
      s_nLabelScale = LABEL_SCALE_MODE;
   if (s_nLabelBill < 0  ||  s_nLabelBill > 2)
      s_nLabelBill = LABEL_BILLBOARD;

   snprintf (aLog, sizeof (aLog),
      "dfw-adsbfi: origin %.5f, %.5f  radius %.1f mi  dart %.0f m  poll %d s  lag %d s",
      s_dOriginLat, s_dOriginLon, s_dRadiusMi, s_dIconLen, s_nPollSec, s_nLagSec);
   Console_Log (pHost, aLog);

   snprintf (aLog, sizeof (aLog),
      "dfw-adsbfi: panel (%.0f, %.0f, %.0f) scale %.4f  %.2f x %.2f",
      s_dPanelX, s_dPanelY, s_dPanelZ, s_dPanelScale, s_dPanelW, s_dPanelH);
   Console_Log (pHost, aLog);

   snprintf (aLog, sizeof (aLog),
      "dfw-adsbfi: labels %.0f x %.0f m  clearance %.0f m  billboard %d  scale %d",
      s_dLabelW, s_dLabelH, s_dLabelAbove, s_nLabelBill, s_nLabelScale);
   Console_Log (pHost, aLog);

   Fetch_ADSB ();
   s_twTimer = Timer_Interval (pHost, SPAWN_HZ, kSNEEZE_ABI_TIMER_UNIT_HZ, 0);
}

void Instance_Request (HOST* pHost, HREQUEST hRequest, bool bSuccess)
{
   if (pHost != s_pHost)
      return;

   if (hRequest != s_hRequest)
      return;

   if (!bSuccess)
   {
      char* sErr = Request_Error (hRequest);
      char  aMsg[192];

      snprintf (aMsg, sizeof (aMsg), "ADSB.fi request failed: %s",
         (sErr && sErr[0]) ? sErr : "transport error");
      Console_Error (pHost, aMsg);
      free (sErr);
   }
   else
   {
      int nStatus = Request_Status (hRequest);

      if (nStatus != 200)
      {
         char aMsg[96];
         snprintf (aMsg, sizeof (aMsg), "ADSB.fi HTTP %d", nStatus);
         Console_Error (pHost, aMsg);
      }
      else
      {
         char* sText = Request_Text (hRequest);

         if (sText == NULL)
         {
            Console_Error (pHost, "dfw-adsbfi: empty ADSB.fi body");
         }
         else
         {
            Ac_Load (sText, (int) strlen (sText));
            free (sText);
         }
      }
   }

   Request_Close (hRequest);
   s_hRequest = 0;
}

void Instance_Timer (HOST* pHost, uint64_t twTimerIx, uint64_t qwParam)
{
   int nNew;
   int nNeed;

   (void) qwParam;

   if (pHost != s_pHost  ||  twTimerIx != s_twTimer)
      return;

   if (s_bPad)
   {
      int     i;
      int64_t tmPlay = Clock_Play_Sync (Chrono_Date (pHost));

      for (i = 0; i < s_nCraft; i++)
      {
         if (Node_IsValid (s_aCraft[i].qwIcon))
            Craft_Animate (&s_aCraft[i], tmPlay);
      }
   }

   // Hold delays the next Node_Open after a close. Aircraft already in the
   // scene keep moving so a label swap does not freeze the traffic.
   if (s_nHoldTicks > 0)
   {
      s_nHoldTicks--;
      return;
   }

   if (!s_bPad)
   {
      Scene_Origin ();
      return;
   }

   nNeed = Crafts_Need_Icon ();

   for (nNew = 0; nNew < CRAFTS_PER_TICK  &&  nNeed > 0; nNew++)
   {
      int i;
      int nHole = -1;

      for (i = 0; i < s_nCraft; i++)
      {
         if (!s_aCraft[i].bTried)
         {
            nHole = i;
            break;
         }
      }

      if (nHole < 0)
         break;

      Craft_Spawn (nHole);
      nNeed--;
   }

   nNeed = Crafts_Need_Icon ();

   if (nNeed == 0  &&  s_nCraft > 0  &&  s_bPanelWanted  &&  !s_bLegend)
   {
      char aLog[96];

      Legend_Open ();
      snprintf (aLog, sizeof (aLog),
         "dfw-adsbfi: %d aircraft  (auto every %d s, digit 1 panel)",
         s_nCraft, s_nPollSec);
      Console_Log (pHost, aLog);
   }

   if (nNeed == 0)
   {
      s_nPollTick++;
      if (s_nPollTick >= s_nPollSec * SPAWN_HZ)
      {
         s_nPollTick = 0;
         Fetch_ADSB ();
      }
   }

   if (nNew == 0)
      Label_Service ();
}

#ifdef TEMPORARY_DIGIT_KEYS
void Instance_Key (HOST* pHost, int nDigit)
{
   char aLog[96];

   snprintf (aLog, sizeof (aLog),
      "dfw-adsbfi: digit %d  host %s",
      nDigit, (pHost == s_pHost) ? "ok" : "mismatch");
   if (s_pHost != 0)
      Console_Warn (s_pHost, aLog);
   else if (pHost != 0)
      Console_Warn (pHost, aLog);

   if (pHost != s_pHost  &&  s_pHost == 0)
      return;

   if (nDigit == 1  ||  nDigit == '1')
      Legend_Toggle ();
   else
      Traffic_Refresh ();
}
#endif

void Instance_Close (HOST* pHost)
{
   if (pHost != s_pHost)
      return;

   if (s_twTimer != 0)
   {
      Timer_Clear (pHost, s_twTimer);
      s_twTimer = 0;
   }

   if (s_hRequest != 0)
   {
      Request_Abort (s_hRequest);
      Request_Close (s_hRequest);
      s_hRequest = 0;
   }

   Scene_Clear_Crafts ();
   Legend_Close ();
   Pad_Close ();

   s_pHost   = 0;
   s_nCraft  = 0;
   s_bListed = false;
}
