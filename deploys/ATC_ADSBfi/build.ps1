# Builds wasm\ATC_ADSBfi.wasm from src\ATC_ADSBfi.c plus the Sneeze C SDK.
# Run: powershell -File .\build.ps1
$ErrorActionPreference = 'Stop'

$Here = Split-Path -Parent $MyInvocation.MyCommand.Path

function First-Existing
{
   param ([string[]] $Paths)

   foreach ($p in $Paths)
   {
      if ($p  -and  (Test-Path -LiteralPath $p))
      {
         return $p
      }
   }
   return $null
}

# ABI headers live in the SneezeSDK superproject; C sources live in SneezeSDK_C.
$SdkRoot = First-Existing @(
   $env:SNEEZE_SDK,
   'C:\s\Sneeze\deps\repos\SneezeSDK',
   "$env:USERPROFILE\Documents\GitHub\cursors\Sneeze\deps\repos\SneezeSDK",
   'e:\Dev\OMB\Sneeze\deps\repos\SneezeSDK'
)

$CRoot = First-Existing @(
   $env:SNEEZE_SDK_C,
   "$env:USERPROFILE\sdk\SneezeSDK_C",
   "$env:USERPROFILE\Documents\dev\SneezeSDK_C",
   $(if ($SdkRoot) { "$SdkRoot\C" } else { $null })
)

$Emsdk = First-Existing @(
   $env:EMSDK,
   "$env:USERPROFILE\sdk\emsdk",
   "$env:USERPROFILE\.emsdk",
   "$env:USERPROFILE\emsdk",
   "$env:USERPROFILE\Documents\GitHub\cursors\emsdk"
)

if (-not $SdkRoot) { Write-Error "SneezeSDK not found (set SNEEZE_SDK)" }
if (-not $CRoot)   { Write-Error "SneezeSDK_C not found (set SNEEZE_SDK_C)" }
if (-not $Emsdk)   { Write-Error "emsdk not found (set EMSDK)" }

$CInc   = "$CRoot\include"
$CSrc   = "$CRoot\src"
$AbiInc = "$SdkRoot\include"

New-Item -ItemType Directory -Force -Path "$Here\wasm" | Out-Null

$sdkSources = @(
   "$CSrc\sneeze_ffi.c",
   "$CSrc\sneeze_json.c",
   "$CSrc\sneeze_snapshot.c",
   "$CSrc\sneeze_mapobject.c",
   "$CSrc\sneeze_objects.c",
   "$CSrc\sneeze_moment.c",
   "$CSrc\sneeze_instance.c"
) -join ' '

$emccFlags = "-std=c11 -Os -Wno-address-of-packed-member " +
             "-I$CInc -I$CSrc -I$AbiInc " +
             "-sSTANDALONE_WASM -sWASM_BIGINT --no-entry " +
             "-sERROR_ON_UNDEFINED_SYMBOLS=0 -sMALLOC=emmalloc"

Write-Host "SDK  $SdkRoot"
Write-Host "C    $CRoot"
Write-Host "emsdk $Emsdk"

$name = 'ATC_ADSBfi'
$emcc = "emcc $emccFlags $sdkSources $Here\src\$name.c -o $Here\wasm\$name.wasm"
cmd /c "`"$Emsdk\emsdk_env.bat`" >nul 2>&1 && $emcc"

if ($LASTEXITCODE -ne 0)
{
   Write-Error "emcc failed for $name (exit $LASTEXITCODE)"
}

Write-Host "Built $Here\wasm\$name.wasm"
