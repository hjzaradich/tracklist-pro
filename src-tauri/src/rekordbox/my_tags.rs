//! My Tags in a track's `Comments` (1aD-4; ROADMAP §5.1).
//!
//! With "Add My Tag to the comments" turned on, rekordbox appends a block
//! at the END of the comment: `<user comment> /* TagA */`. With one tag
//! and no comment it's exactly `/* TLP */`. A `/* … */` anywhere but at
//! the end is the DJ's own text, not My Tags.
//!
//! Only reading: the raw `Comments` attribute is stored and sent back
//! unchanged (1.9 rule 1); [`tags`] and [`without_tags`] are views of it.

/// What separates several tags inside the block. Not confirmed yet (one
/// tag is the only format seen so far), so it lives here alone.
pub const SEPARATOR: &str = " / ";

const OPEN: &str = "/*";
const CLOSE: &str = "*/";

/// Where the My Tags block starts and the tag names in it.
struct Block<'a> {
    /// Byte offset of the block's `/*` in the comment.
    start: usize,
    tags: Vec<&'a str>,
}

/// The block at the end of `comments`, if it names at least one tag. An
/// empty block (`/* */`) names none, so it isn't taken for My Tags.
fn block(comments: &str) -> Option<Block<'_>> {
    let body = comments.trim_end().strip_suffix(CLOSE)?;
    let start = body.rfind(OPEN)?;
    let tags: Vec<&str> = body[start + OPEN.len()..]
        .trim()
        .split(SEPARATOR)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    (!tags.is_empty()).then_some(Block { start, tags })
}

/// The My Tag names in a `Comments` value, in the order rekordbox wrote
/// them.
pub fn tags(comments: &str) -> Vec<String> {
    block(comments).map_or_else(Vec::new, |b| {
        b.tags.into_iter().map(str::to_owned).collect()
    })
}

/// The comment as the DJ wrote it: `comments` without its My Tags block,
/// and without the one space rekordbox puts in front of the block. The
/// value is returned as it is when it holds no block.
pub fn without_tags(comments: &str) -> &str {
    match block(comments) {
        Some(b) => {
            let before = &comments[..b.start];
            before.strip_suffix(' ').unwrap_or(before)
        }
        None => comments,
    }
}

/// [`tags`] as the JSON array stored in `rekordbox_track.my_tags`.
pub fn tags_json(comments: &str) -> String {
    serde_json::to_string(&tags(comments)).expect("strings always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_tag_after_a_comment_is_read() {
        assert_eq!(tags("Great opener /* Peak */"), ["Peak"]);
        assert_eq!(without_tags("Great opener /* Peak */"), "Great opener");
    }

    #[test]
    fn a_comment_that_is_only_the_block_has_the_tag_and_no_comment() {
        assert_eq!(tags("/* TLP */"), ["TLP"]);
        assert_eq!(without_tags("/* TLP */"), "");
    }

    #[test]
    fn several_tags_split_on_the_separator() {
        let comments = format!("warm up /* A{SEPARATOR}B C{SEPARATOR}D */");
        assert_eq!(tags(&comments), ["A", "B C", "D"]);
        assert_eq!(without_tags(&comments), "warm up");
    }

    #[test]
    fn a_comment_without_a_block_has_no_tags_and_is_returned_whole() {
        assert!(tags("just a note").is_empty());
        assert_eq!(without_tags("just a note"), "just a note");
    }

    #[test]
    fn an_empty_comment_has_no_tags() {
        assert!(tags("").is_empty());
        assert_eq!(without_tags(""), "");
        assert_eq!(tags_json(""), "[]");
    }

    #[test]
    fn a_block_that_is_not_at_the_end_is_not_my_tags() {
        let comments = "keep /* this */ as written";
        assert!(tags(comments).is_empty());
        assert_eq!(without_tags(comments), comments);
    }

    #[test]
    fn an_earlier_block_stays_in_the_comment_when_the_real_one_is_last() {
        let comments = "a /* note */ b /* Tag */";
        assert_eq!(tags(comments), ["Tag"]);
        assert_eq!(without_tags(comments), "a /* note */ b");
    }

    #[test]
    fn an_empty_block_names_no_tag_and_is_left_in_the_comment() {
        assert!(tags("x /* */").is_empty());
        assert_eq!(without_tags("x /* */"), "x /* */");
    }

    #[test]
    fn a_comment_with_a_stray_close_and_no_open_has_no_tags() {
        assert!(tags("odd */").is_empty());
    }

    #[test]
    fn tags_are_stored_as_a_json_array_of_names() {
        assert_eq!(tags_json("x /* A */"), r#"["A"]"#);
        assert_eq!(tags_json(r#"/* say "hi" */"#), r#"["say \"hi\""]"#);
    }
}
