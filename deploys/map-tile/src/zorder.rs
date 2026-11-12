// Z-fight planning. Two surfaces that overlap in plan and sit within the depth
// buffer's resolution of each other flicker. Patches are processed in draw
// order; each one finds the earlier patches it overlaps (and conflicts with)
// and takes the lowest level above all of them. A level is one `gap` of lift,
// so patches that do not overlap share a level instead of stacking up.

use std::collections::HashMap;

use crate::tess::{self, Tri};

pub const MAX_LEVEL: u32 = 24;
const MIN_OVERLAP_M2: f64 = 0.02;
const MIN_GAP_M: f64 = 0.05;
const MAX_GAP_M: f64 = 0.6;
const GAP_PER_METRE: f64 = 4.0e-5;

pub struct Patch
{
   pub rgb: [u8; 3],
   pub tris: Vec<Tri>,
}

pub struct Plan
{
   pub level: Vec<u32>,
   /// Overlapping pairs that conflicted, i.e. would have fought without a gap.
   pub pairs: usize,
   pub depth: u32,
   /// Patches that wanted a level past MAX_LEVEL and were held there.
   pub capped: usize,
}

/// Vertical spacing between stacked patches, metres. The view distance grows
/// with the window, so does the spacing; `dGapM` overrides it.
pub fn gap_metres (width_m: f64, length_m: f64, value: Option<f64>) -> f64
{
   match value
   {
      Some (v) if v.is_finite () && v > 0.0 => v,
      _ => (width_m.max (length_m) * GAP_PER_METRE).clamp (MIN_GAP_M, MAX_GAP_M),
   }
}

fn bbox (t: &Tri) -> [f64; 4]
{
   [
      t[0].0.min (t[1].0).min (t[2].0),
      t[0].0.max (t[1].0).max (t[2].0),
      t[0].1.min (t[1].1).min (t[2].1),
      t[0].1.max (t[1].1).max (t[2].1),
   ]
}

fn overlap (a: &Tri, b: &Tri) -> bool
{
   let poly = tess::clip (&tess::ccw (*a), &tess::ccw (*b));

   poly.len () >= 3 && tess::area (&poly).abs () > MIN_OVERLAP_M2
}

/// `conflict (later, earlier)` says whether an overlap between the two matters.
pub fn plan (width_m: f64, length_m: f64, patches: &[Patch], conflict: impl Fn (usize, usize) -> bool) -> Plan
{
   let cell = (width_m.max (length_m) / 48.0).max (4.0);
   let (hw, hl) = (width_m * 0.5, length_m * 0.5);
   let at = |e: f64, n: f64| (((e + hw) / cell).floor () as i32, ((n + hl) / cell).floor () as i32);

   let mut level = vec![0u32; patches.len ()];
   let mut seen = vec![usize::MAX; patches.len ()];
   let mut boxes: Vec<[f64; 4]> = Vec::new ();
   let mut owner: Vec<u32> = Vec::new ();
   let mut tris: Vec<Tri> = Vec::new ();
   let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new ();
   let (mut pairs, mut capped) = (0, 0);

   for (i, patch) in patches.iter ().enumerate ()
   {
      let mut want = 0u32;

      for t in &patch.tris
      {
         let b = bbox (t);
         let (c0, c1) = (at (b[0], b[2]), at (b[1], b[3]));

         for cx in c0.0..=c1.0
         {
            for cy in c0.1..=c1.1
            {
               let Some (list) = grid.get (&(cx, cy)) else { continue };

               for &g in list
               {
                  let j = owner[g as usize] as usize;
                  let o = boxes[g as usize];

                  if seen[j] == i || o[1] < b[0] || o[0] > b[1] || o[3] < b[2] || o[2] > b[3]
                  {
                     continue;
                  }

                  if !overlap (t, &tris[g as usize])
                  {
                     continue;
                  }

                  seen[j] = i;

                  if conflict (i, j)
                  {
                     pairs += 1;
                     want = want.max (level[j] + 1);
                  }
               }
            }
         }
      }

      if want > MAX_LEVEL
      {
         want = MAX_LEVEL;
         capped += 1;
      }

      level[i] = want;

      for t in &patch.tris
      {
         let b = bbox (t);
         let (c0, c1) = (at (b[0], b[2]), at (b[1], b[3]));
         let g = tris.len () as u32;

         tris.push (*t);
         boxes.push (b);
         owner.push (i as u32);

         for cx in c0.0..=c1.0
         {
            for cy in c0.1..=c1.1
            {
               grid.entry ((cx, cy)).or_default ().push (g);
            }
         }
      }
   }

   let depth = level.iter ().copied ().max ().unwrap_or (0);

   Plan { level, pairs, depth, capped }
}

#[cfg(test)]
mod tests
{
   use super::*;

   fn square (x: f64, y: f64, s: f64, rgb: [u8; 3]) -> Patch
   {
      let (a, b, c, d) = ((x, y), (x + s, y), (x + s, y + s), (x, y + s));

      Patch { rgb, tris: vec![[a, b, c], [a, c, d]] }
   }

   #[test]
   fn nested_patches_stack_and_separate_ones_share ()
   {
      let patches = vec![
         square (-40.0, -40.0, 80.0, [1, 1, 1]),
         square (-20.0, -20.0, 40.0, [2, 2, 2]),
         square (-5.0, -5.0, 10.0, [3, 3, 3]),
         square (30.0, 30.0, 5.0, [4, 4, 4]),
      ];
      let p = plan (200.0, 200.0, &patches, |a, b| patches[a].rgb != patches[b].rgb);

      // The small square far from the others only overlaps the big one.
      assert_eq! (p.level, vec![0, 1, 2, 1]);
      assert_eq! (p.depth, 2);
      assert! (p.pairs >= 3);
   }

   #[test]
   fn same_material_and_touching_edges_do_not_conflict ()
   {
      let patches = vec![
         square (0.0, 0.0, 10.0, [9, 9, 9]),
         square (5.0, 5.0, 10.0, [9, 9, 9]),
         square (10.0, -10.0, 10.0, [7, 7, 7]),
      ];
      let p = plan (100.0, 100.0, &patches, |a, b| patches[a].rgb != patches[b].rgb);

      assert_eq! (p.level, vec![0, 0, 0]);
      assert_eq! (p.pairs, 0);
   }

   #[test]
   fn long_chains_are_capped ()
   {
      let patches: Vec<Patch> = (0..40).map (|i| square (0.0, 0.0, 10.0, [i as u8, 0, 0])).collect ();
      let p = plan (100.0, 100.0, &patches, |a, b| patches[a].rgb != patches[b].rgb);

      assert_eq! (p.depth, MAX_LEVEL);
      assert! (p.capped > 0);
   }

   #[test]
   fn gap_grows_with_the_window ()
   {
      assert! ((gap_metres (500.0, 500.0, None) - 0.05).abs () < 1e-9);
      assert! (gap_metres (3000.0, 7000.0, None) > 0.25);
      assert! ((gap_metres (200_000.0, 10.0, None) - 0.6).abs () < 1e-9);
      assert! ((gap_metres (500.0, 500.0, Some (0.2)) - 0.2).abs () < 1e-9);
      assert! ((gap_metres (500.0, 500.0, Some (f64::NAN)) - 0.05).abs () < 1e-9);
   }
}
