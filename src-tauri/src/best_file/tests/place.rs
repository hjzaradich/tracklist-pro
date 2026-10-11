//! The place sub-steps, and "on disk" against "volume online".

use super::*;

fn placed(id: i64, change: impl FnOnce(&mut Place)) -> FileFacts {
    with(file(id), |f| change(&mut f.place))
}

#[test]
fn a_connected_volume_is_ahead_of_an_unplugged_one_whatever_the_rest_of_the_place() {
    let online = placed(2, |p| {
        p.volume_online = true;
        p.folder_role = FolderRole::Inbox;
        p.volume_kind = VolumeKind::Network;
        p.folder_order = 9;
    });
    let unplugged = placed(1, |p| p.volume_online = false);
    let why = why_ahead(&online, &unplugged).unwrap();
    assert_eq!(why.step(), Some(Step::Place));
}

#[test]
fn a_scan_folder_is_ahead_of_an_inbox_whatever_the_volume_kind_or_folder_order() {
    let scan = placed(2, |p| {
        p.volume_kind = VolumeKind::Network;
        p.folder_order = 9;
    });
    let inbox = placed(1, |p| p.folder_role = FolderRole::Inbox);
    assert_eq!(why_ahead(&scan, &inbox).unwrap().step(), Some(Step::Place));
}

#[test]
fn internal_is_ahead_of_external_and_external_of_network_whatever_the_folder_order() {
    let internal = placed(3, |p| {
        p.volume_kind = VolumeKind::Internal;
        p.folder_order = 9;
    });
    let external = placed(2, |p| {
        p.volume_kind = VolumeKind::External;
        p.folder_order = 5;
    });
    let network = placed(1, |p| p.volume_kind = VolumeKind::Network);
    assert!(why_ahead(&internal, &external).is_some());
    assert!(why_ahead(&external, &network).is_some());
    assert!(why_ahead(&internal, &network).is_some());
    assert_eq!(why_ahead(&network, &external), None);
}

#[test]
fn the_folder_added_first_is_ahead_when_the_rest_of_the_place_is_the_same() {
    let first = placed(2, |p| p.folder_order = 3);
    let later = placed(1, |p| p.folder_order = 8);
    assert_eq!(why_ahead(&first, &later).unwrap().step(), Some(Step::Place));
    assert_eq!(why_ahead(&later, &first), None);
}

#[test]
fn the_place_reason_carries_both_places() {
    let first = placed(2, |p| p.folder_order = 3);
    let later = placed(1, |p| p.folder_order = 8);
    assert_eq!(
        why_ahead(&first, &later),
        Some(Reason::BetterPlace {
            best: first.place,
            other: later.place
        })
    );
}

// ---- On disk is not volume online -----------------------------------

#[test]
fn a_file_on_an_unplugged_drive_is_on_disk_and_ahead_of_one_that_is_gone() {
    // Unplugged isn't missing (ROADMAP 1.1): the file counts as on disk.
    let unplugged = with(file(2), |f| {
        f.on_disk = true;
        f.place.volume_online = false;
    });
    let gone = with(file(1), |f| {
        f.on_disk = false;
        f.place.volume_online = true;
    });
    assert_eq!(why_ahead(&unplugged, &gone), Some(Reason::NotOnDisk));
}

#[test]
fn a_file_on_an_unplugged_drive_is_behind_a_connected_one_by_place_not_by_disk() {
    let unplugged = with(file(1), |f| f.place.volume_online = false);
    let connected = with(file(2), |_| {});
    let why = why_ahead(&connected, &unplugged).unwrap();
    assert_eq!(why.step(), Some(Step::Place));
    assert_ne!(why, Reason::NotOnDisk);
}

#[test]
fn a_connected_volume_does_not_make_a_file_that_is_gone_better() {
    // The volume is online, the file isn't there: the first step reads
    // the file, not the volume.
    let gone_on_online_volume = with(file(1), |f| f.on_disk = false);
    let on_unplugged_drive = with(file(2), |f| f.place.volume_online = false);
    assert_eq!(
        why_ahead(&on_unplugged_drive, &gone_on_online_volume),
        Some(Reason::NotOnDisk)
    );
}
