#![cfg(test)]
//! Migrations 0006 (`file.file_id`) and 0008 (`file.online_only`, the
//! music folder's unreadable counts).

use super::support::db;

fn insert(file_id: Option<&str>) -> Result<usize, String> {
    let (_dir, writer, _reads) = db();
    let file_id = file_id.map(str::to_owned);
    writer
        .call(move |c| {
            c.execute_batch(
                "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
                 INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');",
            )?;
            c.execute(
                "INSERT INTO file (music_folder_id, rel_path, rel_path_key, file_id)
                 VALUES (1, 'a.mp3', 'a.mp3', ?1)",
                [file_id],
            )
        })
        .map_err(|e| e.to_string())
}

#[test]
fn file_id_holds_the_serial_and_128_bit_id_in_lowercase_hex_or_nothing() {
    assert_eq!(insert(None), Ok(1));
    assert_eq!(
        insert(Some("00000000a1b2c3d4-0000000000000000000a000000000f3c")),
        Ok(1)
    );
    for bad in [
        "",
        "1234",
        "00000000A1B2C3D4-0000000000000000000a000000000f3c",
        "00000000a1b2c3d4_0000000000000000000a000000000f3c",
        "00000000a1b2c3d4-0000000000000000000a000000000f3",
        "00000000a1b2c3d4-0000000000000000000a000000000f3cc",
        "00000000a1b2c3d-40000000000000000000a000000000f3c",
        "00000000a1b2c3d4-000000000000000000-a000000000f3c",
        "00000000a1b2c3g4-0000000000000000000a000000000f3c",
    ] {
        let refused = insert(Some(bad));
        assert!(
            matches!(&refused, Err(e) if e.contains("CHECK")),
            "{bad:?}: {refused:?}"
        );
    }
}

#[test]
fn a_file_is_local_unless_marked_online_only_and_the_mark_is_0_or_1() {
    let (_dir, writer, _reads) = db();
    let result = writer.call(|c| {
        c.execute_batch(
            "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
             INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');
             INSERT INTO file (music_folder_id, rel_path, rel_path_key) VALUES (1, 'a.mp3', 'a.mp3');
             INSERT INTO file (music_folder_id, rel_path, rel_path_key, online_only)
                 VALUES (1, 'b.mp3', 'b.mp3', 1);",
        )?;
        let marks: Vec<i64> = c
            .prepare("SELECT online_only FROM file ORDER BY id")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let refused = c
            .execute("UPDATE file SET online_only = 2 WHERE id = 1", [])
            .unwrap_err()
            .to_string();
        Ok((marks, refused))
    });
    let (marks, refused) = result.unwrap();
    assert_eq!(marks, [0, 1]);
    assert!(refused.contains("CHECK"), "{refused}");
}

#[test]
fn a_music_folder_has_no_unreadable_counts_until_walked_and_never_negative_ones() {
    let (_dir, writer, _reads) = db();
    let result = writer.call(|c| {
        c.execute_batch(
            "INSERT INTO volume (identity, kind) VALUES ('serial=NTFS-1A2B3C4D', 'external');
             INSERT INTO music_folder (volume_id, rel_path, rel_path_key) VALUES (1, 'M', 'M');",
        )?;
        let fresh: (Option<String>, Option<i64>, Option<i64>) = c.query_row(
            "SELECT walked_at, unreadable_folders, unreadable_files FROM music_folder",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let refused: Vec<String> = ["unreadable_folders", "unreadable_files"]
            .iter()
            .map(|col| {
                c.execute(&format!("UPDATE music_folder SET {col} = -1"), [])
                    .unwrap_err()
                    .to_string()
            })
            .collect();
        Ok((fresh, refused))
    });
    let (fresh, refused) = result.unwrap();
    assert_eq!(fresh, (None, None, None));
    for e in refused {
        assert!(e.contains("CHECK"), "{e}");
    }
}
