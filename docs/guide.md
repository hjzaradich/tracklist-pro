# User guide

What tracklist-pro does today, by task. The screen and button names are the ones the app shows. The app is still early; the plan is in [ROADMAP.md](../ROADMAP.md). To install it, see [installing.md](installing.md).

All example names below are made up.

The short version: tracklist-pro keeps your own Library of tracks, and sends it to rekordbox as an XML file. It reads your music folders and never moves, renames, retags or deletes them.

## First run

On first start the app shows **Start your Library**. There are three parts:

- **Music folders**: choose **Add folder** for each folder that holds your music. The app only reads them.
- **Start from rekordbox**: use your rekordbox collection as the starting point (see "Reading from rekordbox").
- **Start fresh**: choose **Go to All music** and add tracks yourself.

You can change the folders later.

The app also asks once, **Check for updates?** If you say yes, it asks GitHub (api.github.com), about once a day, whether a newer version exists. It sends the app's version and nothing else. It only tells you, with the download page address to copy; you install it yourself, as before. If you say no, it never contacts GitHub. Settings → **Online services** has the switch, and **Check now**.

## All music and Library

**All music** lists every file the app found in your music folders. **Library** lists the tracks you chose to keep. The two are different on purpose: your Library is yours, and nothing gets into it by accident.

- To add a track, use **Add to Library**.
- To take a track out, use **Remove** and confirm. The file stays where it is, and you can Undo.
- A track whose file moved shows **File missing**. A track on a drive that isn't plugged in shows **Drive not connected**.

The **Missing** list under Review helps you find moved files.

## Scanning

The app scans your music folders in the background. Progress shows in **Activity**. Scanning only reads.

Files that look like the same track are grouped. Versions (a remix, an edit, a clean and a dirty cut) are linked to each other but never merged. When the app isn't sure, the item goes to **Review**, where you accept or decide.

## Reading from rekordbox

In rekordbox, export your collection (**Export Collection in xml format**). Then in tracklist-pro:

1. Choose **Choose export** and pick that file.
2. The app takes it in (**Read again** later reads it again). It shows when it was last read.

With **Watch for new exports (Documents)** on, the app tells you when a newer export shows up and offers **Read**.

### Optional: rekordbox's library database

Settings has **Read rekordbox's library database**. It is optional and off by default. The app reads the database only, never writes to it. To use it you supply the database key yourself. Nothing else needs it.

## Sending to rekordbox

Choose **Send to rekordbox**. The app walks you through it:

1. **Export collection from rekordbox**: pick a fresh export. The send always starts from a fresh read, so changes you made in rekordbox aren't lost.
2. **Review what will be sent**: new tracks, crates, tracks left out (with the reason) and fields that differ.
3. **Write file**: the app writes the XML file. Use **Copy path** to copy where it is.
4. In rekordbox, use **Import to Collection** on that file.

If you changed the same field in both places, it becomes a conflict. Open **Review** to choose which side to keep. The app never overwrites silently.

After you import, **After import** lists playlists and tracks you may want to delete in rekordbox yourself. The app never deletes things in rekordbox.

## Crates and smart crates

**Crates** are your folders of tracks. They appear in rekordbox as playlists with the same names.

- **Create crate**, **Create folder**, **Rename**, **Move to folder**.

A **smart crate** fills itself from a rule. Choose **Create smart crate**, then build the rule: **Add a condition**, set the field and value, and choose match **all** or **any**. The rule shows how many tracks match before you **Save rule**. Tracks come and go as your Library changes.

## Tags and genres

**Tags** are your own labels, in groups (like rekordbox's My Tags). Open **Tags** in the sidebar to make groups and tags, or copy your My Tags from rekordbox. Filter any track list with **Has a tag**.

**Genres** (Settings, then **Open Genres**) are a tree. Add a genre with **Add genre**, or **Add starter genres**. Under **Aliases**, add other spellings (for example, "Hse") so tracks with different spellings land in one genre. The list **Genres in your tracks** shows each spelling found and where it goes. Pick one and choose **Map to this genre**.

## Playlists, the timeline and "Fill the gap"

**Playlists** are ordered lists, like a set. **Create playlist**, then add tracks. You can **Save as crate** or **Start a playlist from this crate**.

Turn on **Timeline** to see the playlist with each track's start time, length, BPM, key and energy, and a chart of energy against a target shape you can set. Transitions that clash or jump are marked. Choose **Fill the gap** (before the first track, between two tracks, or after the last). The app lists Library tracks that fit, each with a short reason such as "same key (8A)". Nothing is added until you pick a track.

**Sort by fit** orders a track list by how well each track follows the selected one.

## Play history

**History** lists the sets you played. Bring one in with **Import m3u8 file** (a playlist file from your DJ software) or **Import from a gig stick** (the app reads the stick and never writes to it). Each set shows its plays; plays of tracks you don't own are marked **Not in your music**. Set the date with **Save date**.

On a set's page, **Add the tracks you don't have to this wishlist** sends the ones you don't own to a wishlist (if you use wishlists).

## What's next

**What's next** suggests tracks to play after the one you select. Each suggestion shows its reasons, for example "same key (8A), 2 BPM, same genre (Techno)". Set the **Energy direction** to Up, Same or Down. A track with a missing value comes last, and the reason says so. The suggestions are only a nudge. You decide.

## Wishlists

Wishlists are optional and off by default. Turn them on in Settings under **Optional features**. Then **New wishlist**: paste lines like `Artist - Title`, or **Choose CSV file**. Each line is marked **Owned**, **Owned, other cut**, **Rework** or **Missing**, with a reason. Lines the app can't use are listed with why. A wishlist never downloads or buys anything.

## Inbox

The **Inbox** holds music that has shown up in your folders and isn't in your Library yet. Each item gets a label with a reason: **New to you**, **Better file**, **Other cut**, **Rework** or **Already have**.

- **Add to Library** (or **Add all**), **Not now**, **Hide**, **Clear**.

Nothing in the Inbox changes until you choose.

## Estimates: BPM, key and energy

The app can fill gaps when rekordbox hasn't analyzed a track yet.

- **BPM** estimates are shown greyed. When rekordbox's value arrives, it replaces the estimate.
- **Key** estimates are off by default. Turn on **Local key estimates (test)** in Settings to show them (greyed) for tracks with no key from rekordbox or a file tag.
- **Energy** is yours. The app never sets it by itself. It may show a suggestion beside the control (for example "Suggested: 6") with its reasons, and you choose **Use 6** or ignore it. Energy goes to rekordbox as Rating only if you turn on **Send energy to rekordbox as Rating**.

## Diagnostics

Settings has **Diagnostics**. **Open Diagnostics** shows an **Event log** of what the app did: time, what happened, the task and a reason code. It holds counts and codes, not your track names or paths. Read it before you share it, for example when you report a bug.

## Undo

**Undo** (Ctrl Z) is in the top bar. It reverses your last change to the Library: an add, a removal, a crate, a tag, a playlist. The button names what it would undo, such as "Undo (removed from Library)". It can't unwrite a file you already sent to rekordbox.

## Settings

**Settings** is the gear in the top bar. It holds the rekordbox options, the optional features, **Online services** (update checks), Genres, Diagnostics and **About** (the licence and credits, and the licences of the software inside the app).

## Where your data is

See [installing.md](installing.md#4-where-your-data-is-kept). Your music files are never changed.

## Asking for help or suggesting something

Use the [issue forms](https://github.com/hjzaradich/tracklist-pro/issues/new/choose). For a bug, include the app and Windows versions, what you did and what happened.
