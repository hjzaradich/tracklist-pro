//! The starter tree. PROPOSED, not decided: a small, genre-neutral set of
//! broad branches for the owner to look over (a deeper pass comes later).
//! It is loaded only when the user asks (`edit::load_seed`); nothing is put
//! in a database by default. Names are unique across the tree.

/// `(branch, children)`; children are one level, in order.
const TREE: &[(&str, &[&str])] = &[
    (
        "Electronic",
        &[
            "House",
            "Techno",
            "Trance",
            "Drum & Bass",
            "Dubstep",
            "Garage",
            "Breaks",
            "Ambient",
        ],
    ),
    ("Hip-Hop", &["Trap", "Boom Bap"]),
    ("R&B", &["Soul", "Neo-Soul"]),
    ("Pop", &["Dance Pop", "Synth-Pop"]),
    ("Rock", &["Alternative", "Indie", "Punk", "Classic Rock"]),
    ("Latin", &["Reggaeton", "Salsa", "Bachata", "Cumbia"]),
    ("Throwbacks", &["80s", "90s", "2000s"]),
    ("Funk & Disco", &[]),
    ("Reggae & Dancehall", &[]),
    ("Afrobeats", &[]),
    ("Jazz", &[]),
];

/// Spellings that are common but are not a node's name: `(alias, node)`.
const ALIASES: &[(&str, &str)] = &[
    ("Hip Hop", "Hip-Hop"),
    ("Rap", "Hip-Hop"),
    ("RnB", "R&B"),
    ("DnB", "Drum & Bass"),
    ("Drum and Bass", "Drum & Bass"),
    ("EDM", "Electronic"),
    ("Dance", "Electronic"),
];

/// Every starter genre as `(parent name, name)`, branches first.
pub fn nodes() -> impl Iterator<Item = (Option<&'static str>, &'static str)> {
    let branches = TREE.iter().map(|(branch, _)| (None, *branch));
    let children = TREE
        .iter()
        .flat_map(|(branch, kids)| kids.iter().map(move |kid| (Some(*branch), *kid)));
    branches.chain(children)
}

/// The starter aliases, as `(alias, node name)`.
pub fn aliases() -> impl Iterator<Item = (&'static str, &'static str)> {
    ALIASES.iter().copied()
}
