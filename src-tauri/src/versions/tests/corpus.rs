//! The corpus: names, and the parse each must give. All invented, built on
//! the patterns of ROADMAP §1.5 and of `tools/fixture-gen`.
//!
//! A row reads: source and input, the base title, then whatever else the
//! parse must hold (everything not named must be empty):
//! `.cr` credits, `.m` markers, `.tm` label words kept in the title,
//! `.unk` brackets not understood, `.junk` junk, `.parts` mashup parts.
//! The lists are written as `show_markers` and friends print them.

use super::{parsed, show_credits, show_junk, show_markers, Src};

pub(super) struct Case {
    pub src: Src,
    pub input: &'static str,
    base: &'static str,
    credits: &'static str,
    markers: &'static str,
    title_markers: &'static str,
    unrecognized: &'static str,
    junk: &'static str,
    parts: &'static str,
}

const fn case(src: Src, input: &'static str, base: &'static str) -> Case {
    Case {
        src,
        input,
        base,
        credits: "",
        markers: "",
        title_markers: "",
        unrecognized: "",
        junk: "",
        parts: "",
    }
}

/// A title tag and its base title.
const fn t(input: &'static str, base: &'static str) -> Case {
    case(Src::T, input, base)
}

/// A file name and its base title.
const fn f(input: &'static str, base: &'static str) -> Case {
    case(Src::F, input, base)
}

impl Case {
    const fn cr(mut self, credits: &'static str) -> Case {
        self.credits = credits;
        self
    }
    const fn m(mut self, markers: &'static str) -> Case {
        self.markers = markers;
        self
    }
    const fn tm(mut self, title_markers: &'static str) -> Case {
        self.title_markers = title_markers;
        self
    }
    const fn unk(mut self, unrecognized: &'static str) -> Case {
        self.unrecognized = unrecognized;
        self
    }
    const fn junk(mut self, junk: &'static str) -> Case {
        self.junk = junk;
        self
    }
    const fn parts(mut self, parts: &'static str) -> Case {
        self.parts = parts;
        self
    }
}

pub(super) const CASES: &[Case] = &[
    // ---- Plain names ------------------------------------------------------
    t("Glasswing", "Glasswing"),
    t("  Glasswing  ", "Glasswing"),
    t("Paper Harbor", "Paper Harbor"),
    t("", ""),
    t("   ", ""),
    t("-", "-"),
    t("()", "()"),
    t("\u{2728}", "\u{2728}"),
    t("99 Lanterns", "99 Lanterns"),
    t("1.5 Hours", "1.5 Hours"),
    t("Nox & Vey", "Nox & Vey"),
    t("Q.V.X.", "Q.V.X."),
    t("Nemora Vale - Glasswing", "Nemora Vale - Glasswing"),
    f("Glasswing.wav", "Glasswing").junk("file_extension:.wav"),
    f("Glasswing", "Glasswing"),
    f(".mp3", ".mp3"),
    f("Track 01.wav", "Track 01").junk("file_extension:.wav"),
    f("7 Lanterns.mp3", "7 Lanterns").junk("file_extension:.mp3"),
    // ---- Artist and title in a file name ----------------------------------
    f("Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3"),
    f("Nemora Vale \u{2013} Glasswing.aiff", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.aiff"),
    f("Nemora Vale \u{2014} Glasswing.flac", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.flac"),
    f("Nemora Vale - Glasswing - Reprise.mp3", "Glasswing - Reprise")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3"),
    f("Nemora Vale & Halden Rook - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale | Halden Rook")
        .junk("file_extension:.mp3"),
    f("Nemora Vale x Halden Rook - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale | Halden Rook")
        .junk("file_extension:.mp3"),
    f("Nemora Vale vs. Halden Rook - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale | Halden Rook")
        .junk("file_extension:.mp3"),
    f("Nemora Vale, Halden Rook - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale | Halden Rook")
        .junk("file_extension:.mp3"),
    f("Nemora Vale feat. Vey Sun - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale | +Vey Sun")
        .junk("file_extension:.mp3"),
    f("Nemora Vale (feat. Vey Sun) - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale | +Vey Sun")
        .junk("file_extension:.mp3"),
    f("Nemora Vale ft Vey Sun & Tovi Ash - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale | +Vey Sun | +Tovi Ash")
        .junk("file_extension:.mp3"),
    f("Nemora Vale/Lanterns EP/Glasswing.flac", "Glasswing")
        .junk("folder_path:Nemora Vale/Lanterns EP/ | file_extension:.flac"),
    f("C:\\Music\\Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("folder_path:C:\\Music\\ | file_extension:.mp3"),
    // ---- Featuring credits -------------------------------------------------
    t("Glasswing feat. Vey Sun", "Glasswing").cr("+Vey Sun"),
    t("Glasswing ft. Vey Sun", "Glasswing").cr("+Vey Sun"),
    t("Glasswing Ft Vey Sun", "Glasswing").cr("+Vey Sun"),
    t("Glasswing featuring Vey Sun", "Glasswing").cr("+Vey Sun"),
    t("Glasswing (feat. Vey Sun)", "Glasswing").cr("+Vey Sun"),
    t("Glasswing (ft. Vey Sun)", "Glasswing").cr("+Vey Sun"),
    t("Glasswing (Feat Vey Sun)", "Glasswing").cr("+Vey Sun"),
    t("Glasswing [Featuring Vey Sun]", "Glasswing").cr("+Vey Sun"),
    t("Glasswing (feat. Vey Sun, Nox & Tovi Ash)", "Glasswing").cr("+Vey Sun | +Nox | +Tovi Ash"),
    t("Glasswing (feat. Vey Sun) (Extended Mix)", "Glasswing")
        .cr("+Vey Sun")
        .m("extended"),
    t("Glasswing (Extended Mix) [feat. Vey Sun]", "Glasswing")
        .cr("+Vey Sun")
        .m("extended"),
    t("Glasswing (Extended Mix) feat. Vey Sun", "Glasswing")
        .cr("+Vey Sun")
        .m("extended"),
    t("Glasswing featuring Vey Sun (Extended Mix)", "Glasswing")
        .cr("+Vey Sun")
        .m("extended"),
    t("Glasswing ft. Vey Sun & Nox (Quill Ashby Remix)", "Glasswing")
        .cr("+Vey Sun | +Nox")
        .m("remix=Quill Ashby"),
    t("Featuring You", "Featuring You"),
    t("Feat", "Feat"),
    // ---- Brackets: kinds, nesting, unbalanced -----------------------------
    t("Paper Harbor (Extended Mix)", "Paper Harbor").m("extended"),
    t("Paper Harbor [Extended Mix]", "Paper Harbor").m("extended"),
    t("Paper Harbor {Extended Mix}", "Paper Harbor").m("extended"),
    t("Paper Harbor \u{FF08}Extended Mix\u{FF09}", "Paper Harbor").m("extended"),
    t("Paper Harbor \u{3010}Extended Mix\u{3011}", "Paper Harbor").m("extended"),
    t("Paper Harbor \u{FF3B}Clean\u{FF3D}", "Paper Harbor").m("clean"),
    t("Paper Harbor \u{FF5B}Dub\u{FF5D}", "Paper Harbor").m("dub"),
    t("Paper Harbor \u{300A}Live\u{300B}", "Paper Harbor").m("live"),
    t("Paper Harbor(Extended Mix)", "Paper Harbor").m("extended"),
    t("Paper Harbor (Extended Mix", "Paper Harbor").m("extended"),
    t("Paper Harbor [Quill Ashby Remix", "Paper Harbor").m("remix=Quill Ashby"),
    t("Paper Harbor (Extended Mix]", "Paper Harbor").m("extended"),
    t("Paper Harbor ((Extended Mix))", "Paper Harbor").m("extended"),
    t("Paper Harbor (Quill Ashby Remix (Extended))", "Paper Harbor")
        .m("remix=Quill Ashby | extended"),
    t("Paper Harbor (Quill Ashby Remix [Clean])", "Paper Harbor").m("remix=Quill Ashby | clean"),
    t("Paper Harbor [Extended (Mix]", "Paper Harbor")
        .m("extended")
        .unk("(Mix"),
    t("Paper Harbor Extended Mix)", "Paper Harbor Extended Mix)").tm("extended?"),
    t("(I Can't) Wait (Radio Edit)", "(I Can't) Wait").m("radio_edit"),
    t("Wait (For Me) Now", "Wait (For Me) Now"),
    t("Wait (For Me) Now (Clean)", "Wait (For Me) Now").m("clean"),
    t("[RipCrew] Glasswing", "[RipCrew] Glasswing"),
    t("Paper Harbor (Extended Mix).", "Paper Harbor").m("extended"),
    t("Paper Harbor  ( Extended   Mix )", "Paper Harbor").m("extended"),
    // ---- Unicode -----------------------------------------------------------
    t("Cafe\u{301} Vireo", "Caf\u{e9} Vireo"),
    t("Caf\u{e9} Vireo (Extended Mix)", "Caf\u{e9} Vireo").m("extended"),
    t("\u{9752}\u{3044}\u{56DE}\u{8DEF} (Extended Mix)", "\u{9752}\u{3044}\u{56DE}\u{8DEF}")
        .m("extended"),
    t(
        "\u{591C}\u{306E}\u{4FE1}\u{53F7}\u{FF08}Quill Ashby Remix\u{FF09}",
        "\u{591C}\u{306E}\u{4FE1}\u{53F7}",
    )
    .m("remix=Quill Ashby"),
    t("\u{C0C8}\u{BCBD} \u{D68C}\u{B85C} (Clean)", "\u{C0C8}\u{BCBD} \u{D68C}\u{B85C}").m("clean"),
    t("\u{1F525} Ignis \u{1F525} (Extended Mix)", "\u{1F525} Ignis \u{1F525}").m("extended"),
    t("\u{2728} (Extended Mix)", "\u{2728}").m("extended"),
    t("\u{201C}Glasswing\u{201D} (Extended Mix)", "\"Glasswing\"").m("extended"),
    t("Glasswing (Quill Ashby\u{2019}s Remix)", "Glasswing").m("remix=Quill Ashby's"),
    t("Glasswing\u{A0}(Extended\u{A0}Mix)", "Glasswing").m("extended"),
    t("Glasswing\u{3000}(Extended Mix)", "Glasswing").m("extended"),
    t("Glass\u{200B}wing (Extended Mix)", "Glasswing").m("extended"),
    // ---- " - " segments ----------------------------------------------------
    t("Glasswing - Extended Mix", "Glasswing").m("extended?"),
    t("Glasswing \u{2013} Extended Mix", "Glasswing").m("extended?"),
    t("Glasswing - Quill Ashby Remix", "Glasswing").m("remix=Quill Ashby?"),
    t("Glasswing - Radio Edit - Clean", "Glasswing").m("radio_edit? | clean?"),
    t("Glasswing - Reprise", "Glasswing - Reprise"),
    t("Neon Moth - Clean", "Neon Moth").m("clean?"),
    f("Nemora Vale - Glasswing - Extended Mix.wav", "Glasswing")
        .cr("Nemora Vale")
        .m("extended?")
        .junk("file_extension:.wav"),
    f("Solvane - Neon Moth - Intro - Dirty.mp3", "Neon Moth")
        .cr("Solvane")
        .m("intro? | dirty?")
        .junk("file_extension:.mp3"),
    f("Solvane - Neon Moth - Rivetta Remix.mp3", "Neon Moth")
        .cr("Solvane")
        .m("remix=Rivetta?")
        .junk("file_extension:.mp3"),
    // A file name's two segments are its artist and its title, whatever
    // they say.
    f("Paper Harbor - Extended Mix.mp3", "Extended Mix")
        .cr("Paper Harbor")
        .tm("extended?")
        .junk("file_extension:.mp3"),
    f("Kestrel Nine - Intro.mp3", "Intro")
        .cr("Kestrel Nine")
        .tm("intro?")
        .junk("file_extension:.mp3"),
    f("Nemora Vale - Glasswing - Extended Mix (Clean).mp3", "Glasswing - Extended Mix")
        .cr("Nemora Vale")
        .tm("extended?")
        .m("clean")
        .junk("file_extension:.mp3"),
    // ---- A version word can be the title ----------------------------------
    t("Velo (dirty)", "Velo").m("dirty"),
    t("Velo (dirty) (dirty)", "Velo (dirty)").tm("dirty?").m("dirty"),
    t("Velo (dirty) (Dirty)", "Velo (dirty)").tm("dirty?").m("dirty"),
    t("Velo (Clean) (Dirty)", "Velo").m("clean | dirty"),
    t("Intro", "Intro").tm("intro?"),
    t("Original", "Original").tm("original?"),
    t("Dub", "Dub").tm("dub?"),
    t("Extended Mix", "Extended Mix").tm("extended?"),
    t("(Intro)", "(Intro)").tm("intro?"),
    t("[Live]", "[Live]").tm("live?"),
    t("Come Clean", "Come Clean").tm("clean?"),
    t("Long Live", "Long Live").tm("live?"),
    t("Stay Dirty", "Stay Dirty").tm("dirty?"),
    t("Dirty Clean", "Dirty Clean").tm("dirty? | clean?"),
    t("Paper Harbor Extended Mix", "Paper Harbor Extended Mix").tm("extended?"),
    t("Paper Harbor Remix", "Paper Harbor Remix").tm("remix?"),
    t("Paper Harbor VIP", "Paper Harbor VIP").tm("vip?"),
    t("Neon Moth Clean", "Neon Moth Clean").tm("clean?"),
    t("Paper Harbor Extended Mix (Clean)", "Paper Harbor Extended Mix")
        .tm("extended?")
        .m("clean"),
    t("Come Clean (Clean)", "Come Clean").tm("clean?").m("clean"),
    t("Intro (Extended Mix)", "Intro").tm("intro?").m("extended"),
    // ---- Cuts and pool conventions ----------------------------------------
    t("Neon Moth (Dirty)", "Neon Moth").m("dirty"),
    t("Neon Moth (Clean)", "Neon Moth").m("clean"),
    t("Neon Moth (CLEAN)", "Neon Moth").m("clean"),
    t("Neon Moth (Explicit)", "Neon Moth").m("dirty"),
    t("Neon Moth (Intro - Clean)", "Neon Moth").m("intro | clean"),
    t("Neon Moth (Intro - Dirty)", "Neon Moth").m("intro | dirty"),
    t("Neon Moth (Clean Intro)", "Neon Moth").m("clean | intro"),
    t("Neon Moth (Dirty Intro)", "Neon Moth").m("dirty | intro"),
    t("Neon Moth (DJ Intro - Dirty)", "Neon Moth").m("intro | dirty"),
    t("Neon Moth (Clean / Extended)", "Neon Moth").m("clean | extended"),
    t("Neon Moth (Clean, Intro)", "Neon Moth").m("clean | intro"),
    t("Neon Moth (Clean Extended)", "Neon Moth").m("clean | extended"),
    t("Neon Moth (Dirty Extended Mix)", "Neon Moth").m("dirty | extended"),
    t("Neon Moth (Dirty - Short Edit)", "Neon Moth").m("dirty | short"),
    t("Neon Moth (Quick Hit Clean)", "Neon Moth").m("quick_hit | clean"),
    t("Neon Moth (QH Dirty)", "Neon Moth").m("quick_hit | dirty"),
    t("Neon Moth (Clean) (Intro)", "Neon Moth").m("clean | intro"),
    t("Neon Moth (Radio Edit) [Clean]", "Neon Moth").m("radio_edit | clean"),
    t("Neon Moth [Clean] [Short Edit]", "Neon Moth").m("clean | short"),
    t("Neon Moth (Quick Hit) (Clean)", "Neon Moth").m("quick_hit | clean"),
    t("Neon Moth (Intro/Outro)", "Neon Moth").m("intro_outro"),
    t("Neon Moth (Intro + Outro)", "Neon Moth").m("intro_outro"),
    t("Neon Moth (Intro & Outro)", "Neon Moth").m("intro_outro"),
    t("Neon Moth (Intro - Outro)", "Neon Moth").m("intro_outro"),
    t("Neon Moth (Intro Outro - Clean)", "Neon Moth").m("intro_outro | clean"),
    t("Neon Moth (Clean Intro-Outro)", "Neon Moth").m("clean | intro_outro"),
    t("Neon Moth (Original Mix) (Clean)", "Neon Moth").m("original | clean"),
    t("Neon Moth (Extended Original Mix)", "Neon Moth").m("extended | original"),
    t("Neon Moth (Clean Clean)", "Neon Moth").m("clean"),
    f("Solvane - Neon Moth (Clean) (Intro).mp3", "Neon Moth")
        .cr("Solvane")
        .m("clean | intro")
        .junk("file_extension:.mp3"),
    f("Solvane ft. Tovi Ash - Neon Moth (Clean Intro).mp3", "Neon Moth")
        .cr("Solvane | +Tovi Ash")
        .m("clean | intro")
        .junk("file_extension:.mp3"),
    f("Solvane_-_Neon_Moth_(Clean)_(Extended).mp3", "Neon Moth")
        .cr("Solvane")
        .m("clean | extended")
        .junk("file_extension:.mp3 | underscores:_"),
    f("Solvane - Neon Moth - Clean.mp3", "Neon Moth")
        .cr("Solvane")
        .m("clean?")
        .junk("file_extension:.mp3"),
    f("Solvane/Neon Moth (Dirty).mp3", "Neon Moth")
        .m("dirty")
        .junk("folder_path:Solvane/ | file_extension:.mp3"),
    // A label word with other words isn't a label.
    t("Neon Moth (Something Clean)", "Neon Moth").unk("(Something Clean)"),
    t("Neon Moth (Rivetta Extended)", "Neon Moth").unk("(Rivetta Extended)"),
    // ---- Reworks -----------------------------------------------------------
    t("Paper Harbor (Quill Ashby Remix)", "Paper Harbor").m("remix=Quill Ashby"),
    t("Paper Harbor (quill ashby remix)", "Paper Harbor").m("remix=quill ashby"),
    t("Paper Harbor (Remix)", "Paper Harbor").m("remix"),
    t("Paper Harbor (Quill Ashby RMX)", "Paper Harbor").m("remix=Quill Ashby"),
    t("Paper Harbor (Remix by Quill Ashby)", "Paper Harbor").m("remix=Quill Ashby"),
    t("Paper Harbor (Remixed by Quill Ashby)", "Paper Harbor").m("remix=Quill Ashby"),
    t("Paper Harbor (Nox & Vey Remix)", "Paper Harbor").m("remix=Nox & Vey"),
    t("Paper Harbor (2024 Remix)", "Paper Harbor").m("remix=2024"),
    t("Paper Harbor (Dub Unit Remix)", "Paper Harbor").m("remix=Dub Unit"),
    t("Paper Harbor (Quill Ashby Extended Remix)", "Paper Harbor")
        .m("extended | remix=Quill Ashby"),
    t("Paper Harbor (Quill Ashby Remix - Clean)", "Paper Harbor").m("remix=Quill Ashby | clean"),
    t("Paper Harbor (Quill Ashby Remix) (Extended)", "Paper Harbor")
        .m("remix=Quill Ashby | extended"),
    t("Paper Harbor (Quill Ashby Remix) [Clean] (Intro)", "Paper Harbor")
        .m("remix=Quill Ashby | clean | intro"),
    t("Paper Harbor (Rivetta Remix) (Flint Remix)", "Paper Harbor")
        .m("remix=Rivetta | remix=Flint"),
    t("Paper Harbor (Quill Ashby VIP Remix)", "Paper Harbor")
        .m("vip=Quill Ashby | remix=Quill Ashby"),
    t("Paper Harbor (VIP)", "Paper Harbor").m("vip"),
    t("Paper Harbor (V.I.P.)", "Paper Harbor").m("vip"),
    t("Paper Harbor (Kestrel Nine VIP)", "Paper Harbor").m("vip=Kestrel Nine"),
    t("Paper Harbor [Vey Sun Flip]", "Paper Harbor").m("flip=Vey Sun"),
    t("Paper Harbor (Flip by Vey Sun)", "Paper Harbor").m("flip=Vey Sun"),
    t("Paper Harbor (@lumenlark Flip)", "Paper Harbor").m("flip=@lumenlark"),
    t("Paper Harbor (@lumenlark edit)", "Paper Harbor").m("bootleg=@lumenlark"),
    t("Paper Harbor (@lumenlark Edit)", "Paper Harbor").m("bootleg=@lumenlark"),
    t("Paper Harbor [@lumenlark Re-Edit]", "Paper Harbor").m("bootleg=@lumenlark"),
    t("Korvex (Flint Bootleg)", "Korvex").m("bootleg=Flint"),
    t("Korvex (Bootleg)", "Korvex").m("bootleg"),
    t("Korvex (Flint Bootleg Remix)", "Korvex").m("bootleg=Flint | remix=Flint"),
    t("Korvex (Flint Rework)", "Korvex").m("rework=Flint"),
    t("Korvex (Flint Refix)", "Korvex").m("rework=Flint"),
    t("Korvex (Flint Re-Fix)", "Korvex").m("rework=Flint"),
    t("Korvex (Flint Reboot)", "Korvex").m("rework=Flint"),
    t("Korvex (Dub)", "Korvex").m("dub"),
    t("Korvex (Dub Mix)", "Korvex").m("dub"),
    t("Korvex (Flint Dub)", "Korvex").m("dub=Flint"),
    t("Paper Harbor (Cover)", "Paper Harbor").m("cover"),
    t("Paper Harbor (The Marrow Choir Cover)", "Paper Harbor").m("cover=The Marrow Choir"),
    t("Paper Harbor (Cover by The Marrow Choir)", "Paper Harbor").m("cover=The Marrow Choir"),
    t("Paper Harbor (Live)", "Paper Harbor").m("live"),
    t("Paper Harbor (Live at Harbor Hall)", "Paper Harbor").m("live=Harbor Hall"),
    t("Paper Harbor (Live from Harbor Hall)", "Paper Harbor").m("live=Harbor Hall"),
    t("Paper Harbor (Live in Port Ellery)", "Paper Harbor").m("live=Port Ellery"),
    t("Paper Harbor (Live @ Harbor Hall)", "Paper Harbor").m("live=Harbor Hall"),
    t("Paper Harbor (Live Version)", "Paper Harbor").m("live"),
    t("Paper Harbor (Alternate Mix)", "Paper Harbor").m("alternate"),
    t("Paper Harbor (Alt Take)", "Paper Harbor").m("alternate"),
    t("Paper Harbor (Alternative Version)", "Paper Harbor").m("alternate"),
    f("Quill Ashby/Paper Harbor (Quill Ashby Remix).mp3", "Paper Harbor")
        .m("remix=Quill Ashby")
        .junk("folder_path:Quill Ashby/ | file_extension:.mp3"),
    f("Quill Ashby - Paper Harbor (Quill Ashby Remix).mp3", "Paper Harbor")
        .cr("Quill Ashby")
        .m("remix=Quill Ashby")
        .junk("file_extension:.mp3"),
    f("Edits/Paper Harbor [Vey Sun Flip].mp3", "Paper Harbor")
        .m("flip=Vey Sun")
        .junk("folder_path:Edits/ | file_extension:.mp3"),
    f("Edits/Paper Harbor (@lumenlark edit).mp3", "Paper Harbor")
        .m("bootleg=@lumenlark")
        .junk("folder_path:Edits/ | file_extension:.mp3"),
    // ---- Edits: plain is a cut, named is a bootleg (owner, 2026-10-02) ----
    t("Glasswing (Edit)", "Glasswing").m("edit"),
    t("Glasswing [EDIT]", "Glasswing").m("edit"),
    t("Glasswing (Edit) (Clean)", "Glasswing").m("edit | clean"),
    t("Glasswing (Re-Edit)", "Glasswing").m("bootleg"),
    t("Glasswing (Reedit)", "Glasswing").m("bootleg"),
    t("Glasswing (Vey Sun Edit)", "Glasswing").m("bootleg=Vey Sun"),
    t("Glasswing (Vey Sun Re-Edit)", "Glasswing").m("bootleg=Vey Sun"),
    t("Glasswing (Vey Sun Edit - Clean)", "Glasswing").m("bootleg=Vey Sun | clean"),
    t("Glasswing - Vey Sun Edit", "Glasswing").m("bootleg=Vey Sun?"),
    t("Glasswing Edit", "Glasswing Edit").tm("edit?"),
    // ---- A named mix is a remix by that name ------------------------------
    t("Glasswing (Vey Sun Mix)", "Glasswing").m("remix=Vey Sun"),
    t("Glasswing [Skyline Mix]", "Glasswing").m("remix=Skyline"),
    t("Glasswing (Nox & Vey Mix)", "Glasswing").m("remix=Nox & Vey"),
    t("Glasswing - Vey Sun Mix", "Glasswing").m("remix=Vey Sun?"),
    // A cut's name isn't a remixer.
    t("Glasswing (Extended Mix)", "Glasswing").m("extended"),
    t("Glasswing (Original Mix)", "Glasswing").m("original"),
    t("Glasswing (Radio Mix)", "Glasswing").m("radio_edit"),
    t("Glasswing (Club Mix)", "Glasswing").m("club_edit"),
    // ---- Remaster, instrumental, acapella ---------------------------------
    t("Glasswing (Remaster)", "Glasswing").m("remaster"),
    t("Glasswing (Remastered)", "Glasswing").m("remaster"),
    t("Glasswing (2019 Remaster)", "Glasswing").m("remaster"),
    t("Glasswing (Remastered 2019)", "Glasswing").m("remaster"),
    t("Glasswing (2019 Remastered Version)", "Glasswing").m("remaster"),
    t("Glasswing (Extended Mix) [2019 Remaster]", "Glasswing").m("extended | remaster"),
    t("Glasswing (2019 Remaster - Clean)", "Glasswing").m("remaster | clean"),
    t("Glasswing - 2019 Remaster", "Glasswing").m("remaster?"),
    t("Glasswing (Instrumental)", "Glasswing").m("instrumental"),
    t("Glasswing (Instrumental Mix)", "Glasswing").m("instrumental"),
    t("Glasswing (Inst)", "Glasswing").m("instrumental"),
    t("Glasswing (Quill Ashby Remix Instrumental)", "Glasswing")
        .m("remix=Quill Ashby | instrumental=Quill Ashby"),
    t("Glasswing (Extended Instrumental)", "Glasswing").m("extended | instrumental"),
    t("Glasswing (Acapella)", "Glasswing").m("acapella"),
    t("Glasswing (A Cappella)", "Glasswing").m("acapella"),
    t("Glasswing (Acapella - Clean)", "Glasswing").m("acapella | clean"),
    t("Glasswing (Intro - Dirty) (Acapella)", "Glasswing").m("intro | dirty | acapella"),
    // The year belongs to the remaster, not to a remix beside it.
    t("Glasswing (2024 Remix)", "Glasswing").m("remix=2024"),
    // ---- Brackets not understood: kept, reported, never guessed -----------
    t("Glasswing (Part 2)", "Glasswing").unk("(Part 2)"),
    t("Lo+Fi Run, Pt. (2)", "Lo+Fi Run, Pt.").unk("(2)"),
    t("Glasswing (Mix)", "Glasswing").unk("(Mix)"),
    // Ordinary words aren't a remixer or an editor.
    t("Glasswing (Main Mix)", "Glasswing").unk("(Main Mix)"),
    t("Glasswing (Album Mix)", "Glasswing").unk("(Album Mix)"),
    t("Glasswing (Vocal Mix)", "Glasswing").unk("(Vocal Mix)"),
    t("Glasswing (Dance Mix)", "Glasswing").unk("(Dance Mix)"),
    t("Glasswing (12\" Mix)", "Glasswing").unk("(12\" Mix)"),
    t("Glasswing (12 Inch Mix)", "Glasswing").unk("(12 Inch Mix)"),
    t("Glasswing (DJ Edit)", "Glasswing").unk("(DJ Edit)"),
    t("Glasswing (Single Edit)", "Glasswing").unk("(Single Edit)"),
    t("Glasswing (Hype Edit)", "Glasswing").unk("(Hype Edit)"),
    t("Glasswing (Main Edit)", "Glasswing").unk("(Main Edit)"),
    t("Glasswing (Extended DJ Edit)", "Glasswing").unk("(Extended DJ Edit)"),
    t("Glasswing (Vocal Remix)", "Glasswing").unk("(Vocal Remix)"),
    t("Glasswing (Special Dub)", "Glasswing").unk("(Special Dub)"),
    t("Glasswing - Main Mix", "Glasswing - Main Mix"),
    // One real name among them is enough.
    t("Glasswing (DJ Flint Edit)", "Glasswing").m("bootleg=DJ Flint"),
    t("Glasswing (Flint Main Mix)", "Glasswing").m("remix=Flint Main"),
    t("Glasswing (Flint Vocal Remix)", "Glasswing").m("remix=Flint Vocal"),
    t("Glasswing (Skyline Session)", "Glasswing").unk("(Skyline Session)"),
    t("Glasswing (Rivetta Club Mix)", "Glasswing").unk("(Rivetta Club Mix)"),
    t("Glasswing (Vey Sun Radio Edit)", "Glasswing").unk("(Vey Sun Radio Edit)"),
    t("Glasswing (Vey Sun Remaster)", "Glasswing").unk("(Vey Sun Remaster)"),
    t("Glasswing (Interlude) (Extended Mix)", "Glasswing")
        .unk("(Interlude)")
        .m("extended"),
    t("Glasswing ()", "Glasswing").unk("()"),
    t("(Something)", "(Something)"),
    // ---- Mashups -----------------------------------------------------------
    t("Paper Harbor x Neon Moth (Vey Sun Mashup)", "Paper Harbor x Neon Moth")
        .m("mashup=Vey Sun")
        .parts("Paper Harbor | Neon Moth"),
    t("Duskline x Emberlow (Mashup)", "Duskline x Emberlow")
        .m("mashup")
        .parts("Duskline | Emberlow"),
    t("Duskline x Emberlow (Mash-Up)", "Duskline x Emberlow")
        .m("mashup")
        .parts("Duskline | Emberlow"),
    t("Duskline x Emberlow (Vey Sun Mash Up)", "Duskline x Emberlow")
        .m("mashup=Vey Sun")
        .parts("Duskline | Emberlow"),
    t("Duskline x Emberlow", "Duskline x Emberlow")
        .m("mashup?")
        .parts("Duskline | Emberlow"),
    t("Duskline X Emberlow", "Duskline X Emberlow")
        .m("mashup?")
        .parts("Duskline | Emberlow"),
    t("Duskline vs Emberlow", "Duskline vs Emberlow")
        .m("mashup?")
        .parts("Duskline | Emberlow"),
    t("Duskline vs. Emberlow", "Duskline vs. Emberlow")
        .m("mashup?")
        .parts("Duskline | Emberlow"),
    t("Duskline VS Emberlow", "Duskline VS Emberlow")
        .m("mashup?")
        .parts("Duskline | Emberlow"),
    t("Duskline versus Emberlow", "Duskline versus Emberlow")
        .m("mashup?")
        .parts("Duskline | Emberlow"),
    t("Paper Harbor x Neon Moth x Korvex", "Paper Harbor x Neon Moth x Korvex")
        .m("mashup?")
        .parts("Paper Harbor | Neon Moth | Korvex"),
    t("Paper Harbor vs. Neon Moth (Flint Bootleg)", "Paper Harbor vs. Neon Moth")
        .m("bootleg=Flint | mashup?")
        .parts("Paper Harbor | Neon Moth"),
    t("Paper Harbor x Neon Moth (Clean)", "Paper Harbor x Neon Moth")
        .m("clean | mashup?")
        .parts("Paper Harbor | Neon Moth"),
    f("Vey Sun - Paper Harbor x Neon Moth.mp3", "Paper Harbor x Neon Moth")
        .cr("Vey Sun")
        .m("mashup?")
        .parts("Paper Harbor | Neon Moth")
        .junk("file_extension:.mp3"),
    f("Edits/Duskline x Emberlow (Mashup).mp3", "Duskline x Emberlow")
        .m("mashup")
        .parts("Duskline | Emberlow")
        .junk("folder_path:Edits/ | file_extension:.mp3"),
    // An "x" at either end joins nothing.
    t("Planet X", "Planet X"),
    t("X Marks", "X Marks"),
    t("Glasswing (Mashup)", "Glasswing").m("mashup"),
    // ---- Junk: HTML entities ----------------------------------------------
    t("Orbit Line &#40;Original Mix&#41;", "Orbit Line")
        .m("original")
        .junk("html_entity:&#40; | html_entity:&#41;"),
    t("Orbit Line &#x28;Extended Mix&#x29;", "Orbit Line")
        .m("extended")
        .junk("html_entity:&#x28; | html_entity:&#x29;"),
    t("Nox &amp; Vey", "Nox & Vey").junk("html_entity:&amp;"),
    t("&quot;Glasswing&quot;", "\"Glasswing\"").junk("html_entity:&quot; | html_entity:&quot;"),
    t("Glasswing&nbsp;(Clean)", "Glasswing")
        .m("clean")
        .junk("html_entity:&nbsp;"),
    t("Nox &; Vey", "Nox &; Vey"),
    t("Nox &bogus; Vey", "Nox &bogus; Vey"),
    t("Nox &#0; Vey", "Nox &#0; Vey"),
    // ---- Junk: store IDs, underscores, URL escapes ------------------------
    f("10000001_Orbit_Line_&#40;Original Mix&#41;.mp3", "Orbit Line")
        .m("original")
        .junk("file_extension:.mp3 | html_entity:&#40; | html_entity:&#41; | store_id:10000001_ | underscores:_"),
    f("10000001_Orbit_Line.mp3", "Orbit Line")
        .junk("file_extension:.mp3 | store_id:10000001_ | underscores:_"),
    f("7654321-Orbit Line.mp3", "Orbit Line").junk("file_extension:.mp3 | store_id:7654321-"),
    f("7654321 Orbit Line.mp3", "Orbit Line").junk("file_extension:.mp3 | store_id:7654321 "),
    f("Halden_Rook_-_Orbit_Line.mp3", "Orbit Line")
        .cr("Halden Rook")
        .junk("file_extension:.mp3 | underscores:_"),
    t("Orbit_Line_(Extended_Mix)", "Orbit Line")
        .m("extended")
        .junk("underscores:_"),
    t("Orbit_Line and the long way home", "Orbit_Line and the long way home"),
    t("12345 Lanterns", "12345 Lanterns"),
    t("1234567 Lanterns", "1234567 Lanterns"),
    f("Late%20Night.mp3", "Late Night").junk("file_extension:.mp3 | url_escape:%20"),
    t("Late%20Night", "Late%20Night"),
    // ---- Junk: site names --------------------------------------------------
    f("RipSite.example - Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | site_name:RipSite.example"),
    f("Downloads/RipSite.example - Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("folder_path:Downloads/ | file_extension:.mp3 | site_name:RipSite.example"),
    f("www.ripsite.example - Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | site_name:www.ripsite.example"),
    f("Nemora Vale - Glasswing - RipSite.example.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | site_name:RipSite.example"),
    f("Nemora Vale - Glasswing (Extended Mix) - RipSite.example.mp3", "Glasswing")
        .cr("Nemora Vale")
        .m("extended")
        .junk("file_extension:.mp3 | site_name:RipSite.example"),
    f("Nemora Vale - Glasswing (Extended Mix) [RipSite.example].mp3", "Glasswing")
        .cr("Nemora Vale")
        .m("extended")
        .junk("file_extension:.mp3 | site_name:[RipSite.example]"),
    f("Nemora Vale - Glasswing (Downloaded from ripsite.example).mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | site_name:(Downloaded from ripsite.example)"),
    f("RipSite.example_Nemora_Vale_-_Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | underscores:_ | site_name:RipSite.example"),
    f("Glasswing ripsite.example.mp3", "Glasswing")
        .junk("file_extension:.mp3 | site_name:ripsite.example"),
    f("Orbit Line [ ripper.example ].mp3", "Orbit Line")
        .junk("file_extension:.mp3 | site_name:[ ripper.example ]"),
    t("Glasswing - RipSite.example", "Glasswing").junk("site_name:RipSite.example"),
    t("Glasswing [www.ripsite.example]", "Glasswing").junk("site_name:[www.ripsite.example]"),
    t("Glasswing (via ripsite.example)", "Glasswing").junk("site_name:(via ripsite.example)"),
    t("[RipSite.example] Glasswing", "Glasswing").junk("site_name:[RipSite.example]"),
    // A site name alone is the only title there is.
    t("ripsite.example", "ripsite.example"),
    // ---- Junk: rip tags ----------------------------------------------------
    t("Glasswing [ example-ripper ]", "Glasswing").junk("rip_tag:[ example-ripper ]"),
    t("[ example-ripper ] Glasswing", "Glasswing").junk("rip_tag:[ example-ripper ]"),
    t("Glasswing [ Vey Sun Flip ]", "Glasswing").m("flip=Vey Sun"),
    f("Nemora Vale - Glasswing (320).mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | rip_tag:(320)"),
    f("Glasswing (retag).mp3", "Glasswing").junk("file_extension:.mp3 | rip_tag:(retag)"),
    t("Glasswing (320kbps)", "Glasswing").junk("rip_tag:(320kbps)"),
    t("Glasswing (192 kbps)", "Glasswing").junk("rip_tag:(192 kbps)"),
    t("Glasswing [MP3 320]", "Glasswing").junk("rip_tag:[MP3 320]"),
    t("Glasswing [FLAC]", "Glasswing").junk("rip_tag:[FLAC]"),
    t("Glasswing (128k)", "Glasswing").junk("rip_tag:(128k)"),
    t("Glasswing (Free Download)", "Glasswing").junk("rip_tag:(Free Download)"),
    t("Glasswing [FREE DL]", "Glasswing").junk("rip_tag:[FREE DL]"),
    t("Glasswing (Official Audio)", "Glasswing").junk("rip_tag:(Official Audio)"),
    t("Glasswing [HQ]", "Glasswing").junk("rip_tag:[HQ]"),
    t("Glasswing (Vinyl Rip)", "Glasswing").junk("rip_tag:(Vinyl Rip)"),
    t("Glasswing (Out Now)", "Glasswing").junk("rip_tag:(Out Now)"),
    t("Glasswing (Extended Mix) (Free Download)", "Glasswing")
        .m("extended")
        .junk("rip_tag:(Free Download)"),
    t("Glasswing (Free Download) (Extended Mix)", "Glasswing")
        .m("extended")
        .junk("rip_tag:(Free Download)"),
    t("Glasswing (3:45)", "Glasswing").junk("duration:(3:45)"),
    t("Glasswing [05:12]", "Glasswing").junk("duration:[05:12]"),
    t("Glasswing (1:02:03)", "Glasswing").junk("duration:(1:02:03)"),
    // Not a bitrate, not a time: not junk.
    t("Glasswing (321)", "Glasswing").unk("(321)"),
    t("Glasswing (3:4)", "Glasswing").unk("(3:4)"),
    // ---- Junk: track numbers ----------------------------------------------
    f("01 Glasswing.flac", "Glasswing").junk("file_extension:.flac | track_number:01"),
    f("01. Glasswing.mp3", "Glasswing").junk("file_extension:.mp3 | track_number:01."),
    f("01.Glasswing.mp3", "Glasswing").junk("file_extension:.mp3 | track_number:01."),
    f("07-Glasswing.mp3", "Glasswing").junk("file_extension:.mp3 | track_number:07-"),
    f("3. Glasswing.mp3", "Glasswing").junk("file_extension:.mp3 | track_number:3."),
    f("3) Glasswing.mp3", "Glasswing").junk("file_extension:.mp3 | track_number:3)"),
    f("1-03 Glasswing.mp3", "Glasswing").junk("file_extension:.mp3 | track_number:1-03"),
    f("03 - Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | track_number:03 -"),
    f("3 - Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | track_number:3 -"),
    f("A1 - Nemora Vale - Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | track_number:A1 -"),
    f("01_Nemora_Vale_-_Glasswing.mp3", "Glasswing")
        .cr("Nemora Vale")
        .junk("file_extension:.mp3 | underscores:_ | track_number:01"),
    t("01 Glasswing", "Glasswing").junk("track_number:01"),
    // A number that may be an artist or a title is left alone.
    f("3 - Glasswing.mp3", "Glasswing")
        .cr("3")
        .junk("file_extension:.mp3"),
    f("A1 - Glasswing.mp3", "Glasswing")
        .cr("A1")
        .junk("file_extension:.mp3"),
    f("01.mp3", "01").junk("file_extension:.mp3"),
    // ---- Awkward names from the fixture generator -------------------------
    f(
        "#1 Hits & 100% Mixes/Nox & Vey #2 (100% Tape) + 'Edit'.mp3",
        "Nox & Vey #2 (100% Tape) + 'Edit'",
    )
    .tm("edit?")
    .junk("folder_path:#1 Hits & 100% Mixes/ | file_extension:.mp3"),
    f("Misc/CON.mp3", "CON").junk("folder_path:Misc/ | file_extension:.mp3"),
    f("Drift Unit /Halvane.flac", "Halvane").junk("folder_path:Drift Unit / | file_extension:.flac"),
    f("Nemora Vale/Lanterns EP/._01 Glasswing.flac", ". 01 Glasswing")
        .junk("folder_path:Nemora Vale/Lanterns EP/ | file_extension:.flac | underscores:_"),
];

/// What the parse of a case holds, in the corpus's own notation.
fn shown(case: &Case) -> [String; 7] {
    let parsed = parsed(case.src, case.input);
    [
        parsed.base_title.clone(),
        show_credits(&parsed.credits),
        show_markers(&parsed.markers),
        show_markers(&parsed.title_markers),
        parsed.unrecognized.join(" | "),
        show_junk(&parsed.junk),
        parsed.mashup_parts.join(" | "),
    ]
}

#[test]
fn every_corpus_name_parses_as_the_table_says() {
    const FIELDS: [&str; 7] = [
        "base title",
        "credits",
        "markers",
        "title markers",
        "unrecognized",
        "junk",
        "mashup parts",
    ];
    let mut wrong = Vec::new();
    for case in CASES {
        let expected = [
            case.base,
            case.credits,
            case.markers,
            case.title_markers,
            case.unrecognized,
            case.junk,
            case.parts,
        ];
        let got = shown(case);
        for (n, field) in FIELDS.iter().enumerate() {
            if got[n] != expected[n] {
                wrong.push(format!(
                    "{:?} {:?}: {field}: expected {:?}, got {:?}",
                    case.src, case.input, expected[n], got[n]
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn every_corpus_name_keeps_its_original_string() {
    for case in CASES {
        assert_eq!(parsed(case.src, case.input).original, case.input);
    }
}

#[test]
fn the_corpus_has_no_duplicate_rows() {
    let mut seen = std::collections::BTreeSet::new();
    for case in CASES {
        assert!(
            seen.insert((case.src == Src::F, case.input)),
            "twice: {:?}",
            case.input
        );
    }
}

#[test]
fn a_names_own_reading_comes_first_and_matches_its_fields() {
    for case in CASES {
        let parsed = parsed(case.src, case.input);
        let readings = parsed.readings();
        let own = &readings[0];
        assert_eq!(own.base_title, parsed.base_title, "{:?}", case.input);
        assert_eq!(own.markers, parsed.markers, "{:?}", case.input);
        assert_eq!(own.distance(), 0, "{:?}", case.input);
    }
}
