//! Decrypts the staged copy of rekordbox's library database into a plain
//! SQLite file. The database is SQLCipher 4 with its default settings:
//! 4096-byte pages, each encrypted with AES-256-CBC under its own IV and
//! signed with HMAC-SHA512 over the page and its number; the file begins
//! with the 16-byte salt the keys are derived from. A `-wal` file holds
//! newer pages in the same form, and its committed frames are folded in,
//! so the plain file is the database as rekordbox last committed it.
//!
//! The plain file keeps the page size and the reserved bytes at each
//! page's end (zeroed), and is marked as a rollback-journal database, so
//! it opens read-only with no `-wal` or `-shm` beside it.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::Sha512;

use super::DbKey;

pub(super) const PAGE: usize = 4096;
const SALT: usize = 16;
const IV: usize = 16;
const MAC_LEN: usize = 64;
/// The IV and the HMAC, at the end of every page.
const RESERVE: usize = IV + MAC_LEN;
const KDF_ROUNDS: u32 = 256_000;
const MAC_KDF_ROUNDS: u32 = 2;
const MAC_SALT_MASK: u8 = 0x3a;
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";
const WAL_HEADER: usize = 32;
const FRAME_HEADER: usize = 24;
const WAL_MAGIC: u32 = 0x377f_0682;

type Decryptor = cbc::Decryptor<aes::Aes256>;

#[derive(Debug)]
pub(super) enum DecryptError {
    /// The first page doesn't verify: the key isn't this database's.
    WrongKey,
    /// A later page doesn't verify, or the file isn't whole.
    Damaged(&'static str),
    /// A plain SQLite file, not an encrypted one.
    NotEncrypted,
    Io(io::Error),
}

impl From<io::Error> for DecryptError {
    fn from(e: io::Error) -> DecryptError {
        DecryptError::Io(e)
    }
}

/// The page key and the HMAC key, wiped when dropped.
struct Keys {
    cipher: [u8; 32],
    mac: [u8; 32],
}

impl Keys {
    fn derive(key: &DbKey, salt: &[u8]) -> Keys {
        let mut cipher = [0u8; 32];
        match key.raw() {
            Some(raw) => cipher = raw,
            None => pbkdf2::pbkdf2_hmac::<Sha512>(key.passphrase(), salt, KDF_ROUNDS, &mut cipher),
        }
        let mac_salt: Vec<u8> = salt.iter().map(|b| b ^ MAC_SALT_MASK).collect();
        let mut mac = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha512>(&cipher, &mac_salt, MAC_KDF_ROUNDS, &mut mac);
        Keys { cipher, mac }
    }

    fn page_mac(&self, pgno: u32, signed: &[u8]) -> Hmac<Sha512> {
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(&self.mac).expect("any key length");
        mac.update(signed);
        mac.update(&pgno.to_le_bytes());
        mac
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        self.cipher.fill(0);
        self.mac.fill(0);
        std::hint::black_box(&self.cipher);
        std::hint::black_box(&self.mac);
    }
}

/// Where page `pgno`'s encrypted data starts: page 1 begins with the salt.
fn data_start(pgno: u32) -> usize {
    if pgno == 1 {
        SALT
    } else {
        0
    }
}

/// Decrypts one page into `out`. A page of zeros was never written and
/// stays zeros.
fn decrypt_page(keys: &Keys, pgno: u32, page: &[u8], out: &mut [u8]) -> Result<(), ()> {
    out.fill(0);
    if page.iter().all(|&b| b == 0) {
        return Ok(());
    }
    let (start, end) = (data_start(pgno), PAGE - RESERVE);
    keys.page_mac(pgno, &page[start..end + IV])
        .verify_slice(&page[end + IV..])
        .map_err(|_| ())?;
    out[start..end].copy_from_slice(&page[start..end]);
    Decryptor::new_from_slices(&keys.cipher, &page[end..end + IV])
        .map_err(|_| ())?
        .decrypt_padded_mut::<NoPadding>(&mut out[start..end])
        .map_err(|_| ())?;
    if pgno == 1 {
        out[..SALT].copy_from_slice(SQLITE_HEADER);
        // Rollback journal, not WAL: no -wal or -shm is needed to open it.
        out[18] = 1;
        out[19] = 1;
    }
    Ok(())
}

/// The committed pages of a `-wal` file: where each page's newest
/// committed copy is, and the database's size in pages after the last
/// commit.
#[derive(Default)]
struct WalPages {
    at: HashMap<u32, u64>,
    size: Option<u32>,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// SQLite's WAL checksum over `data`, continuing from `s`.
fn wal_sum(big_endian: bool, data: &[u8], mut s: (u32, u32)) -> (u32, u32) {
    let word = |b: &[u8]| {
        let b = [b[0], b[1], b[2], b[3]];
        if big_endian {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    };
    for pair in data.chunks_exact(8) {
        s.0 = s.0.wrapping_add(word(&pair[..4])).wrapping_add(s.1);
        s.1 = s.1.wrapping_add(word(&pair[4..])).wrapping_add(s.0);
    }
    s
}

/// Reads a `-wal` file as SQLite recovers one: frames count only while
/// their salts and running checksum hold, and only up to the last commit.
/// A file with no valid header holds nothing, as for SQLite.
fn read_wal(wal: &mut File) -> io::Result<WalPages> {
    let mut found = WalPages::default();
    let mut reader = BufReader::with_capacity(1 << 16, wal);
    let mut header = [0u8; WAL_HEADER];
    if reader.read_exact(&mut header).is_err() {
        return Ok(found);
    }
    let magic = be32(&header);
    let big_endian = magic & 1 == 1;
    let mut sum = wal_sum(big_endian, &header[..24], (0, 0));
    if magic & !1 != WAL_MAGIC
        || be32(&header[8..]) as usize != PAGE
        || sum != (be32(&header[24..]), be32(&header[28..]))
    {
        return Ok(found);
    }
    let mut pending = Vec::new();
    let mut frame = vec![0u8; FRAME_HEADER + PAGE];
    let mut offset = WAL_HEADER as u64;
    while reader.read_exact(&mut frame).is_ok() {
        let (head, page) = frame.split_at(FRAME_HEADER);
        sum = wal_sum(big_endian, page, wal_sum(big_endian, &head[..8], sum));
        let pgno = be32(head);
        if head[8..16] != header[16..24]
            || sum != (be32(&head[16..]), be32(&head[20..]))
            || pgno == 0
        {
            break;
        }
        pending.push((pgno, offset + FRAME_HEADER as u64));
        let commit = be32(&head[4..]);
        if commit != 0 {
            found.at.extend(pending.drain(..));
            found.size = Some(commit);
        }
        offset += frame.len() as u64;
    }
    Ok(found)
}

/// Decrypts the database `db`, with its `-wal` file if there is one, into
/// `out`. Returns how many pages were written.
pub(super) fn decrypt(
    key: &DbKey,
    db: &mut File,
    mut wal: Option<&mut File>,
    out: &mut impl Write,
) -> Result<u32, DecryptError> {
    let db_len = db.metadata()?.len();
    if db_len % PAGE as u64 != 0 {
        return Err(DecryptError::Damaged("the file isn't whole pages"));
    }
    let db_pages =
        u32::try_from(db_len / PAGE as u64).map_err(|_| DecryptError::Damaged("too many pages"))?;
    let wal_pages = match wal.as_deref_mut() {
        Some(file) => read_wal(file)?,
        None => WalPages::default(),
    };
    let count = wal_pages.size.unwrap_or(db_pages);
    if count == 0 {
        return Err(DecryptError::Damaged("the file is empty"));
    }
    let mut db = BufReader::with_capacity(1 << 16, db);
    let mut page = vec![0u8; PAGE];
    let mut plain = vec![0u8; PAGE];
    let mut keys = None;
    for pgno in 1..=count {
        page.fill(0);
        if pgno <= db_pages {
            db.read_exact(&mut page)?;
        }
        if let (Some(&at), Some(file)) = (wal_pages.at.get(&pgno), wal.as_deref_mut()) {
            file.seek(SeekFrom::Start(at))?;
            file.read_exact(&mut page)?;
        }
        if pgno == 1 {
            if page.starts_with(SQLITE_HEADER) {
                return Err(DecryptError::NotEncrypted);
            }
            if page.iter().all(|&b| b == 0) {
                return Err(DecryptError::Damaged("the first page is empty"));
            }
        }
        let keys = keys.get_or_insert_with(|| Keys::derive(key, &page[..SALT]));
        if decrypt_page(keys, pgno, &page, &mut plain).is_err() {
            return Err(if pgno == 1 {
                DecryptError::WrongKey
            } else {
                DecryptError::Damaged("a page doesn't verify")
            });
        }
        if pgno == 1
            && (u16::from_be_bytes([plain[16], plain[17]]) as usize != PAGE
                || plain[20] as usize != RESERVE)
        {
            return Err(DecryptError::Damaged("the header isn't SQLCipher's"));
        }
        out.write_all(&plain)?;
    }
    out.flush()?;
    plain.fill(0);
    Ok(count)
}

/// The inverse, for tests only: encrypts a plain SQLite file (with
/// [`RESERVE`] reserved bytes per page) and builds `-wal` files, so tests
/// make synthetic databases and never need a real one or a real key.
#[cfg(test)]
pub(super) mod inverse {
    use aes::cipher::{block_padding::NoPadding, BlockEncryptMut, KeyIvInit};
    use hmac::Mac;

    use super::*;

    type Encryptor = cbc::Encryptor<aes::Aes256>;

    /// Page `pgno` of `plain`, encrypted under `salt`'s keys.
    pub fn encrypt_page(key: &DbKey, salt: &[u8; 16], pgno: u32, plain: &[u8]) -> Vec<u8> {
        let keys = Keys::derive(key, salt);
        let (start, end) = (data_start(pgno), PAGE - RESERVE);
        let mut page = vec![0u8; PAGE];
        page[start..end].copy_from_slice(&plain[start..end]);
        let iv: Vec<u8> = (0..IV)
            .map(|i| (pgno as u8).wrapping_mul(31) ^ i as u8)
            .collect();
        Encryptor::new_from_slices(&keys.cipher, &iv)
            .unwrap()
            .encrypt_padded_mut::<NoPadding>(&mut page[start..end], end - start)
            .unwrap();
        page[end..end + IV].copy_from_slice(&iv);
        let mac = keys
            .page_mac(pgno, &page[start..end + IV])
            .finalize()
            .into_bytes();
        page[end + IV..].copy_from_slice(&mac);
        if pgno == 1 {
            page[..SALT].copy_from_slice(salt);
        }
        page
    }

    /// A whole plain file, encrypted.
    pub fn encrypt(key: &DbKey, salt: &[u8; 16], plain: &[u8]) -> Vec<u8> {
        assert_eq!(plain.len() % PAGE, 0);
        plain
            .chunks(PAGE)
            .enumerate()
            .flat_map(|(i, page)| encrypt_page(key, salt, i as u32 + 1, page))
            .collect()
    }

    /// A `-wal` file holding `frames` (page number, encrypted page, the
    /// database size after it for a commit or 0), in SQLite's format.
    pub fn wal(frames: &[(u32, Vec<u8>, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(WAL_MAGIC.to_be_bytes());
        out.extend(3_007_000u32.to_be_bytes());
        out.extend((PAGE as u32).to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out.extend([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let mut sum = wal_sum(false, &out, (0, 0));
        out.extend(sum.0.to_be_bytes());
        out.extend(sum.1.to_be_bytes());
        for (pgno, page, commit) in frames {
            let mut head = Vec::new();
            head.extend(pgno.to_be_bytes());
            head.extend(commit.to_be_bytes());
            head.extend_from_slice(&out[16..24]);
            sum = wal_sum(false, page, wal_sum(false, &head[..8], sum));
            head.extend(sum.0.to_be_bytes());
            head.extend(sum.1.to_be_bytes());
            out.extend(head);
            out.extend_from_slice(page);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::inverse::*;
    use super::*;

    const SALT_BYTES: [u8; 16] = *b"synthetic salt!!";

    /// A made-up raw key: 64 hex digits from a counter, never a real one.
    fn raw_key(seed: u8) -> DbKey {
        let hex: String = (0..32u8)
            .map(|i| format!("{:02x}", i.wrapping_mul(7) ^ seed))
            .collect();
        DbKey::from_text(format!("x'{hex}'")).unwrap()
    }

    /// `n` pages of patterned data, the first with a header that says
    /// 4096-byte pages and 80 reserved bytes.
    fn plain_pages(n: usize) -> Vec<u8> {
        let mut plain: Vec<u8> = (0..n * PAGE).map(|i| (i * 7 % 253) as u8).collect();
        for page in plain.chunks_mut(PAGE) {
            page[PAGE - RESERVE..].fill(0);
        }
        plain[..16].copy_from_slice(SQLITE_HEADER);
        plain[16..18].copy_from_slice(&(PAGE as u16).to_be_bytes());
        plain[18] = 1;
        plain[19] = 1;
        plain[20] = RESERVE as u8;
        plain
    }

    fn file(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> File {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        File::open(path).unwrap()
    }

    fn run(key: &DbKey, db: &[u8], wal: Option<&[u8]>) -> Result<Vec<u8>, DecryptError> {
        let dir = tempfile::tempdir().unwrap();
        let mut db = file(&dir, "db", db);
        let mut wal = wal.map(|w| file(&dir, "wal", w));
        let mut out = Vec::new();
        decrypt(key, &mut db, wal.as_mut(), &mut out)?;
        Ok(out)
    }

    #[test]
    fn what_the_inverse_encrypts_decrypts_back_page_for_page() {
        let key = raw_key(1);
        let plain = plain_pages(3);
        let encrypted = encrypt(&key, &SALT_BYTES, &plain);
        assert_ne!(encrypted[SALT..PAGE - RESERVE], plain[SALT..PAGE - RESERVE]);
        assert_eq!(run(&key, &encrypted, None).unwrap(), plain);
    }

    #[test]
    fn a_typed_passphrase_derives_the_keys_as_sqlcipher_does() {
        let key = DbKey::from_text("made-up passphrase".into()).unwrap();
        let plain = plain_pages(2);
        let encrypted = encrypt(&key, &SALT_BYTES, &plain);
        assert_eq!(run(&key, &encrypted, None).unwrap(), plain);
    }

    #[test]
    fn a_wrong_key_a_changed_page_a_cut_file_and_a_plain_file_each_say_so() {
        let key = raw_key(1);
        let encrypted = encrypt(&key, &SALT_BYTES, &plain_pages(3));
        assert!(matches!(
            run(&raw_key(2), &encrypted, None),
            Err(DecryptError::WrongKey)
        ));
        let mut changed = encrypted.clone();
        changed[2 * PAGE + 100] ^= 1;
        assert!(matches!(
            run(&key, &changed, None),
            Err(DecryptError::Damaged(_))
        ));
        assert!(matches!(
            run(&key, &encrypted[..PAGE + 10], None),
            Err(DecryptError::Damaged(_))
        ));
        assert!(matches!(
            run(&key, &[], None),
            Err(DecryptError::Damaged(_))
        ));
        assert!(matches!(
            run(&key, &plain_pages(1), None),
            Err(DecryptError::NotEncrypted)
        ));
    }

    #[test]
    fn a_page_never_written_stays_zeros() {
        let key = raw_key(1);
        let mut plain = plain_pages(3);
        let mut encrypted = encrypt(&key, &SALT_BYTES, &plain);
        encrypted[PAGE..2 * PAGE].fill(0);
        plain[PAGE..2 * PAGE].fill(0);
        assert_eq!(run(&key, &encrypted, None).unwrap(), plain);
    }

    #[test]
    fn committed_wal_frames_are_folded_in_and_the_rest_ignored() {
        let key = raw_key(1);
        let plain = plain_pages(2);
        let encrypted = encrypt(&key, &SALT_BYTES, &plain);
        let newer = |fill: u8| {
            let mut page = vec![fill; PAGE];
            page[PAGE - RESERVE..].fill(0);
            page
        };
        let frame = |pgno: u32, fill: u8, commit: u32| {
            (
                pgno,
                encrypt_page(&key, &SALT_BYTES, pgno, &newer(fill)),
                commit,
            )
        };
        // Page 2 changed, page 3 added, committed; then an uncommitted
        // change to page 2.
        let wal_file = wal(&[frame(2, 0xAA, 0), frame(3, 0xBB, 3), frame(2, 0xCC, 0)]);
        let out = run(&key, &encrypted, Some(wal_file.as_slice())).unwrap();
        assert_eq!(out.len(), 3 * PAGE);
        assert_eq!(out[..PAGE], plain[..PAGE]);
        assert_eq!(out[PAGE..2 * PAGE], newer(0xAA)[..]);
        assert_eq!(out[2 * PAGE..], newer(0xBB)[..]);

        // A frame whose checksum fails ends the log there.
        let mut broken = wal(&[frame(2, 0xAA, 2)]);
        broken[WAL_HEADER + FRAME_HEADER + 5] ^= 1;
        assert_eq!(
            run(&key, &encrypted, Some(broken.as_slice())).unwrap(),
            plain
        );
        // So does a file with no valid header.
        assert_eq!(
            run(&key, &encrypted, Some(&[1u8, 2, 3][..])).unwrap(),
            plain
        );
    }
}
