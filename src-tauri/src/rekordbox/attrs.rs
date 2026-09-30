//! An element's attributes as read, and the lenient value parsers the
//! typed fields use.

use std::borrow::Cow;
use std::fmt;

/// Every attribute of one element, in the order the file lists them, with
/// entities resolved (`&amp;` is `&`). This is what the XML writer sends
/// back (ROADMAP 1.9 rule 1), so nothing is dropped or defaulted here,
/// including attributes this parser doesn't know.
///
/// Stored compactly, since a large collection has millions of them: one
/// text buffer per element holding the values (and any unknown names) back
/// to back, and a small span per attribute. Known names are an index into
/// [`KNOWN_NAMES`] rather than text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attrs {
    text: String,
    spans: Vec<Span>,
}

/// One attribute: its name (a [`KNOWN_NAMES`] index, or [`UNKNOWN`] with
/// the name stored in the text), then where its name and value end in the
/// text. Each attribute's text starts where the previous one's value ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    known: u8,
    name_end: u32,
    value_end: u32,
}

const UNKNOWN: u8 = u8::MAX;

impl Attrs {
    /// The value of `name` (exact, case-sensitive, as rekordbox writes it).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.iter().find(|&(n, _)| n == name).map(|(_, v)| v)
    }

    /// Every attribute, in file order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        let mut start = 0;
        self.spans.iter().map(move |span| {
            let (name_end, value_end) = (span.name_end as usize, span.value_end as usize);
            let name = match KNOWN_NAMES.get(usize::from(span.known)) {
                Some(known) => known,
                None => &self.text[start..name_end],
            };
            let value = &self.text[name_end..value_end];
            start = value_end;
            (name, value)
        })
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Adds an attribute. An element whose attribute text passes 4 GiB
    /// (never a real rekordbox file) keeps what fits.
    pub(crate) fn push(&mut self, name: &str, value: &str) {
        let known = KNOWN_NAMES.iter().position(|&k| k == name);
        let name_len = if known.is_some() { 0 } else { name.len() };
        let Ok(value_end) = u32::try_from(self.text.len() + name_len + value.len()) else {
            return;
        };
        if known.is_none() {
            self.text.push_str(name);
        }
        let name_end = self.text.len() as u32;
        self.text.push_str(value);
        self.spans.push(Span {
            known: known.map_or(UNKNOWN, |i| i as u8),
            name_end,
            value_end,
        });
    }

    pub(crate) fn shrink(&mut self) {
        self.text.shrink_to_fit();
        self.spans.shrink_to_fit();
    }
}

/// Attribute names rekordbox writes, stored as an index rather than as
/// text. Fewer than 255 ([`UNKNOWN`] marks the rest).
const KNOWN_NAMES: &[&str] = &[
    "TrackID",
    "Name",
    "Artist",
    "Composer",
    "Album",
    "Grouping",
    "Genre",
    "Kind",
    "Size",
    "TotalTime",
    "DiscNumber",
    "TrackNumber",
    "Year",
    "AverageBpm",
    "DateModified",
    "DateAdded",
    "BitRate",
    "SampleRate",
    "Comments",
    "PlayCount",
    "LastPlayed",
    "Rating",
    "Location",
    "Remixer",
    "Tonality",
    "Label",
    "Mix",
    "Colour",
    "Inizio",
    "Bpm",
    "Metro",
    "Battito",
    "Type",
    "Start",
    "End",
    "Num",
    "Red",
    "Green",
    "Blue",
];

/// Resolves XML's entities in a raw attribute value: the five named ones
/// and `&#…;` / `&#x…;`. A malformed or unknown reference (a bare `&`,
/// `&nbsp;`) is kept literally and reported through the `bool`, so one bad
/// character never loses the rest of the value.
pub(crate) fn unescape(raw: &str) -> (Cow<'_, str>, bool) {
    if !raw.contains('&') {
        return (Cow::Borrowed(raw), false);
    }
    let mut out = String::with_capacity(raw.len());
    let mut bad = false;
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let resolved = rest
            .find(';')
            .filter(|&semi| semi <= 12)
            .and_then(|semi| entity(&rest[1..semi]).map(|c| (c, semi)));
        match resolved {
            Some((c, semi)) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                bad = true;
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    (Cow::Owned(out), bad)
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) if !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                    u32::from_str_radix(hex, 16).ok()?
                }
                None if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
                    digits.parse().ok()?
                }
                _ => return None,
            };
            char::from_u32(code).filter(|&c| c != '\0')
        }
    }
}

/// A calendar date as rekordbox writes it: `2026-09-28`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub(crate) fn parse(s: &str) -> Option<Date> {
        let mut parts = s.split('-');
        let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return None;
        }
        let year: u16 = digits(y)?;
        let month: u8 = digits(m)?;
        let day: u8 = digits(d)?;
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        (1..=days)
            .contains(&day)
            .then_some(Date { year, month, day })
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A color: a track's `Colour` (`0xFF0000`) or a cue's `Red`/`Green`/`Blue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// `0xRRGGBB` (either case of `x` and of the digits).
    pub(crate) fn parse_hex(s: &str) -> Option<Rgb> {
        let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
        if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Rgb {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        })
    }
}

/// Unsigned decimal digits only: no sign, no spaces, no `1e3`.
pub(crate) fn digits<T: std::str::FromStr>(s: &str) -> Option<T> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A decimal number such as `128.00` or `-0.023`: finite, no exponent.
pub(crate) fn decimal(s: &str) -> Option<f64> {
    let body = s.strip_prefix('-').unwrap_or(s);
    let mut dots = 0;
    let ok = !body.is_empty()
        && body.bytes().all(|b| {
            if b == b'.' {
                dots += 1;
                true
            } else {
                b.is_ascii_digit()
            }
        })
        && dots <= 1
        && body != ".";
    if !ok {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_named_entities_and_numeric_references_resolve() {
        let (s, bad) =
            unescape("Kit &amp; Kin&apos;s &lt;x&gt; &quot;q&quot; &#38;&#x26;&#X1F680;");
        assert_eq!(s, "Kit & Kin's <x> \"q\" &&🚀");
        assert!(!bad);
    }

    #[test]
    fn a_bad_entity_is_kept_literally_and_reported() {
        for raw in [
            "a & b",
            "a &nbsp; b",
            "a &#xZZ; b",
            "a &#0; b",
            "a &#1114112; b",
            "a&",
        ] {
            let (s, bad) = unescape(raw);
            assert!(bad, "{raw:?}");
            assert_eq!(s, raw, "kept as written");
        }
        let (s, bad) = unescape("&bogus; &amp;");
        assert_eq!(s, "&bogus; &");
        assert!(bad);
    }

    #[test]
    fn text_without_ampersands_is_borrowed_unchanged() {
        assert!(matches!(unescape("plain"), (Cow::Borrowed("plain"), false)));
    }

    #[test]
    fn dates_parse_only_as_real_calendar_days() {
        assert_eq!(
            Date::parse("2024-02-29"),
            Some(Date {
                year: 2024,
                month: 2,
                day: 29
            })
        );
        for bad in [
            "2023-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-04-31",
            "26-09-28",
            "2026-9-28",
            "",
            "2026-09-28x",
            "2026/09/28",
        ] {
            assert_eq!(Date::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Date::parse("2026-09-28").unwrap().to_string(), "2026-09-28");
    }

    #[test]
    fn colours_parse_from_rekordbox_hex() {
        assert_eq!(Rgb::parse_hex("0xFF0000"), Some(Rgb { r: 255, g: 0, b: 0 }));
        assert_eq!(
            Rgb::parse_hex("0x25fd"),
            Some(Rgb {
                r: 0,
                g: 0x25,
                b: 0xfd
            })
        );
        for bad in ["FF0000", "0x", "0x1000000", "0xGG0000", "#FF0000"] {
            assert_eq!(Rgb::parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn numbers_are_strict_about_their_shape() {
        assert_eq!(digits::<u32>("0042"), Some(42));
        assert_eq!(digits::<u32>("+1"), None);
        assert_eq!(digits::<u32>(" 1"), None);
        assert_eq!(digits::<u8>("256"), None);
        assert_eq!(decimal("128.00"), Some(128.0));
        assert_eq!(decimal("-0.023"), Some(-0.023));
        assert_eq!(decimal("5"), Some(5.0));
        for bad in ["", ".", "1.2.3", "1e3", "NaN", "inf", "12,5", " 1"] {
            assert_eq!(decimal(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn unknown_attribute_names_are_kept_and_known_ones_stored_as_an_index() {
        let mut attrs = Attrs::default();
        attrs.push("Name", "x");
        attrs.push("SomethingNew", "y");
        attrs.push("Empty", "");
        attrs.push("Artist", "Kit & Kin");
        assert_eq!(attrs.text, "xSomethingNewyEmptyKit & Kin");
        assert_eq!(attrs.get("SomethingNew"), Some("y"));
        assert_eq!(attrs.get("Empty"), Some(""));
        assert_eq!(attrs.get("Missing"), None);
        assert_eq!(
            attrs.iter().collect::<Vec<_>>(),
            [
                ("Name", "x"),
                ("SomethingNew", "y"),
                ("Empty", ""),
                ("Artist", "Kit & Kin")
            ]
        );
        assert_eq!(attrs.len(), 4);
        assert!(KNOWN_NAMES.len() < usize::from(UNKNOWN));
    }
}
