// Planar geometry in east/north metres: rings to polygons, earcut, convex clip, line ribbons.

pub type Pt = (f64, f64);
pub type Tri = [Pt; 3];

pub struct Poly
{
   pub outer: Vec<Pt>,
   pub holes: Vec<Vec<Pt>>,
}

pub fn area (ring: &[Pt]) -> f64
{
   let mut sum = 0.0;

   for i in 0..ring.len ()
   {
      let a = ring[i];
      let b = ring[(i + 1) % ring.len ()];
      sum += a.0 * b.1 - b.0 * a.1;
   }

   sum * 0.5
}

pub fn tri_area (t: &Tri) -> f64
{
   ((t[1].0 - t[0].0) * (t[2].1 - t[0].1) - (t[2].0 - t[0].0) * (t[1].1 - t[0].1)) * 0.5
}

pub fn ccw (t: Tri) -> Tri
{
   if tri_area (&t) < 0.0 { [t[0], t[2], t[1]] } else { t }
}

fn clean (ring: &[Pt]) -> Vec<Pt>
{
   let mut out: Vec<Pt> = Vec::with_capacity (ring.len ());

   for p in ring
   {
      if let Some (q) = out.last ()
      {
         if (q.0 - p.0).abs () < 1e-4 && (q.1 - p.1).abs () < 1e-4 { continue; }
      }

      out.push (*p);
   }

   while out.len () > 1
   {
      let (a, b) = (out[0], out[out.len () - 1]);
      if (a.0 - b.0).abs () < 1e-4 && (a.1 - b.1).abs () < 1e-4 { out.pop (); } else { break; }
   }

   out
}

pub fn inside (p: Pt, ring: &[Pt]) -> bool
{
   let mut yes = false;
   let mut j = ring.len () - 1;

   for i in 0..ring.len ()
   {
      let (a, b) = (ring[i], ring[j]);

      if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0
      {
         yes = !yes;
      }

      j = i;
   }

   yes
}

// Vector tiles list each exterior ring followed by its holes. In east/north
// coordinates an exterior winds clockwise and a hole counter-clockwise; a ring
// only counts as a hole when it also lies inside the polygon before it.
// Outers come out counter-clockwise and holes clockwise.
pub fn polygons (rings: &[Vec<Pt>]) -> Vec<Poly>
{
   let mut out: Vec<Poly> = Vec::new ();

   for ring in rings
   {
      let mut r = clean (ring);

      if r.len () < 3 { continue; }

      let a = area (&r);

      if a.abs () < 1e-4 { continue; }

      let hole = a > 0.0 && out.last ().map_or (false, |p| inside (r[0], &p.outer));

      if hole
      {
         r.reverse ();
         out.last_mut ().unwrap ().holes.push (r);
      }
      else
      {
         if a < 0.0 { r.reverse (); }
         out.push (Poly { outer: r, holes: Vec::new () });
      }
   }

   out
}

pub fn poly_area (p: &Poly) -> f64
{
   area (&p.outer) - p.holes.iter ().map (|h| area (h).abs ()).sum::<f64> ()
}

// Points are the outer ring then each hole, in order; triangles index them and
// are counter-clockwise.
pub fn indexed (poly: &Poly) -> (Vec<Pt>, Vec<[usize; 3]>)
{
   let mut pts: Vec<Pt> = poly.outer.clone ();
   let mut holes: Vec<usize> = Vec::new ();

   for h in &poly.holes
   {
      holes.push (pts.len ());
      pts.extend_from_slice (h);
   }

   let flat: Vec<f64> = pts.iter ().flat_map (|p| [p.0, p.1]).collect ();
   let Ok (idx) = earcutr::earcut (&flat, &holes, 2) else { return (pts, Vec::new ()) };
   let mut tris = Vec::with_capacity (idx.len () / 3);

   for c in idx.chunks_exact (3)
   {
      let mut t = [c[0], c[1], c[2]];
      let a = tri_area (&[pts[t[0]], pts[t[1]], pts[t[2]]]);

      if a.abs () < 1e-6 { continue; }
      if a < 0.0 { t.swap (1, 2); }

      tris.push (t);
   }

   (pts, tris)
}

pub fn triangulate (poly: &Poly) -> Vec<Tri>
{
   let (pts, tris) = indexed (poly);

   tris.iter ().map (|t| [pts[t[0]], pts[t[1]], pts[t[2]]]).collect ()
}

fn cut (a: Pt, b: Pt, sa: f64, sb: f64) -> Pt
{
   let t = sa / (sa - sb);
   (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

// Sutherland-Hodgman against a counter-clockwise convex polygon.
pub fn clip (subject: &[Pt], by: &[Pt]) -> Vec<Pt>
{
   const EPS: f64 = 1e-9;
   let mut out: Vec<Pt> = subject.to_vec ();

   for i in 0..by.len ()
   {
      if out.is_empty () { break; }

      let a = by[i];
      let b = by[(i + 1) % by.len ()];
      let side = |p: Pt| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
      let input = std::mem::take (&mut out);

      for j in 0..input.len ()
      {
         let cur = input[j];
         let prev = input[(j + input.len () - 1) % input.len ()];
         let (sc, sp) = (side (cur), side (prev));

         if sc >= -EPS
         {
            if sp < -EPS { out.push (cut (prev, cur, sp, sc)); }
            out.push (cur);
         }
         else if sp >= -EPS
         {
            out.push (cut (prev, cur, sp, sc));
         }
      }
   }

   out
}

// A mitred strip of constant width along a polyline.
pub fn ribbon (line: &[Pt], half: f64) -> Vec<Tri>
{
   let mut pts: Vec<Pt> = Vec::with_capacity (line.len ());

   for p in line
   {
      if let Some (q) = pts.last ()
      {
         if (q.0 - p.0).hypot (q.1 - p.1) < 0.05 { continue; }
      }

      pts.push (*p);
   }

   let n = pts.len ();

   if n < 2 { return Vec::new (); }

   let dir = |a: Pt, b: Pt| { let l = (b.0 - a.0).hypot (b.1 - a.1); ((b.0 - a.0) / l, (b.1 - a.1) / l) };
   let mut left: Vec<Pt> = Vec::with_capacity (n);
   let mut right: Vec<Pt> = Vec::with_capacity (n);

   for i in 0..n
   {
      let d0 = if i > 0 { dir (pts[i - 1], pts[i]) } else { dir (pts[0], pts[1]) };
      let d1 = if i + 1 < n { dir (pts[i], pts[i + 1]) } else { d0 };
      let n0 = (-d0.1, d0.0);
      let n1 = (-d1.1, d1.0);
      let (mx, my) = (n0.0 + n1.0, n0.1 + n1.1);
      let ml = mx.hypot (my);
      let u = if ml < 1e-6 { n0 } else { (mx / ml, my / ml) };
      let c = (u.0 * n0.0 + u.1 * n0.1).max (0.35);
      let s = half / c;
      let p = pts[i];

      left.push ((p.0 + u.0 * s, p.1 + u.1 * s));
      right.push ((p.0 - u.0 * s, p.1 - u.1 * s));
   }

   let mut out = Vec::with_capacity ((n - 1) * 2);

   for i in 0..n - 1
   {
      out.push (ccw ([left[i], right[i], right[i + 1]]));
      out.push (ccw ([left[i], right[i + 1], left[i + 1]]));
   }

   out
}

#[cfg(test)]
mod tests
{
   use super::*;

   #[test]
   fn square_with_hole_triangulates_to_its_area ()
   {
      // clockwise exterior then counter-clockwise hole, as a tile stores them
      let rings = vec![
         vec![(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)],
         vec![(3.0, 3.0), (7.0, 3.0), (7.0, 7.0), (3.0, 7.0)],
      ];
      let polys = polygons (&rings);

      assert_eq! (polys.len (), 1);
      assert_eq! (polys[0].holes.len (), 1);

      let total: f64 = triangulate (&polys[0]).iter ().map (tri_area).sum ();
      assert! ((total - 84.0).abs () < 1e-6, "area {total}");
   }

   #[test]
   fn clip_keeps_overlap_only ()
   {
      let square = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
      let tri = [(0.0, 0.0), (8.0, 0.0), (0.0, 8.0)];
      let out = clip (&square, &tri);

      assert! ((area (&out) - 16.0).abs () < 1e-9);
   }

   #[test]
   fn ribbon_has_the_requested_width ()
   {
      let tris = ribbon (&[(0.0, 0.0), (10.0, 0.0)], 1.0);
      let total: f64 = tris.iter ().map (tri_area).sum ();

      assert! ((total - 20.0).abs () < 1e-9);
   }
}
