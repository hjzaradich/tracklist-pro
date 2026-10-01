//! A file's tags as rekordbox TRACK attributes (1aD-8).
//!
//! The database keeps each file's tags as the frames the format itself
//! holds (`file.raw_tags`, [`crate::tags::raw`]), not as canonical fields,
//! so this maps those frames to the attributes a send writes. Only fields
//! a tag can honestly give are mapped: no BPM, key, grid or cues, and
//! nothing rekordbox owns (play count, rating, colour, `Grouping`, dates).
//!
//! | Attribute     | ID3v2          | MP4 ilst        | Vorbis / APE                | RIFF INFO    | AIFF   | ID3v1   |
//! |---------------|----------------|-----------------|-----------------------------|--------------|--------|---------|
//! | `Name`        | `TIT2`         | `©nam`          | `TITLE`                     | `INAM`       | `NAME` | title   |
//! | `Artist`      | `TPE1`         | `©ART`          | `ARTIST`                    | `IART`       | `AUTH` | artist  |
//! | `Album`       | `TALB`         | `©alb`          | `ALBUM`                     | `IPRD`       |        | album   |
//! | `Genre`       | `TCON`         | `©gen`          | `GENRE`                     | `IGNR`       |        |         |
//! | `Composer`    | `TCOM`         | `©wrt`          | `COMPOSER`                  |              |        |         |
//! | `Remixer`     | `TPE4`         | freeform REMIXER| `REMIXER`                   |              |        |         |
//! | `Label`       | `TPUB`         | freeform LABEL  | `LABEL`, `ORGANIZATION`, `PUBLISHER` |     |        |         |
//! | `Year`        | `TDRC`, `TYER` | `©day`          | `DATE`, `YEAR`              | `ICRD`       |        | year    |
//! | `TrackNumber` | `TRCK`         | (binary: none)  | `TRACKNUMBER`, `TRACK`      | `IPRT`,`ITRK`|        | track   |
//! | `DiscNumber`  | `TPOS`         | (binary: none)  | `DISCNUMBER`, `DISC`        |              |        |         |
//! | `Comments`    | `COMM::<lang>` | `©cmt`          | `COMMENT`, `DESCRIPTION`    | `ICMT`       |        | comment |
//!
//! - Of a file's tag blocks the first that has the field wins, in the order
//!   ID3v2, MP4, Vorbis, APE, RIFF INFO, AIFF, ID3v1.
//! - `Year` is the leading four digits (`2020-05-01` is `2020`);
//!   `TrackNumber` and `DiscNumber` the leading number (`3/12` is `3`).
//!   Anything else for those three is no value.
//! - Several values in one field (ID3v2.4's NUL separator, or a repeated
//!   Vorbis / APE item) become one, joined with `, `. Nothing else in a
//!   value is changed: control characters stay, and the writer reports a
//!   value XML can't carry. Values aren't trimmed; only all-blank text is
//!   no value.
//! - An ID3v2 `Genre` in the old numeric forms (`(17)`, `17`, `(17)Hard`)
//!   goes through the ID3v1 genre list; an unknown number is left as it is.
//! - `Year` must be 1000-9999, like `tags::parse_year`.
//! - Blank text is no value. A value that was cut when it was stored
//!   (`full_len` set: lyrics, long notes) is no value either, since sending
//!   half a comment would be worse than none.
//! - ID3v1 has no usable genre (a number) and MP4's track and disc numbers
//!   are binary, so those aren't mapped.

use lofty::id3::v1::GENRES;
use serde_json::Value;

/// The TRACK attributes a tag can give, in the order rekordbox writes them.
pub const TAG_ATTRIBUTES: [&str; 11] = [
    "Name",
    "Artist",
    "Composer",
    "Album",
    "Genre",
    "DiscNumber",
    "TrackNumber",
    "Year",
    "Comments",
    "Remixer",
    "Label",
];

/// One field's value in a file, and the tag it came from, e.g.
/// `id3v2:TPE1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagValue {
    pub attribute: &'static str,
    pub value: String,
    pub tag: String,
}

/// The block order: the first block with a field gives it.
const BLOCKS: [&str; 7] = [
    "id3v2",
    "mp4_ilst",
    "vorbis_comments",
    "ape",
    "riff_info",
    "aiff_text",
    "id3v1",
];

/// The attribute a frame gives, if it gives one.
fn attribute_for(block: &str, key: &str) -> Option<&'static str> {
    let lower = key.to_ascii_lowercase();
    Some(match block {
        "id3v2" => match key {
            "TIT2" => "Name",
            "TPE1" => "Artist",
            "TALB" => "Album",
            "TCON" => "Genre",
            "TCOM" => "Composer",
            "TPE4" => "Remixer",
            "TPUB" => "Label",
            "TDRC" | "TYER" => "Year",
            "TRCK" => "TrackNumber",
            "TPOS" => "DiscNumber",
            // The plain comment frame (no description: `COMM::eng`), not
            // the ones programs keep their own data in (`COMM:iTunNORM:…`).
            _ if key.starts_with("COMM::") => "Comments",
            _ => return None,
        },
        "mp4_ilst" => match key {
            "\u{a9}nam" => "Name",
            "\u{a9}ART" => "Artist",
            "\u{a9}alb" => "Album",
            "\u{a9}gen" => "Genre",
            "\u{a9}wrt" => "Composer",
            "\u{a9}day" => "Year",
            "\u{a9}cmt" => "Comments",
            _ => match lower.as_str() {
                "----:com.apple.itunes:label" => "Label",
                "----:com.apple.itunes:remixer" => "Remixer",
                _ => return None,
            },
        },
        "vorbis_comments" | "ape" => match lower.as_str() {
            "title" => "Name",
            "artist" => "Artist",
            "album" => "Album",
            "genre" => "Genre",
            "composer" => "Composer",
            "remixer" => "Remixer",
            "label" | "organization" | "publisher" => "Label",
            "date" | "year" => "Year",
            "tracknumber" | "track" => "TrackNumber",
            "discnumber" | "disc" => "DiscNumber",
            "comment" | "description" => "Comments",
            _ => return None,
        },
        "riff_info" => match key {
            "INAM" => "Name",
            "IART" => "Artist",
            "IPRD" => "Album",
            "IGNR" => "Genre",
            "ICRD" => "Year",
            "IPRT" | "ITRK" => "TrackNumber",
            "ICMT" => "Comments",
            _ => return None,
        },
        "aiff_text" => match key {
            "NAME" => "Name",
            "AUTH" => "Artist",
            _ => return None,
        },
        "id3v1" => match key {
            "title" => "Name",
            "artist" => "Artist",
            "album" => "Album",
            "year" => "Year",
            "track" => "TrackNumber",
            "comment" => "Comments",
            _ => return None,
        },
        _ => return None,
    })
}

/// The ID3v2 genre `text` with its numeric forms resolved through the
/// ID3v1 list: `(17)` and `17` give `Rock`; `(17)Hard` gives the text after
/// the number, `Hard`; `((x)` is a literal `(x)`. A number that isn't in
/// the list, and `(RX)` / `(CR)` with nothing after them, give nothing
/// more than the text itself.
fn genre(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(literal) = trimmed.strip_prefix("((") {
        return format!("({literal}");
    }
    let name = |number: &str| -> Option<String> {
        let n: usize = number.parse().ok()?;
        GENRES.get(n).map(|g| (*g).to_owned())
    };
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return name(trimmed).unwrap_or_else(|| text.to_owned());
    }
    // Leading `(NN)` groups, then an optional refinement.
    let mut rest = trimmed;
    let mut first: Option<String> = None;
    while let Some(after) = rest.strip_prefix('(') {
        let Some(close) = after.find(')') else { break };
        let code = &after[..close];
        if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
            break;
        }
        if first.is_none() {
            first = name(code);
        }
        rest = &after[close + 1..];
        if first.is_none() && code.chars().all(|c| c.is_ascii_digit()) {
            // An unknown number: leave the text as it was.
            return text.to_owned();
        }
    }
    if rest.len() == trimmed.len() {
        return text.to_owned();
    }
    if !rest.trim().is_empty() {
        return rest.to_owned();
    }
    first.unwrap_or_else(|| text.to_owned())
}

/// A multi-value text (ID3v2.4 separates values with NUL) as one value:
/// the non-blank parts joined with `", "`. Nothing else in it is touched:
/// a value is never altered silently, and the writer reports a character
/// XML can't carry. Values aren't trimmed either (only an all-blank value
/// is none).
fn join_values(text: &str) -> Option<String> {
    let parts: Vec<&str> = text.split('\0').filter(|p| !p.trim().is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join(JOIN))
}

/// What joins several values of one field.
const JOIN: &str = ", ";

/// The value as the attribute takes it, or `None` for no value.
fn clean(block: &str, attribute: &str, text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    let leading_digits = |max: usize| -> Option<String> {
        let digits: String = text
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        (!digits.is_empty() && digits.len() <= max).then_some(digits)
    };
    match attribute {
        // `2020`, `2020-05-01`, but not `20200501`, `May` or `0999`; the
        // same range as `tags::parse_year`.
        "Year" => {
            let text = text.trim_start();
            let year: String = text.chars().take(4).collect();
            let rest = &text[year.len()..];
            (year.len() == 4
                && year.chars().all(|c| c.is_ascii_digit())
                && !rest.starts_with(|c: char| c.is_ascii_digit()))
            .then_some(year)
            .filter(|y| (1000..=9999).contains(&y.parse::<u32>().unwrap_or(0)))
        }
        "TrackNumber" | "DiscNumber" => leading_digits(5)
            .and_then(|d| d.parse::<u32>().ok())
            .filter(|n| *n > 0)
            .map(|n| n.to_string()),
        "Genre" if block == "id3v2" => {
            let parts: Vec<String> = text
                .split('\0')
                .filter(|p| !p.trim().is_empty())
                .map(genre)
                .collect();
            (!parts.is_empty()).then(|| parts.join(JOIN))
        }
        _ => join_values(text),
    }
}

/// Every mapped field the file's `raw_tags` give, one value each, in the
/// order of [`TAG_ATTRIBUTES`]. Tags that don't parse give nothing. In
/// Vorbis and APE blocks a repeated item (two `ARTIST`s) is one value, the
/// items joined like a NUL-separated one; the first block with the field
/// gives it.
pub fn read(raw_tags: &str) -> Vec<TagValue> {
    let Ok(Value::Object(blocks)) = serde_json::from_str::<Value>(raw_tags) else {
        return Vec::new();
    };
    let mut found: Vec<TagValue> = Vec::new();
    for block in BLOCKS {
        let Some(Value::Array(items)) = blocks.get(block) else {
            continue;
        };
        let repeats_join = matches!(block, "vorbis_comments" | "ape");
        // This block's values, by attribute, in item order.
        let mut here: Vec<(&'static str, Vec<String>, String)> = Vec::new();
        for item in items {
            let key = item.get("key").and_then(Value::as_str).unwrap_or("");
            let Some(attribute) = attribute_for(block, key) else {
                continue;
            };
            let value = item.get("value");
            let text = value.and_then(|v| v.get("text")).and_then(Value::as_str);
            let cut = value.and_then(|v| v.get("full_len")).is_some();
            let (Some(text), false) = (text, cut) else {
                continue;
            };
            let Some(value) = clean(block, attribute, text) else {
                continue;
            };
            match here.iter_mut().find(|(a, _, _)| *a == attribute) {
                Some((_, values, _)) => values.push(value),
                None => here.push((attribute, vec![value], format!("{block}:{key}"))),
            }
        }
        for (attribute, values, tag) in here {
            if found.iter().any(|f| f.attribute == attribute) {
                continue;
            }
            let value =
                if repeats_join && !matches!(attribute, "Year" | "TrackNumber" | "DiscNumber") {
                    values.join(JOIN)
                } else {
                    values[0].clone()
                };
            found.push(TagValue {
                attribute,
                value,
                tag,
            });
        }
    }
    found.sort_by_key(|f| TAG_ATTRIBUTES.iter().position(|a| *a == f.attribute));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(key: &str, text: &str) -> Value {
        json!({"key": key, "value": {"type": "text", "text": text}})
    }

    fn raw(blocks: Value) -> String {
        blocks.to_string()
    }

    fn get<'a>(values: &'a [TagValue], attribute: &str) -> Option<&'a str> {
        values
            .iter()
            .find(|v| v.attribute == attribute)
            .map(|v| v.value.as_str())
    }

    #[test]
    fn id3v2_frames_map_to_their_attributes() {
        let tags = raw(json!({"id3v2": [
            text("TIT2", "Synthetic Title"), text("TPE1", "Synthetic Artist"),
            text("TALB", "Synthetic Album"), text("TCON", "House"),
            text("TCOM", "Synthetic Composer"), text("TPE4", "Synthetic Remixer"),
            text("TPUB", "Synthetic Label"), text("TDRC", "2020-05-01"),
            text("TRCK", "3/12"), text("TPOS", "1/2"), text("COMM::eng", "a note"),
        ]}));
        let v = read(&tags);
        for (attribute, value) in [
            ("Name", "Synthetic Title"),
            ("Artist", "Synthetic Artist"),
            ("Album", "Synthetic Album"),
            ("Genre", "House"),
            ("Composer", "Synthetic Composer"),
            ("Remixer", "Synthetic Remixer"),
            ("Label", "Synthetic Label"),
            ("Year", "2020"),
            ("TrackNumber", "3"),
            ("DiscNumber", "1"),
            ("Comments", "a note"),
        ] {
            assert_eq!(get(&v, attribute), Some(value), "{attribute}");
        }
        assert_eq!(v.len(), 11);
        let order: Vec<_> = v.iter().map(|t| t.attribute).collect();
        assert_eq!(order, TAG_ATTRIBUTES);
        assert_eq!(v[0].tag, "id3v2:TIT2");
    }

    #[test]
    fn vorbis_and_ape_names_match_in_any_case() {
        let tags = raw(json!({"vorbis_comments": [
            text("Title", "T"), text("ARTIST", "A"), text("organization", "L"),
            text("DATE", "1999"), text("TRACKNUMBER", "07"),
        ]}));
        let v = read(&tags);
        assert_eq!(get(&v, "Name"), Some("T"));
        assert_eq!(get(&v, "Artist"), Some("A"));
        assert_eq!(get(&v, "Label"), Some("L"));
        assert_eq!(get(&v, "Year"), Some("1999"));
        assert_eq!(get(&v, "TrackNumber"), Some("7"));
    }

    #[test]
    fn mp4_riff_aiff_and_id3v1_blocks_map_too() {
        let mp4 = read(&raw(json!({"mp4_ilst": [
            text("\u{a9}nam", "T"), text("\u{a9}ART", "A"), text("\u{a9}day", "2001-02-03"),
            text("----:com.apple.iTunes:LABEL", "L"),
        ]})));
        assert_eq!(
            (get(&mp4, "Name"), get(&mp4, "Year"), get(&mp4, "Label")),
            (Some("T"), Some("2001"), Some("L"))
        );
        let riff = read(&raw(
            json!({"riff_info": [text("INAM", "T"), text("IART", "A")]}),
        ));
        assert_eq!(get(&riff, "Artist"), Some("A"));
        let aiff = read(&raw(
            json!({"aiff_text": [text("NAME", "T"), text("AUTH", "A")]}),
        ));
        assert_eq!(get(&aiff, "Name"), Some("T"));
        let v1 = read(&raw(
            json!({"id3v1": [text("title", "T"), text("genre", "17")]}),
        ));
        assert_eq!((get(&v1, "Name"), get(&v1, "Genre")), (Some("T"), None));
    }

    #[test]
    fn the_first_block_with_a_field_wins() {
        let tags = raw(json!({
            "id3v1": [text("title", "From v1"), text("album", "Only v1")],
            "id3v2": [text("TIT2", "From v2")],
        }));
        let v = read(&tags);
        assert_eq!(get(&v, "Name"), Some("From v2"));
        assert_eq!(get(&v, "Album"), Some("Only v1"));
    }

    #[test]
    fn blank_odd_and_cut_values_are_no_value() {
        let cut = json!({"key": "COMM::eng", "value": {"type": "text", "text": "half", "full_len": 9000}});
        let tags = raw(json!({"id3v2": [
            text("TIT2", "   "), text("TDRC", "May"), text("TRCK", "x"), text("TPOS", "0"),
            text("TALB", ""), cut,
            {"key": "TPE1", "value": {"type": "binary", "len": 4}},
            text("COMM:iTunNORM:eng", "000001"),
        ]}));
        assert!(read(&tags).is_empty());
    }

    #[test]
    fn a_year_is_four_digits_not_a_longer_number() {
        let year = |t| clean("id3v2", "Year", t);
        assert_eq!(year("2020").as_deref(), Some("2020"));
        assert_eq!(year("2020-05-01").as_deref(), Some("2020"));
        assert_eq!(year("20200501"), None);
        assert_eq!(year("99"), None);
    }

    #[test]
    fn a_year_outside_1000_to_9999_gives_no_year() {
        for text in ["0000", "0999", "0001"] {
            assert_eq!(clean("id3v2", "Year", text), None, "{text}");
        }
        assert_eq!(clean("id3v2", "Year", "1000").as_deref(), Some("1000"));
        assert_eq!(clean("id3v2", "Year", "9999").as_deref(), Some("9999"));
    }

    #[test]
    fn values_separated_by_nul_are_joined_with_a_comma() {
        let v = read(&raw(
            json!({"id3v2": [text("TPE1", "A\0B"), text("TCOM", "C\0")]}),
        ));
        assert_eq!(get(&v, "Artist"), Some("A, B"));
        // A trailing separator is just the end of the value.
        assert_eq!(get(&v, "Composer"), Some("C"));
    }

    #[test]
    fn a_lone_nul_is_no_value() {
        assert!(read(&raw(json!({"id3v2": [text("TPE1", "\0")]}))).is_empty());
    }

    #[test]
    fn a_repeated_vorbis_or_ape_item_is_one_joined_value() {
        let v = read(&raw(json!({"vorbis_comments": [
            text("ARTIST", "One"), text("TITLE", "T"), text("ARTIST", "Two"),
            text("GENRE", "House\0Techno"),
        ]})));
        assert_eq!(get(&v, "Artist"), Some("One, Two"));
        assert_eq!(get(&v, "Genre"), Some("House, Techno"));
        // The tag named is the first item's.
        let artist = v.iter().find(|t| t.attribute == "Artist").unwrap();
        assert_eq!(artist.tag, "vorbis_comments:ARTIST");
        let ape = read(&raw(
            json!({"ape": [text("Artist", "A"), text("Artist", "B")]}),
        ));
        assert_eq!(get(&ape, "Artist"), Some("A, B"));
        // Other blocks give their first item only.
        let id3 = read(&raw(
            json!({"id3v2": [text("TPE1", "First"), text("TPE1", "Second")]}),
        ));
        assert_eq!(get(&id3, "Artist"), Some("First"));
    }

    #[test]
    fn other_control_characters_and_spaces_are_kept_as_they_are() {
        let comment = " line1\nline2\t\u{1} ";
        let v = read(&raw(json!({"id3v2": [text("COMM::eng", comment)]})));
        assert_eq!(get(&v, "Comments"), Some(comment));
    }

    #[test]
    fn old_numeric_genres_are_resolved_through_the_id3v1_list() {
        for (tcon, expected) in [
            ("(17)", "Rock"),
            ("17", "Rock"),
            ("(17)Hard Rock", "Hard Rock"),
            ("(17)(0)", "Rock"),
            ("((Rock)", "(Rock)"),
            ("House", "House"),
            ("(17)\0(0)", "Rock, Blues"),
            // An unknown number, and codes with nothing after them, stay.
            ("(250)", "(250)"),
            ("250", "250"),
            ("(RX)", "(RX)"),
            ("(RX)Remix", "Remix"),
        ] {
            let v = read(&raw(json!({"id3v2": [text("TCON", tcon)]})));
            assert_eq!(get(&v, "Genre"), Some(expected), "{tcon:?}");
        }
        // Only ID3v2 genres are read this way.
        let v = read(&raw(json!({"vorbis_comments": [text("GENRE", "17")]})));
        assert_eq!(get(&v, "Genre"), Some("17"));
    }

    #[test]
    fn tags_that_are_not_json_give_nothing() {
        assert!(read("not json").is_empty());
        assert!(read("[1]").is_empty());
        assert!(read("{}").is_empty());
    }
}
