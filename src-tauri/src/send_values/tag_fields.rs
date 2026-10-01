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
//! - Blank text is no value. A value that was cut when it was stored
//!   (`full_len` set: lyrics, long notes) is no value either, since sending
//!   half a comment would be worse than none.
//! - ID3v1 has no usable genre (a number) and MP4's track and disc numbers
//!   are binary, so those aren't mapped.

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

/// The value as the attribute takes it, or `None` for no value.
fn clean(attribute: &str, text: &str) -> Option<String> {
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
        // `2020`, `2020-05-01`, but not `20200501` or `May`.
        "Year" => {
            let text = text.trim_start();
            let year: String = text.chars().take(4).collect();
            let rest = &text[year.len()..];
            (year.len() == 4
                && year.chars().all(|c| c.is_ascii_digit())
                && !rest.starts_with(|c: char| c.is_ascii_digit()))
            .then_some(year)
        }
        "TrackNumber" | "DiscNumber" => leading_digits(5)
            .and_then(|d| d.parse::<u32>().ok())
            .filter(|n| *n > 0)
            .map(|n| n.to_string()),
        _ => Some(text.to_owned()),
    }
}

/// Every mapped field the file's `raw_tags` give, one value each, in the
/// order of [`TAG_ATTRIBUTES`]. Tags that don't parse give nothing.
pub fn read(raw_tags: &str) -> Vec<TagValue> {
    let Ok(Value::Object(blocks)) = serde_json::from_str::<Value>(raw_tags) else {
        return Vec::new();
    };
    let mut found: Vec<TagValue> = Vec::new();
    for block in BLOCKS {
        let Some(Value::Array(items)) = blocks.get(block) else {
            continue;
        };
        for item in items {
            let key = item.get("key").and_then(Value::as_str).unwrap_or("");
            let Some(attribute) = attribute_for(block, key) else {
                continue;
            };
            if found.iter().any(|f| f.attribute == attribute) {
                continue;
            }
            let value = item.get("value");
            let text = value.and_then(|v| v.get("text")).and_then(Value::as_str);
            let cut = value.and_then(|v| v.get("full_len")).is_some();
            if let (Some(text), false) = (text, cut) {
                if let Some(value) = clean(attribute, text) {
                    found.push(TagValue {
                        attribute,
                        value,
                        tag: format!("{block}:{key}"),
                    });
                }
            }
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
        assert_eq!(clean("Year", "2020").as_deref(), Some("2020"));
        assert_eq!(clean("Year", "2020-05-01").as_deref(), Some("2020"));
        assert_eq!(clean("Year", "20200501"), None);
        assert_eq!(clean("Year", "99"), None);
    }

    #[test]
    fn tags_that_are_not_json_give_nothing() {
        assert!(read("not json").is_empty());
        assert!(read("[1]").is_empty());
        assert!(read("{}").is_empty());
    }
}
