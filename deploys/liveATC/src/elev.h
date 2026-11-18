// Terrain height at a latitude and longitude, from one Terrarium PNG.
// Height is metres above sea level, the same encoding map-tile uses:
// (R * 256 + G + B / 256) - 32768. A failed decode returns a non-finite value.

#pragma once

#include <stdbool.h>
#include <stdint.h>

// Decodes an 8-bit RGB or RGBA PNG into tightly packed RGB. *ppRgb is malloc'd.
bool Elev_Rgb (const uint8_t* aSrc, int nSrc, int* pnW, int* pnH, uint8_t** ppRgb);

// Slippy tile that holds the point, and where it falls inside that tile (0..1).
void Elev_Slippy (double dLat, double dLon, int nZ, int* pnX, int* pnY, double* pdX, double* pdY);

// Bilinear Terrarium height. dX/dY are 0..1 across the image. Not finite on a void.
double Elev_Sample (const uint8_t* aRgb, int nW, int nH, double dX, double dY);
