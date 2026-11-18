#include "elev.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>
#include <zlib.h>

static uint32_t Be32 (const uint8_t* p)
{
   return ((uint32_t) p[0] << 24) | ((uint32_t) p[1] << 16)
        | ((uint32_t) p[2] << 8)  | (uint32_t) p[3];
}

static int Paeth (int a, int b, int c)
{
   int p  = a + b - c;
   int pa = abs (p - a);
   int pb = abs (p - b);
   int pc = abs (p - c);

   if (pa <= pb && pa <= pc) return a;
   if (pb <= pc) return b;
   return c;
}

static bool Inflate (const uint8_t* aSrc, int nSrc, uint8_t* aDst, int nDst)
{
   uLongf n = (uLongf) nDst;

   return nSrc > 0 && uncompress (aDst, &n, aSrc, (uLong) nSrc) == Z_OK && (int) n == nDst;
}

bool Elev_Rgb (const uint8_t* aSrc, int nSrc, int* pnW, int* pnH, uint8_t** ppRgb)
{
   static const uint8_t aSig[8] = { 137, 80, 78, 71, 13, 10, 26, 10 };
   int      nW = 0;
   int      nH = 0;
   int      nDepth = 0;
   int      nColor = 0;
   int      nBpp = 0;
   int      i = 8;
   int      nIdat = 0;
   int      nCap = 0;
   uint8_t* aIdat = 0;
   uint8_t* aRaw = 0;
   uint8_t* aRgb = 0;
   int      nStride;
   int      y;

   if (ppRgb) *ppRgb = 0;
   if (aSrc == 0 || nSrc < 8 || memcmp (aSrc, aSig, 8) != 0)
      return false;

   while (i + 12 <= nSrc)
   {
      uint32_t    nLen  = Be32 (aSrc + i);
      const char* sType = (const char*) (aSrc + i + 4);
      const uint8_t* p  = aSrc + i + 8;

      if (nLen > 16u * 1024u * 1024u || i + 12 + (int) nLen > nSrc)
         break;

      if (memcmp (sType, "IHDR", 4) == 0 && nLen >= 13)
      {
         nW     = (int) Be32 (p);
         nH     = (int) Be32 (p + 4);
         nDepth = p[8];
         nColor = p[9];
         if (p[10] != 0 || p[11] != 0 || p[12] != 0)
            nColor = -1;
      }
      else if (memcmp (sType, "IDAT", 4) == 0)
      {
         if (nIdat + (int) nLen > nCap)
         {
            int      nNew = nCap > 0 ? nCap * 2 : 65536;
            uint8_t* aNew;

            while (nNew < nIdat + (int) nLen)
               nNew *= 2;
            aNew = (uint8_t*) realloc (aIdat, (size_t) nNew);
            if (aNew == 0)
               break;
            aIdat = aNew;
            nCap  = nNew;
         }
         memcpy (aIdat + nIdat, p, nLen);
         nIdat += (int) nLen;
      }
      else if (memcmp (sType, "IEND", 4) == 0)
         break;

      i += 12 + (int) nLen;
   }

   nBpp = (nColor == 2) ? 3 : (nColor == 6) ? 4 : 0;

   if (nDepth != 8 || nBpp == 0 || nW < 1 || nH < 1 || nW > 2048 || nH > 2048 || nIdat < 2)
   {
      free (aIdat);
      return false;
   }

   nStride = nW * nBpp;
   aRaw = (uint8_t*) malloc ((size_t) nH * (size_t) (nStride + 1));
   aRgb = (uint8_t*) malloc ((size_t) nW * (size_t) nH * 3);

   if (aRaw == 0 || aRgb == 0 || !Inflate (aIdat, nIdat, aRaw, nH * (nStride + 1)))
   {
      free (aIdat);
      free (aRaw);
      free (aRgb);
      return false;
   }

   free (aIdat);

   for (y = 0; y < nH; y++)
   {
      uint8_t* pRow  = aRaw + (size_t) y * (size_t) (nStride + 1);
      uint8_t* pPrev = (y > 0) ? pRow - (nStride + 1) + 1 : 0;
      int      nFilter = pRow[0];
      uint8_t* pCur = pRow + 1;
      int      x;

      for (x = 0; x < nStride; x++)
      {
         int a = (x >= nBpp) ? pCur[x - nBpp] : 0;
         int b = pPrev ? pPrev[x] : 0;
         int c = (pPrev && x >= nBpp) ? pPrev[x - nBpp] : 0;
         int nPred = 0;
         int v = pCur[x];

         if (nFilter == 1) nPred = a;
         else if (nFilter == 2) nPred = b;
         else if (nFilter == 3) nPred = (a + b) / 2;
         else if (nFilter == 4) nPred = Paeth (a, b, c);
         else if (nFilter != 0)
         {
            free (aRaw);
            free (aRgb);
            return false;
         }

         pCur[x] = (uint8_t) ((v + nPred) & 255);
      }

      for (x = 0; x < nW; x++)
      {
         uint8_t* pDst = aRgb + ((size_t) y * (size_t) nW + (size_t) x) * 3;
         uint8_t* pSrc = pCur + x * nBpp;

         pDst[0] = pSrc[0];
         pDst[1] = pSrc[1];
         pDst[2] = pSrc[2];
      }
   }

   free (aRaw);
   if (pnW) *pnW = nW;
   if (pnH) *pnH = nH;
   if (ppRgb) *ppRgb = aRgb;
   else free (aRgb);
   return true;
}

void Elev_Slippy (double dLat, double dLon, int nZ, int* pnX, int* pnY, double* pdX, double* pdY)
{
   double n = (double) (1 << nZ);
   double x = (dLon + 180.0) / 360.0 * n;
   double r = dLat * 3.141592653589793 / 180.0;
   double y = (1.0 - log (tan (r) + 1.0 / cos (r)) / 3.141592653589793) / 2.0 * n;
   int    nX = (int) floor (x);
   int    nY = (int) floor (y);
   int    nMax = (1 << nZ) - 1;

   if (nX < 0) nX = 0;
   if (nY < 0) nY = 0;
   if (nX > nMax) nX = nMax;
   if (nY > nMax) nY = nMax;

   if (pnX) *pnX = nX;
   if (pnY) *pnY = nY;
   if (pdX) *pdX = x - floor (x);
   if (pdY) *pdY = y - floor (y);
}

static double Height_At (const uint8_t* aRgb, int nW, int nH, int x, int y)
{
   const uint8_t* p;
   double d;

   if (x < 0) x = 0;
   if (y < 0) y = 0;
   if (x >= nW) x = nW - 1;
   if (y >= nH) y = nH - 1;

   p = aRgb + ((size_t) y * (size_t) nW + (size_t) x) * 3;
   d = ((double) p[0] * 256.0 + (double) p[1] + (double) p[2] / 256.0) - 32768.0;

   if (d < -500.0 || d > 9000.0)
      return nan ("");
   return d;
}

double Elev_Sample (const uint8_t* aRgb, int nW, int nH, double dX, double dY)
{
   double dPx;
   double dPy;
   int    x0;
   int    y0;
   double dTx;
   double dTy;
   double d00, d10, d01, d11;
   double d0, d1;

   if (aRgb == 0 || nW < 1 || nH < 1 || !isfinite (dX) || !isfinite (dY))
      return nan ("");

   dPx = dX * (double) nW - 0.5;
   dPy = dY * (double) nH - 0.5;
   if (dPx < 0.0) dPx = 0.0;
   if (dPy < 0.0) dPy = 0.0;
   x0 = (int) floor (dPx);
   y0 = (int) floor (dPy);
   if (x0 >= nW - 1) { x0 = nW - 1; dTx = 0.0; }
   else dTx = dPx - (double) x0;
   if (y0 >= nH - 1) { y0 = nH - 1; dTy = 0.0; }
   else dTy = dPy - (double) y0;

   d00 = Height_At (aRgb, nW, nH, x0,     y0);
   d10 = Height_At (aRgb, nW, nH, x0 + 1, y0);
   d01 = Height_At (aRgb, nW, nH, x0,     y0 + 1);
   d11 = Height_At (aRgb, nW, nH, x0 + 1, y0 + 1);

   if (!isfinite (d00)) d00 = isfinite (d10) ? d10 : (isfinite (d01) ? d01 : d11);
   if (!isfinite (d10)) d10 = d00;
   if (!isfinite (d01)) d01 = d00;
   if (!isfinite (d11)) d11 = d01;
   if (!isfinite (d00))
      return nan ("");

   d0 = d00 * (1.0 - dTx) + d10 * dTx;
   d1 = d01 * (1.0 - dTx) + d11 * dTx;
   return d0 * (1.0 - dTy) + d1 * dTy;
}
