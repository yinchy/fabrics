// Reads the fabric URL's query string:
//   ?lat_lon=[40.758,-73.9855]&len_wid=[500,100]
// lat_lon is [latitude, longitude] in degrees; len_wid is [length, width] in
// metres, length running north-south and width east-west. Brackets are
// optional, percent-encoding is accepted, and anything malformed is ignored so
// the fabric's own Data values apply.

fn decode (text: &str) -> String
{
   let bytes = text.as_bytes ();
   let mut out: Vec<u8> = Vec::with_capacity (bytes.len ());
   let mut i = 0;

   while i < bytes.len ()
   {
      match bytes[i]
      {
         b'%' if i + 2 < bytes.len () && bytes[i + 1].is_ascii_hexdigit () && bytes[i + 2].is_ascii_hexdigit () =>
         {
            let hex = std::str::from_utf8 (&bytes[i + 1..i + 3]).unwrap_or ("00");
            out.push (u8::from_str_radix (hex, 16).unwrap_or (b'?'));
            i += 3;
         }
         b'+' =>
         {
            out.push (b' ');
            i += 1;
         }
         c =>
         {
            out.push (c);
            i += 1;
         }
      }
   }

   String::from_utf8_lossy (&out).into_owned ()
}

fn pair (href: &str, key: &str) -> Option<(f64, f64)>
{
   let query = href.split ('#').next ()?.split_once ('?')?.1;

   for part in query.split ('&')
   {
      let (k, v) = part.split_once ('=').unwrap_or ((part, ""));

      if decode (k) != key { continue; }

      let value = decode (v);
      let tokens: Vec<&str> = value
         .split (|c: char| matches!(c, '[' | ']' | '(' | ')' | ',' | ';') || c.is_whitespace ())
         .filter (|s| !s.is_empty ())
         .collect ();

      if tokens.len () != 2 { return None; }

      let a: f64 = tokens[0].parse ().ok ()?;
      let b: f64 = tokens[1].parse ().ok ()?;

      return if a.is_finite () && b.is_finite () { Some ((a, b)) } else { None };
   }

   None
}

/// (latitude, longitude), only when it is a real place on the map.
pub fn lat_lon (href: &str) -> Option<(f64, f64)>
{
   pair (href, "lat_lon").filter (|(lat, lon)| lat.abs () <= 85.0 && lon.abs () <= 180.0)
}

/// (length, width) in metres, only when both are positive.
pub fn len_wid (href: &str) -> Option<(f64, f64)>
{
   pair (href, "len_wid").filter (|(len, wid)| *len > 0.0 && *wid > 0.0)
}

#[cfg(test)]
mod tests
{
   use super::*;

   const BASE: &str = "https://cdn.rp1.com/fabric/map-tile.json";

   #[test]
   fn reads_both_pairs ()
   {
      let url = format! ("{BASE}?lat_lon=[40,-50]&len_wid=[500,100]");

      assert_eq! (lat_lon (&url), Some ((40.0, -50.0)));
      assert_eq! (len_wid (&url), Some ((500.0, 100.0)));
   }

   #[test]
   fn accepts_percent_encoding_and_loose_brackets ()
   {
      assert_eq! (lat_lon (&format! ("{BASE}?lat_lon=%5B40.758%2C-73.9855%5D")), Some ((40.758, -73.9855)));
      assert_eq! (lat_lon (&format! ("{BASE}?lat_lon=40.5,-73.5")), Some ((40.5, -73.5)));
      assert_eq! (len_wid (&format! ("{BASE}?v=1&len_wid=%5B300%2C%20200%5D#top")), Some ((300.0, 200.0)));
   }

   #[test]
   fn missing_or_bad_values_fall_back ()
   {
      assert_eq! (lat_lon (BASE), None);
      assert_eq! (lat_lon (&format! ("{BASE}?lat_lon=[40]")), None);
      assert_eq! (lat_lon (&format! ("{BASE}?lat_lon=[40,x]")), None);
      assert_eq! (lat_lon (&format! ("{BASE}?lat_lon=[95,10]")), None);
      assert_eq! (lat_lon (&format! ("{BASE}?lat_lon=[10,200]")), None);
      assert_eq! (len_wid (&format! ("{BASE}?len_wid=[0,100]")), None);
      assert_eq! (len_wid (&format! ("{BASE}?len_wid=[-5,100]")), None);
      assert_eq! (len_wid (&format! ("{BASE}?len_wid=[500,100,7]")), None);
   }

   #[test]
   fn module_cache_buster_is_not_confused_for_input ()
   {
      assert_eq! (lat_lon ("map_tile.wasm?v=7"), None);
   }
}
