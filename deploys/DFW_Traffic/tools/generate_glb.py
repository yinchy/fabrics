# Builds the origin pad GLB. res/aircraft.glb is hand-authored — never write it.

from __future__ import annotations

import json
import struct
from pathlib import Path


def _cross(a, b):
    return (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )


def _sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def _norm(v):
    length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) ** 0.5
    if length < 1e-12:
        return (0.0, 0.0, 1.0)
    return (v[0] / length, v[1] / length, v[2] / length)


class Mesh:
    def __init__(self):
        self.positions: list[float] = []
        self.normals: list[float] = []
        self.indices: list[int] = []

    def tri(self, a, b, c):
        n = _norm(_cross(_sub(b, a), _sub(c, a)))
        base = len(self.positions) // 3
        for p in (a, b, c):
            self.positions.extend(p)
            self.normals.extend(n)
        self.indices.extend((base, base + 1, base + 2))

    def bounds(self):
        xs = self.positions[0::3]
        ys = self.positions[1::3]
        zs = self.positions[2::3]
        return [min(xs), min(ys), min(zs)], [max(xs), max(ys), max(zs)]

    def scale(self, s):
        for i in range(len(self.positions)):
            self.positions[i] *= s

    # Authoring below is Z-up (nose +X, left +Y, up +Z). glTF is Y-up; the
    # engine then applies Rx(+90) (x,y,z)->(x,-z,y). This rotation makes that
    # authored frame land in world ENU: +X east, +Y north, +Z up.
    def zup_to_gltf_yup(self):
        def rot(a):
            for i in range(0, len(a), 3):
                x, y, z = a[i], a[i + 1], a[i + 2]
                a[i], a[i + 1], a[i + 2] = x, z, -y

        rot(self.positions)
        rot(self.normals)


def origin_mesh() -> Mesh:
    m = Mesh()
    ring = []
    steps = 16
    for i in range(steps):
        ang = 2.0 * 3.141592653589793 * i / steps
        c, s = __import__("math").cos(ang), __import__("math").sin(ang)
        ring.append((c, s, 0.02))

    centre = (0.0, 0.0, 0.02)
    for i in range(steps):
        m.tri(centre, ring[i], ring[(i + 1) % steps])

    north = (0.00, 1.45, 0.06)
    n_l = (-0.28, 0.72, 0.06)
    n_r = (0.28, 0.72, 0.06)
    m.tri(north, n_l, n_r)

    pylon_top = (0.0, 0.0, 0.55)
    pylon_a = (0.08, 0.08, 0.02)
    pylon_b = (-0.08, 0.08, 0.02)
    pylon_c = (-0.08, -0.08, 0.02)
    pylon_d = (0.08, -0.08, 0.02)
    m.tri(pylon_top, pylon_a, pylon_b)
    m.tri(pylon_top, pylon_b, pylon_c)
    m.tri(pylon_top, pylon_c, pylon_d)
    m.tri(pylon_top, pylon_d, pylon_a)

    return m


def write_glb(path: Path, mesh: Mesh, color):
    pos = bytes().join(struct.pack("<fff", *mesh.positions[i : i + 3]) for i in range(0, len(mesh.positions), 3))
    nrm = bytes().join(struct.pack("<fff", *mesh.normals[i : i + 3]) for i in range(0, len(mesh.normals), 3))
    idx = bytes().join(struct.pack("<H", i) for i in mesh.indices)

    def align4(blob: bytes) -> bytes:
        pad = (4 - (len(blob) % 4)) % 4
        return blob + (b"\x00" * pad)

    pos = align4(pos)
    nrm = align4(nrm)
    idx = align4(idx)
    binary = pos + nrm + idx

    n_verts = len(mesh.positions) // 3
    n_idx = len(mesh.indices)
    mn, mx = mesh.bounds()

    gltf = {
        "asset": {"version": "2.0", "generator": "DFW_Traffic/tools/generate_glb.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [
            {
                "primitives": [
                    {
                        "attributes": {"POSITION": 0, "NORMAL": 1},
                        "indices": 2,
                        "material": 0,
                        "mode": 4,
                    }
                ]
            }
        ],
        "accessors": [
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": n_verts,
                "type": "VEC3",
                "min": mn,
                "max": mx,
            },
            {"bufferView": 1, "componentType": 5126, "count": n_verts, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5123, "count": n_idx, "type": "SCALAR"},
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": len(pos), "target": 34962},
            {"buffer": 0, "byteOffset": len(pos), "byteLength": len(nrm), "target": 34962},
            {
                "buffer": 0,
                "byteOffset": len(pos) + len(nrm),
                "byteLength": len(idx),
                "target": 34963,
            },
        ],
        "buffers": [{"byteLength": len(binary)}],
        "materials": [
            {
                "pbrMetallicRoughness": {
                    "baseColorFactor": list(color),
                    "metallicFactor": 0.0,
                    "roughnessFactor": 0.6,
                },
            }
        ],
    }

    json_bytes = json.dumps(gltf, separators=(",", ":")).encode("utf-8")
    json_pad = (4 - (len(json_bytes) % 4)) % 4
    json_bytes = json_bytes + (b" " * json_pad)

    total = 12 + 8 + len(json_bytes) + 8 + len(binary)
    header = struct.pack("<4sII", b"glTF", 2, total)
    json_chunk = struct.pack("<II", len(json_bytes), 0x4E4F534A) + json_bytes
    bin_chunk = struct.pack("<II", len(binary), 0x004E4942) + binary
    path.write_bytes(header + json_chunk + bin_chunk)


def main():
    root = Path(__file__).resolve().parents[1] / "res"
    root.mkdir(parents=True, exist_ok=True)

    origin = origin_mesh()
    origin.zup_to_gltf_yup()
    write_glb(root / "origin.glb", origin, (0.95, 0.72, 0.18, 1.0))
    print(f"wrote {root / 'origin.glb'}")
    print(f"left {root / 'aircraft.glb'} (hand-authored)")


if __name__ == "__main__":
    main()
