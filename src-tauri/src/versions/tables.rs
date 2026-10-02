//! The pattern data: every label spelling and junk pattern the parser
//! knows, in one place.
//!
//! To teach the parser a convention a user reported, add a row here and a
//! row to the test corpus (`tests/corpus.rs`); a test fails if a spelling
//! here has no corpus row.
//!
//! Spellings are matched whole words at a time, ignoring case, dots,
//! apostrophes and hyphens inside a word: "V.I.P." matches `vip`, "Re-Fix"
//! matches `refix`, "Intro-Outro" matches `intro-outro`.

use super::MarkerKind;

/// Cut labels (ROADMAP §1.5) and their spellings, record-pool ones
/// included. Pools combine them freely ("Intro - Clean", "Clean Extended",
/// "Quick Hit Dirty"); the parser reads each label in a combination.
pub const CUT_LABELS: &[(MarkerKind, &[&str])] = &[
    (
        MarkerKind::Original,
        &[
            "original",
            "original mix",
            "original version",
            "original edit",
            "orig",
            "orig mix",
            "og",
            "og mix",
            "og version",
        ],
    ),
    (
        MarkerKind::Extended,
        &[
            "extended",
            "extended mix",
            "extended version",
            "extended edit",
            "extended cut",
            "ext",
            "ext mix",
            "ext version",
            "extd",
            "xtd",
        ],
    ),
    (
        MarkerKind::RadioEdit,
        &[
            "radio edit",
            "radio mix",
            "radio version",
            "radio cut",
            "radio",
        ],
    ),
    (
        MarkerKind::ClubEdit,
        &["club edit", "club mix", "club version", "club cut", "club"],
    ),
    (
        MarkerKind::Short,
        &[
            "short",
            "short edit",
            "short mix",
            "short version",
            "short cut",
        ],
    ),
    (
        MarkerKind::Clean,
        &[
            "clean",
            "clean edit",
            "clean mix",
            "clean version",
            "clean cut",
            "cln",
        ],
    ),
    (
        MarkerKind::Dirty,
        &[
            "dirty",
            "dirty edit",
            "dirty mix",
            "dirty version",
            "dirty cut",
            "explicit",
            "explicit version",
            "drt",
        ],
    ),
    (
        MarkerKind::Intro,
        &[
            "intro",
            "intro edit",
            "intro mix",
            "intro version",
            "dj intro",
            "with intro",
        ],
    ),
    (
        MarkerKind::Outro,
        &[
            "outro",
            "outro edit",
            "outro mix",
            "outro version",
            "dj outro",
            "with outro",
        ],
    ),
    (
        MarkerKind::IntroOutro,
        &[
            "intro-outro",
            "intro outro",
            "intro-outro edit",
            "intro outro edit",
            "in-out",
            "in out",
            "dj intro outro",
        ],
    ),
    (
        MarkerKind::QuickHit,
        &["quick hit", "quickhit", "quick hitter", "qh"],
    ),
    // A plain "(Edit)". With a name in front it is a bootleg by that name
    // ("(Vey Sun Edit)", "(@handle edit)"): the parser does that by rule.
    (MarkerKind::Edit, &["edit"]),
    // A year beside it ("2019 Remaster", "Remastered 2019") is read with it.
    (
        MarkerKind::Remaster,
        &[
            "remaster",
            "remastered",
            "remastered version",
            "remaster version",
        ],
    ),
];

/// Rework labels (ROADMAP §1.5). A name in front of one is its detail:
/// "(Quill Ashby Remix)" is a remix by Quill Ashby.
pub const REWORK_LABELS: &[(MarkerKind, &[&str])] = &[
    (
        MarkerKind::Remix,
        &["remix", "rmx", "remixed", "re-mix", "remix edit"],
    ),
    (
        MarkerKind::Vip,
        &["vip", "vip mix", "vip edit", "vip version"],
    ),
    (MarkerKind::Flip, &["flip", "flipped"]),
    (
        MarkerKind::Bootleg,
        &[
            "bootleg",
            "bootleg mix",
            "bootleg edit",
            "boot",
            "booty",
            "re-edit",
        ],
    ),
    (
        MarkerKind::Rework,
        &["rework", "reworked", "refix", "re-fix", "reboot", "re-boot"],
    ),
    (
        MarkerKind::Dub,
        &["dub", "dub mix", "dub version", "dub edit"],
    ),
    (MarkerKind::Cover, &["cover", "cover version", "covered"]),
    (
        MarkerKind::Live,
        &[
            "live",
            "live version",
            "live mix",
            "live edit",
            "live recording",
        ],
    ),
    (
        MarkerKind::Alternate,
        &[
            "alternate",
            "alternate mix",
            "alternate version",
            "alternate take",
            "alternate arrangement",
            "alternative",
            "alternative mix",
            "alternative version",
            "alt",
            "alt mix",
            "alt version",
            "alt take",
        ],
    ),
    (
        MarkerKind::Instrumental,
        &[
            "instrumental",
            "instrumental mix",
            "instrumental version",
            "inst",
            "instr",
        ],
    ),
    (
        MarkerKind::Acapella,
        &[
            "acapella",
            "acappella",
            "accapella",
            "a cappella",
            "acapella version",
            "acap",
        ],
    ),
    (MarkerKind::Mashup, &["mashup", "mash-up", "mash up"]),
];

/// The longest label spelling, in words.
pub const MAX_LABEL_WORDS: usize = 3;

/// "(Some Name Mix)": a name and one of these words is a remix by that
/// name, unless the words are a cut's spelling ("Extended Mix").
pub const NAMED_MIX_WORDS: &[&str] = &["mix"];

/// A rework label can be followed by one of these and a name:
/// "(Remix by Quill Ashby)", "(Cover by The Marrow Choir)".
pub const BY_WORDS: &[&str] = &["by"];

/// "Live" can be followed by one of these and a place:
/// "(Live at Harbor Hall)".
pub const LIVE_PLACE_WORDS: &[&str] = &["at", "from", "in", "@"];

/// Words that start a featuring credit.
pub const FEAT_WORDS: &[&str] = &["feat", "ft", "featuring"];

/// Words that join the parts of a mashup title ("A x B", "A vs B") and the
/// names in an artist credit.
pub const MASHUP_SEPARATORS: &[&str] = &["x", "vs", "versus"];

/// Characters that separate labels inside one bracket:
/// "(Clean / Intro)", "(Intro + Outro)".
pub const LABEL_SEPARATORS: &[char] = &['/', '+', '&', ',', '|', ';'];

/// Named HTML entities seen in store file names. Numeric ones (`&#40;`,
/// `&#x28;`) are decoded by rule.
pub const HTML_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("quot", '"'),
    ("apos", '\''),
    ("lt", '<'),
    ("gt", '>'),
    ("nbsp", ' '),
    ("lpar", '('),
    ("rpar", ')'),
    ("lsqb", '['),
    ("rsqb", ']'),
    ("ndash", '\u{2013}'),
    ("mdash", '\u{2014}'),
];

/// Endings that make a word look like a site's name ("ripsite.example").
/// Kept to endings that rarely end a real title word.
pub const SITE_ENDINGS: &[&str] = &[
    "com", "net", "org", "example", "test", "invalid", "io", "fm", "ru", "cc", "biz", "info",
    "xyz", "site", "online", "blog", "app", "uk", "de", "nl", "fr", "eu", "ca", "tv", "co",
];

/// Words that may sit beside a site's name in a prefix, suffix or bracket:
/// "Downloaded from ripsite.example".
pub const SITE_FILLER_WORDS: &[&str] = &[
    "downloaded",
    "download",
    "from",
    "via",
    "by",
    "at",
    "on",
    "ripped",
    "courtesy",
    "of",
    "visit",
    "free",
    "more",
    "music",
];

/// Bracket contents that are rip or upload tags, whole.
pub const RIP_TAGS: &[&str] = &[
    "free download",
    "free dl",
    "official audio",
    "official video",
    "official music video",
    "hq",
    "hd",
    "cdq",
    "web",
    "webrip",
    "web rip",
    "vinyl rip",
    "cd rip",
    "rip",
    "promo",
    "promo only",
    "retag",
    "retagged",
    "out now",
    "premiere",
];

/// Bitrates that, alone in a bracket, are a rip tag: "(320)", "[192kbps]".
pub const BITRATES: &[u32] = &[64, 96, 112, 128, 160, 192, 224, 256, 320];

/// Units that may follow a bitrate.
pub const BITRATE_UNITS: &[&str] = &["kbps", "kbit", "kb", "k"];

/// Format names that, alone in a bracket or beside a bitrate, are a rip
/// tag: "[FLAC]", "(MP3 320)".
pub const FORMAT_WORDS: &[&str] = &["mp3", "flac", "wav", "aiff", "aif", "m4a", "lossless"];

/// File extensions taken off a file name.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "aiff", "aif", "aifc", "m4a", "mp4", "aac", "ogg", "opus", "wma", "alac",
    "wv", "ape",
];
