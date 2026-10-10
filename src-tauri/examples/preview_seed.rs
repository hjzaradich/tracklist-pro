//! Makes the development preview's sample library (debug builds only; see
//! `src/preview.rs` and `scripts/preview.mjs`).
//!
//! Run by `npm run design`, never by hand: it writes into the preview's
//! fixed folder (`C:\dev\tracklist-pro-preview`) and nowhere else, and only
//! when that folder carries the marker the script puts there.
//!
//! What it makes, all invented (no real track, artist or library):
//!
//! - a music folder of small generated WAV files with made-up tags, in
//!   genre, artist and album folders: about 250 tracks, a few with very
//!   long titles and artists, some untagged, some with the same audio
//!   under two names (duplicates), some versions of one track;
//! - a rekordbox export for most of them, with BPM, key, play counts and
//!   playlists, and a few tracks whose files were never here;
//! - the app's own state, made by its own commands as the end-to-end tests
//!   drive them: the folder added and scanned, the export read, tracks
//!   added to the Library, crates (one of them empty), some files then
//!   deleted so the Missing list has rows, and a send written.
//!
//! Some things stay empty on purpose (Activity, the Review pages that need
//! a conflict) so the empty states can be designed too.
//!
//! The generated files are WAV only, mono, a few seconds long: enough for
//! every list, not for format or duration variety.

#[cfg(not(windows))]
fn main() {
    eprintln!("preview_seed: the preview is for Windows only");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    use std::path::Path;
    use tracklist_pro_lib::preview::{Preview, ROOT};

    let preview = match Preview::at(Path::new(ROOT)) {
        Ok(p) => p,
        Err(refusal) => {
            eprintln!("preview_seed: not seeding, because {refusal}");
            std::process::exit(2);
        }
    };
    match seeding::seed(&preview) {
        Ok(summary) => println!("{summary}"),
        Err(e) => {
            eprintln!("preview_seed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(windows)]
mod seeding {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{mpsc, Mutex};
    use std::time::Duration;

    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};
    use tauri::test::{mock_builder, MockRuntime};
    use tauri::Manager;
    use tauri_specta::Event;
    use tracklist_pro_lib::jobs::{self, ActivitySnapshot, JobId, JobStatus};
    use tracklist_pro_lib::preview::{Preview, SEEDED};
    use tracklist_pro_lib::{db, scan};

    /// One file of the sample music folder.
    #[derive(Debug, Clone)]
    pub struct Sample {
        /// Inside the music folder, `/`-separated.
        pub rel: String,
        pub title: Option<String>,
        pub artist: Option<String>,
        pub genre: Option<String>,
        pub album: Option<String>,
        /// Two samples with the same `audio` are the same audio.
        pub audio: u32,
        /// Added to the Library.
        pub in_library: bool,
        /// Deleted after the first scan, so the Library shows it missing.
        pub gone: bool,
        /// rekordbox has it, with its BPM and key.
        pub rekordbox: Option<(String, String)>,
    }

    impl Sample {
        fn file_name(&self) -> &str {
            self.rel.rsplit('/').next().unwrap()
        }
    }

    /// A small deterministic generator, so the same sample is made every
    /// time.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn pick<'a>(&mut self, list: &[&'a str]) -> &'a str {
            list[self.below(list.len())]
        }
    }

    const FIRST: [&str; 24] = [
        "Ar", "Bel", "Cor", "Dun", "El", "Fen", "Gal", "Hol", "Ira", "Jun", "Kes", "Lun", "Mar",
        "Nov", "Os", "Pel", "Quin", "Ros", "Sol", "Tev", "Ul", "Vel", "Wen", "Yar",
    ];
    const SECOND: [&str; 12] = [
        "vane", "mere", "drow", "lith", "wick", "senn", "marsh", "torr", "bin", "quay", "nelle",
        "vik",
    ];
    const TITLE_A: [&str; 16] = [
        "Tovala", "Brindle", "Quenlo", "Marvesk", "Sibera", "Oltane", "Fennick", "Dray", "Ulmara",
        "Pell", "Castrel", "Hovern", "Zimble", "Aurex", "Nolliv", "Taspen",
    ];
    const TITLE_B: [&str; 12] = [
        "Drift", "Hours", "Room", "Lines", "Bloom", "Static", "Weather", "Steps", "Glow", "Return",
        "Fold", "Season",
    ];
    const GENRES: [&str; 9] = [
        "House",
        "Deep House",
        "Techno",
        "Disco",
        "Garage",
        "Breaks",
        "Drum & Bass",
        "Electro",
        "Ambient",
    ];
    const KEYS: [&str; 24] = [
        "1A", "2A", "3A", "4A", "5A", "6A", "7A", "8A", "9A", "10A", "11A", "12A", "1B", "2B",
        "3B", "4B", "5B", "6B", "7B", "8B", "9B", "10B", "11B", "12B",
    ];
    const VERSIONS: [&str; 4] = ["Extended Mix", "Radio Edit", "Dub Mix", "Club Mix"];

    /// How many ordinary tracks the plan has, before the special ones.
    const ORDINARY: usize = 215;

    fn artist_name(rng: &mut Lcg) -> String {
        let name = format!("{}{}", rng.pick(&FIRST), rng.pick(&SECOND));
        match rng.below(6) {
            0 => format!("{name} & {}{}", rng.pick(&FIRST), rng.pick(&SECOND)),
            1 => format!("{name} Trio"),
            _ => name,
        }
    }

    fn title_name(rng: &mut Lcg) -> String {
        match rng.below(3) {
            0 => rng.pick(&TITLE_A).to_owned(),
            _ => format!("{} {}", rng.pick(&TITLE_A), rng.pick(&TITLE_B)),
        }
    }

    /// Every file of the sample, in the order they're made.
    pub fn plan() -> Vec<Sample> {
        let mut rng = Lcg(20_261_010);
        let artists: Vec<String> = (0..34).map(|_| artist_name(&mut rng)).collect();
        let mut plan: Vec<Sample> = Vec::new();
        let mut used: BTreeSet<String> = BTreeSet::new();
        let mut audio = 1000;

        let rekordbox = |rng: &mut Lcg, known: bool| -> Option<(String, String)> {
            known.then(|| {
                let bpm = 84 + rng.below(94);
                (format!("{bpm}.00"), rng.pick(&KEYS).to_owned())
            })
        };

        // Ordinary tracks: tagged, in Genre/Artist/Album folders.
        let mut i = 0;
        while plan.len() < ORDINARY {
            i += 1;
            let artist = artists[rng.below(artists.len())].clone();
            let title = title_name(&mut rng);
            if !used.insert(format!("{artist}|{title}")) {
                continue;
            }
            let genre = GENRES[(i + rng.below(3)) % GENRES.len()].to_owned();
            let album = format!("{} {}", rng.pick(&TITLE_A), rng.pick(&TITLE_B));
            let file = format!("{artist} - {title}.wav");
            audio += 1;
            let in_library = rng.below(100) < 72;
            let known = rng.below(100) < 62;
            plan.push(Sample {
                rel: format!("{genre}/{artist}/{album}/{file}"),
                title: Some(title),
                artist: Some(artist),
                genre: Some(genre),
                album: Some(album),
                audio,
                in_library,
                gone: false,
                rekordbox: rekordbox(&mut rng, known),
            });
        }

        // Versions: one track in several cuts, under one artist.
        for (n, base) in plan.clone().iter().take(8).enumerate() {
            let version = VERSIONS[n % VERSIONS.len()];
            let title = format!("{} ({version})", base.title.as_ref().unwrap());
            let artist = base.artist.clone().unwrap();
            if !used.insert(format!("{artist}|{title}")) {
                continue;
            }
            audio += 1;
            let known = rng.below(100) < 50;
            plan.push(Sample {
                rel: format!(
                    "{}/{artist}/Singles/{artist} - {title}.wav",
                    base.genre.as_ref().unwrap()
                ),
                title: Some(title),
                artist: Some(artist),
                genre: base.genre.clone(),
                album: Some("Singles".into()),
                audio,
                in_library: n % 2 == 0,
                gone: false,
                rekordbox: rekordbox(&mut rng, known),
            });
        }

        // Duplicates: the same audio under another name or folder.
        for (n, base) in plan.clone().iter().skip(20).take(7).enumerate() {
            let mut copy = base.clone();
            let name = base.file_name().trim_end_matches(".wav").to_owned();
            copy.rel = match n % 3 {
                0 => format!("Downloads/{name} (1).wav"),
                1 => format!("Old backup/{name}.wav"),
                _ => format!("Downloads/Copy of {name}.wav"),
            };
            copy.in_library = false;
            copy.rekordbox = None;
            copy.gone = false;
            plan.push(copy);
        }

        // Very long titles and artists, and awkward characters.
        let long = [
            (
                "A Very Long Title That Keeps Going Well Past Every Reasonable Column Width Until Something Has To Give (Extended Club Mix With The Long Intro) [Remastered Edition]",
                "The Extremely Long Named Ensemble Of Many Collaborators feat. Another Very Long Guest Artist & Yet More Friends",
                "Long names",
            ),
            (
                "Short Title",
                "An Artist Whose Name Alone Is Longer Than Most Titles In This Library Put Together",
                "Long names",
            ),
            (
                "Ünïcödé Tïtle – with “quotes”, an ellipsis… and a #hash & ampersand (Dub)",
                "Zoë Ñandú & Ångström",
                "Awkward characters",
            ),
            (
                "日本語のタイトル (Extended Mix)",
                "Test Artist",
                "Awkward characters",
            ),
        ];
        for (n, (title, artist, album)) in long.iter().enumerate() {
            audio += 1;
            plan.push(Sample {
                rel: format!("Long names/{album}/Long name sample {}.wav", n + 1),
                title: Some((*title).into()),
                artist: Some((*artist).into()),
                genre: Some("House".into()),
                album: Some((*album).into()),
                audio,
                in_library: true,
                gone: false,
                rekordbox: Some((format!("{}.50", 120 + n * 3), KEYS[n * 5].into())),
            });
        }

        // Untagged files: only a file name to show.
        for n in 1..=5 {
            audio += 1;
            plan.push(Sample {
                rel: format!("Unsorted/track {n:02}.wav"),
                title: None,
                artist: None,
                genre: None,
                album: None,
                audio,
                in_library: n <= 2,
                gone: false,
                rekordbox: None,
            });
        }

        // Files that go missing after the first scan: in the Library, and
        // some known to rekordbox. Never one with a duplicate, so the
        // Library track keeps a file it can name.
        let mut audio_count: BTreeMap<u32, usize> = BTreeMap::new();
        for sample in &plan {
            *audio_count.entry(sample.audio).or_default() += 1;
        }
        let candidates: Vec<usize> = (0..plan.len())
            .filter(|&n| {
                let s = &plan[n];
                s.in_library
                    && s.title.is_some()
                    && audio_count[&s.audio] == 1
                    && !s.rel.starts_with("Long names")
            })
            .collect();
        for n in candidates.into_iter().step_by(5).take(12) {
            plan[n].gone = true;
        }
        plan
    }

    /// Entries rekordbox has whose files were never in the music folder.
    pub const NEVER_HERE: [(&str, &str, &str); 6] = [
        ("Gone/Orrery Hours.mp3", "Quillon", "Deep House"),
        ("Gone/Tessel (Dub Mix).mp3", "Hannevik", "Techno"),
        ("Gone/Fallow Season.flac", "Corvane Trio", "Ambient"),
        ("Gone/Brass Lantern.mp3", "Ulmarsh", "Disco"),
        ("Gone/Vellum.wav", "Osmere", "Garage"),
        ("Gone/Pale Fire Escape.mp3", "Wenwick & Yarvik", "Breaks"),
    ];

    /// A mono 16-bit WAV of quiet noise seeded by `audio`, a few seconds
    /// long, then its tags as a RIFF INFO list.
    fn wav(sample: &Sample) -> Vec<u8> {
        const RATE: u32 = 8000;
        let seconds = 3 + sample.audio % 3;
        let count = (RATE * seconds) as usize;
        let mut data = Vec::with_capacity(count * 2);
        let mut state = sample.audio.wrapping_mul(2_654_435_761).wrapping_add(1);
        for _ in 0..count {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = ((state >> 16) % 4000) as i16 - 2000;
            data.extend_from_slice(&v.to_le_bytes());
        }

        let tags = [
            (b"INAM", sample.title.as_deref()),
            (b"IART", sample.artist.as_deref()),
            (b"IGNR", sample.genre.as_deref()),
            (b"IPRD", sample.album.as_deref()),
        ];
        let mut info = b"INFO".to_vec();
        for (id, text) in tags {
            let Some(text) = text else { continue };
            info.extend_from_slice(id);
            info.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
            info.extend_from_slice(text.as_bytes());
            info.push(0);
            if info.len() % 2 == 1 {
                info.push(0);
            }
        }

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        let list = if info.len() > 4 { 8 + info.len() } else { 0 };
        bytes.extend_from_slice(&((4 + 24 + 8 + data.len() + list) as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&RATE.to_le_bytes());
        bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        if list > 0 {
            bytes.extend_from_slice(b"LIST");
            bytes.extend_from_slice(&(info.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&info);
        }
        bytes
    }

    /// A path as rekordbox 7 writes a `Location` (ROADMAP §5.3): `# ( ) ,
    /// + !` raw, anything else outside letters, digits and `- . _ ~ / :`
    /// as lowercase `%xx`.
    fn location_of(path: &Path) -> String {
        let path = path.to_string_lossy().replace('\\', "/");
        let mut out = String::from("file://localhost/");
        for byte in path.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~/:(),+#!".contains(&byte) {
                out.push(char::from(byte));
            } else {
                write!(out, "%{byte:02x}").unwrap();
            }
        }
        out
    }

    fn escaped(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// rekordbox's export of the sample: its tracks with analysis, play
    /// counts and a rating, and a few playlists, some in folders.
    fn export(music: &Path, plan: &[Sample]) -> String {
        struct Entry {
            id: u64,
            name: String,
            artist: String,
            genre: String,
            album: String,
            path: PathBuf,
            bpm: String,
            key: String,
            seconds: u32,
        }
        let mut rng = Lcg(77);
        let mut entries: Vec<Entry> = Vec::new();
        for sample in plan {
            let Some((bpm, key)) = &sample.rekordbox else {
                continue;
            };
            entries.push(Entry {
                id: 9001 + entries.len() as u64,
                name: sample.title.clone().unwrap_or_default(),
                artist: sample.artist.clone().unwrap_or_default(),
                genre: sample.genre.clone().unwrap_or_default(),
                album: sample.album.clone().unwrap_or_default(),
                path: music.join(sample.rel.replace('/', "\\")),
                bpm: bpm.clone(),
                key: key.clone(),
                seconds: 3 + sample.audio % 3,
            });
        }
        for (rel, artist, genre) in NEVER_HERE {
            entries.push(Entry {
                id: 9001 + entries.len() as u64,
                name: Path::new(rel)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                artist: artist.into(),
                genre: genre.into(),
                album: String::new(),
                path: music
                    .parent()
                    .unwrap()
                    .join("old-drive")
                    .join(rel.replace('/', "\\")),
                bpm: format!("{}.00", 100 + rng.below(60)),
                key: rng.pick(&KEYS).to_owned(),
                seconds: 240,
            });
        }

        let mut xml = String::new();
        xml.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n<DJ_PLAYLISTS Version=\"1.0.0\">\n",
        );
        xml.push_str("  <PRODUCT Name=\"rekordbox\" Version=\"7.2.19\" Company=\"AlphaTheta\"/>\n");
        writeln!(xml, "  <COLLECTION Entries=\"{}\">", entries.len()).unwrap();
        for e in &entries {
            let kind = if e.path.extension().is_some_and(|x| x == "wav") {
                "WAV File"
            } else {
                "MP3 File"
            };
            let attrs = [
                ("Name", e.name.clone()),
                ("Artist", e.artist.clone()),
                ("Composer", String::new()),
                ("Album", e.album.clone()),
                ("Grouping", String::new()),
                ("Genre", e.genre.clone()),
                ("Kind", kind.to_owned()),
                ("Size", "48000".to_owned()),
                ("TotalTime", e.seconds.to_string()),
                ("DiscNumber", "0".to_owned()),
                ("TrackNumber", "0".to_owned()),
                ("Year", "0".to_owned()),
                ("AverageBpm", e.bpm.clone()),
                (
                    "DateAdded",
                    format!("2026-0{}-{:02}", 1 + e.id % 8, 1 + e.id % 27),
                ),
                ("BitRate", "128".to_owned()),
                ("SampleRate", "8000".to_owned()),
                ("Comments", String::new()),
                ("PlayCount", (e.id * 7 % 23).to_string()),
                (
                    "Rating",
                    ["0", "51", "102", "153", "204", "255"][(e.id % 6) as usize].to_owned(),
                ),
                ("Location", location_of(&e.path)),
                ("Remixer", String::new()),
                ("Tonality", e.key.clone()),
                ("Label", String::new()),
                ("Mix", String::new()),
            ];
            write!(xml, "    <TRACK TrackID=\"{}\"", e.id).unwrap();
            for (name, value) in attrs {
                write!(xml, " {name}=\"{}\"", escaped(&value)).unwrap();
            }
            xml.push_str(">\n");
            writeln!(
                xml,
                "      <TEMPO Inizio=\"0.025\" Bpm=\"{}\" Metro=\"4/4\" Battito=\"1\"/>",
                e.bpm
            )
            .unwrap();
            xml.push_str("    </TRACK>\n");
        }
        xml.push_str("  </COLLECTION>\n  <PLAYLISTS>\n");
        // Three playlists in a folder, one outside it.
        let ids: Vec<u64> = entries.iter().map(|e| e.id).collect();
        let slice = |from: usize, len: usize| -> Vec<u64> {
            ids.iter().copied().skip(from).take(len).collect()
        };
        let playlist = |name: &str, tracks: Vec<u64>, pad: &str| -> String {
            let mut out = format!(
                "{pad}<NODE Name=\"{}\" Type=\"1\" KeyType=\"0\" Entries=\"{}\">\n",
                escaped(name),
                tracks.len()
            );
            for key in tracks {
                writeln!(out, "{pad}  <TRACK Key=\"{key}\"/>").unwrap();
            }
            writeln!(out, "{pad}</NODE>").unwrap();
            out
        };
        writeln!(xml, "    <NODE Type=\"0\" Name=\"ROOT\" Count=\"2\">").unwrap();
        xml.push_str("      <NODE Name=\"Sets\" Type=\"0\" Count=\"3\">\n");
        xml.push_str(&playlist("Opening hour", slice(0, 14), "        "));
        xml.push_str(&playlist("Peak", slice(14, 22), "        "));
        xml.push_str(&playlist(
            "A playlist with a name long enough to need cutting off somewhere",
            slice(40, 9),
            "        ",
        ));
        xml.push_str("      </NODE>\n");
        xml.push_str(&playlist("Everything I own", ids.clone(), "      "));
        xml.push_str("    </NODE>\n  </PLAYLISTS>\n</DJ_PLAYLISTS>\n");
        xml
    }

    /// The running app and what's needed to drive it: its commands over
    /// IPC, and a way to wait for its jobs.
    struct Driver {
        app: tauri::App<MockRuntime>,
        webview: tauri::WebviewWindow<MockRuntime>,
        updates: mpsc::Receiver<()>,
    }

    const PATIENCE: Duration = Duration::from_secs(10 * 60);

    #[allow(deprecated)]
    fn start(data: PathBuf) -> Driver {
        let mut app = tracklist_pro_lib::setup_for_tests(mock_builder(), data)
            .build(tauri::generate_context!(test = true))
            .unwrap();
        // The startup hook runs when the event loop starts; on the mock
        // runtime one iteration runs it and returns.
        app.run_iteration(|_, _| {});
        let webview = app.get_webview_window("main").unwrap();
        let (heard, updates) = mpsc::channel();
        let heard = Mutex::new(heard);
        jobs::JobUpdates::listen_any(app.handle(), move |_| {
            let _ = heard.lock().unwrap().send(());
        });
        Driver {
            app,
            webview,
            updates,
        }
    }

    impl Driver {
        fn try_call<T: DeserializeOwned>(&self, cmd: &str, args: Value) -> Result<T, Value> {
            let request = tauri::webview::InvokeRequest {
                cmd: cmd.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            };
            tauri::test::get_ipc_response(&self.webview, request).map(|body| {
                body.deserialize()
                    .unwrap_or_else(|e| panic!("{cmd} answered something unexpected: {e}"))
            })
        }

        fn call<T: DeserializeOwned>(&self, cmd: &str, args: Value) -> T {
            self.try_call(cmd, args)
                .unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
        }

        /// Waits until no job is queued or running.
        fn settle(&self) {
            loop {
                let activity: ActivitySnapshot = self.call("activity", json!({}));
                if activity.jobs.is_empty() {
                    return;
                }
                self.updates
                    .recv_timeout(PATIENCE)
                    .expect("a job never finished");
            }
        }

        /// Calls a command that queues a job, waits for it and what it
        /// queues, and fails unless it ended done.
        fn job_done(&self, cmd: &str, args: Value) -> Result<(), String> {
            let id: JobId = self.call(cmd, args);
            self.settle();
            let job = self
                .app
                .state::<db::Writer>()
                .call(move |c| jobs::store::get(c, id))
                .map_err(|e| format!("{cmd}: {e:?}"))?
                .ok_or_else(|| format!("{cmd}: no job row"))?;
            if job.status == JobStatus::Done {
                Ok(())
            } else {
                Err(format!("{cmd} ended {:?}: {:?}", job.status, job.error))
            }
        }

        /// What closing the app does: no watcher or job outlives the run.
        fn stop(self) {
            if let Some(watchers) = self.app.try_state::<scan::Watchers>() {
                watchers.shutdown();
            }
            if let Some(queue) = self.app.try_state::<jobs::JobQueue>() {
                queue.shutdown();
            }
        }
    }

    /// What the seed made, for the line it prints.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub struct Summary {
        pub files: usize,
        pub library: usize,
        pub missing: usize,
        pub crates: usize,
        pub rekordbox_entries: usize,
    }

    impl std::fmt::Display for Summary {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "sample library made: {} files, {} in the Library ({} of them missing), {} crates, {} rekordbox entries",
                self.files, self.library, self.missing, self.crates, self.rekordbox_entries
            )
        }
    }

    /// Makes the sample under `preview`: files, export, then the app's own
    /// state. Refuses a data folder that isn't empty.
    pub fn seed(preview: &Preview) -> Result<Summary, String> {
        let data = preview.data();
        let has_data = fs::read_dir(&data).is_ok_and(|mut d| d.next().is_some());
        if has_data || preview.is_seeded() {
            return Err(format!(
                "{} already holds a library; `npm run design:reset` clears it",
                data.display()
            ));
        }
        let music = preview.music();
        if fs::read_dir(&music).is_ok_and(|mut d| d.next().is_some()) {
            return Err(format!("{} isn't empty", music.display()));
        }
        let documents = preview.documents();
        fs::create_dir_all(&music).map_err(|e| e.to_string())?;
        fs::create_dir_all(&documents).map_err(|e| e.to_string())?;

        let plan = plan();
        for sample in &plan {
            let path = music.join(sample.rel.replace('/', "\\"));
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::write(&path, wav(sample)).map_err(|e| e.to_string())?;
        }
        let export_path = documents.join("rekordbox.xml");
        let xml = export(&music, &plan);
        fs::write(&export_path, &xml).map_err(|e| e.to_string())?;

        let app = start(data);
        let result = drive(&app, &music, &export_path, &plan);
        app.stop();
        let mut summary = result?;
        summary.files = plan.len();
        summary.rekordbox_entries = xml.matches("<TRACK TrackID=").count();
        fs::write(preview.root().join(SEEDED), format!("{summary}\n"))
            .map_err(|e| e.to_string())?;
        Ok(summary)
    }

    /// The user's steps, as commands: add the folder, read rekordbox, add
    /// to the Library, make crates, lose some files, send.
    fn drive(
        app: &Driver,
        music: &Path,
        export: &Path,
        plan: &[Sample],
    ) -> Result<Summary, String> {
        let _: Value = app.call(
            "add_music_folder",
            json!({ "path": music.to_string_lossy(), "role": null }),
        );
        app.settle();
        app.job_done(
            "read_rekordbox_xml",
            json!({ "path": export.to_string_lossy() }),
        )?;

        // All music, by file name.
        let rows: Value = app.call("all_music_tracks", json!({ "search": null }));
        let rows = rows["tracks"].as_array().cloned().unwrap_or_default();
        let recording_of = |sample: &Sample| -> Option<i64> {
            rows.iter()
                .find(|row| row["file"]["name"] == sample.file_name())
                .and_then(|row| row["recordingId"].as_i64())
        };

        // Add to the Library, in plan order; a track whose match isn't
        // confirmed yet can't be added, and stays in All music.
        let mut library: Vec<(usize, i64)> = Vec::new();
        for (n, sample) in plan.iter().enumerate() {
            if !sample.in_library {
                continue;
            }
            let Some(recording) = recording_of(sample) else {
                continue;
            };
            if let Ok(promoted) =
                app.try_call::<Value>("promote_track", json!({ "recordingId": recording }))
            {
                if let Some(id) = promoted["libraryTrack"]["id"].as_i64() {
                    library.push((n, id));
                }
            }
        }

        // Crates: sizes from none to a few dozen, one with a long name.
        let ids = |from: usize, len: usize| -> Vec<i64> {
            library
                .iter()
                .skip(from)
                .take(len)
                .map(|(_, id)| *id)
                .collect()
        };
        let crates: [(&str, Vec<i64>); 5] = [
            ("Warm-up", ids(0, 12)),
            ("Peak hour", ids(12, 26)),
            ("Late and deep", ids(38, 9)),
            (
                "Sunday records, the long name that has to be cut off somewhere",
                ids(50, 5),
            ),
            ("To sort", Vec::new()),
        ];
        for (name, tracks) in &crates {
            let id: i64 = app.call("create_crate", json!({ "name": name }));
            if !tracks.is_empty() {
                let _: Value =
                    app.call("add_tracks_to_crate", json!({ "id": id, "tracks": tracks }));
            }
        }

        // Some files vanish, and the next scan notices.
        let mut missing = 0;
        for sample in plan.iter().filter(|s| s.gone) {
            let path = music.join(sample.rel.replace('/', "\\"));
            fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            missing += 1;
        }
        app.job_done("scan_music_folders", json!({ "ids": null }))?;

        // The send is prepared and written, so the checklist and the
        // lists after it have something to show.
        app.job_done("prepare_send", json!({ "path": export.to_string_lossy() }))?;
        let state: Value = app.call("send_state", json!({}));
        if let Some(token) = state["preflight"]["token"].as_str() {
            let confirmed = state["preflight"]["needsConfirm"]
                .as_bool()
                .unwrap_or(false);
            if state["preflight"]["canSend"].as_bool().unwrap_or(false) {
                app.job_done(
                    "write_send",
                    json!({ "token": token, "confirmed": confirmed }),
                )?;
            }
        }

        Ok(Summary {
            library: library.len(),
            missing,
            crates: crates.len(),
            ..Summary::default()
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use tracklist_pro_lib::preview::MARKER;

        fn marked() -> (tempfile::TempDir, Preview) {
            let dir = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(dir.path()).unwrap();
            let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
            let root = root.join("preview");
            fs::create_dir(&root).unwrap();
            fs::write(root.join(MARKER), "").unwrap();
            let preview = Preview::at(&root).unwrap();
            (dir, preview)
        }

        #[test]
        fn the_plan_has_every_kind_of_row_a_screen_needs() {
            let plan = plan();
            assert!(plan.len() > 200, "{}", plan.len());
            // Paths are unique, so every file is a file.
            let paths: BTreeSet<_> = plan.iter().map(|s| s.rel.to_lowercase()).collect();
            assert_eq!(paths.len(), plan.len());
            // The seed finds rows by file name, so one name is one audio:
            // a copy in another folder keeps the name, never a different
            // track's.
            let mut audio_of: BTreeMap<String, u32> = BTreeMap::new();
            for s in &plan {
                let name = s.file_name().to_lowercase();
                assert_eq!(*audio_of.entry(name).or_insert(s.audio), s.audio);
            }

            let longest = plan
                .iter()
                .filter_map(|s| s.title.as_ref().map(|t| t.chars().count()))
                .max()
                .unwrap();
            assert!(longest > 120, "longest title {longest}");
            assert!(plan.iter().any(|s| s.title.is_none()), "untagged files");
            assert!(plan.iter().any(|s| s.gone), "missing files");
            assert!(plan.iter().any(|s| s.in_library));
            assert!(
                plan.iter().any(|s| !s.in_library),
                "tracks only in All music"
            );
            assert!(plan.iter().any(|s| s.rekordbox.is_none()));
            assert!(plan.iter().any(|s| s.rekordbox.is_some()));
            // Duplicates: the same audio under two names.
            let mut audio = BTreeSet::new();
            assert!(plan.iter().any(|s| !audio.insert(s.audio)), "duplicates");
            // Only files that exist can go missing, and only once the
            // Library has them.
            assert!(plan.iter().filter(|s| s.gone).all(|s| s.in_library));
        }

        #[test]
        fn the_plan_is_the_same_every_time() {
            let a = plan();
            let b = plan();
            let key = |p: &[Sample]| {
                p.iter()
                    .map(|s| format!("{}|{:?}|{}", s.rel, s.title, s.audio))
                    .collect::<Vec<_>>()
            };
            assert_eq!(key(&a), key(&b));
        }

        #[test]
        fn ordinary_artists_are_built_from_the_syllable_lists() {
            // Every ordinary name is made from the syllable lists above
            // (the hand-written rows say what they are), so none is a real
            // artist's.
            let plan = plan();
            for sample in &plan[..super::ORDINARY] {
                let artist = sample.artist.as_ref().unwrap();
                assert!(
                    FIRST.iter().any(|f| artist.starts_with(f)),
                    "{artist} isn't a made-up name"
                );
            }
        }

        #[test]
        fn seeding_builds_a_library_with_everything_the_plan_promises() {
            let (_dir, preview) = marked();
            let summary = seed(&preview).unwrap();
            let plan = plan();
            assert_eq!(summary.files, plan.len());
            assert_eq!(summary.crates, 5);
            assert!(summary.missing >= 10, "{summary:?}");
            assert!(summary.library > 100, "{summary:?}");
            assert!(preview.is_seeded());

            // The sample's music folder is what the plan says, minus the
            // files that went missing.
            let on_disk = fs::read_dir(preview.music()).unwrap().count();
            assert!(on_disk > 5);
            for sample in &plan {
                let path = preview.music().join(sample.rel.replace('/', "\\"));
                assert_eq!(path.is_file(), !sample.gone, "{}", sample.rel);
            }

            // The app wrote its database into the preview's own data
            // folder, and a second seed refuses.
            assert!(fs::read_dir(preview.data()).unwrap().count() > 0);
            assert!(seed(&preview).is_err());
        }

        #[test]
        fn the_export_reads_back_with_the_entries_the_seed_counted() {
            let (_dir, preview) = marked();
            let plan = plan();
            let xml = export(&preview.music(), &plan);
            let file = preview.root().join("export-test.xml");
            fs::write(&file, xml).unwrap();
            let parsed = tracklist_pro_lib::rekordbox::RekordboxXml::read_file(&file)
                .expect("the sample export is a rekordbox export");
            let known = plan.iter().filter(|s| s.rekordbox.is_some()).count();
            assert_eq!(parsed.tracks.len(), known + NEVER_HERE.len());
        }
    }
}
