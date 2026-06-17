//! Content-addressed store.
//!
//! Every file version is stored once, keyed by its BLAKE3 hash. Identical files
//! across many snapshots therefore cost one copy (cheap version history), and
//! integrity is verifiable by re-hashing. Objects are sharded by hash prefix to
//! keep directories small:
//!
//! ```text
//! store/ab/abcd...   <- object whose hash starts "ab"
//! store/.incoming/   <- temp landing zone for in-flight writes
//! ```

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::crypto::{random_bytes, Dek, NONCE_LEN};
use crate::error::{Error, Result};
use crate::model::Hash;
use crate::util::new_id;

const CHUNK: usize = 1 << 16; // 64 KiB streaming buffer
const LZMA_LEVEL: u32 = 6;

/// Magic header on encrypted objects: `nonce` follows, then ciphertext.
const ENC_MAGIC: &[u8; 4] = b"GSE1";

pub struct Cas {
    root: PathBuf,
    /// When set, objects are stored encrypted at rest. The object key is always
    /// BLAKE3 of the *plaintext* (so dedup still works); the bytes on disk are
    /// `GSE1 || nonce || ciphertext`.
    cipher: Option<XChaCha20Poly1305>,
    /// When true, object contents are LZMA2-compressed (the 7-Zip codec) before
    /// being stored (and, if encryption is on, before being encrypted). This is
    /// a store-wide mode; on read it's reversed using the same flag.
    compress: AtomicBool,
}

impl Cas {
    pub fn open(root: PathBuf) -> Result<Self> {
        Self::open_inner(root, None)
    }

    /// Open the store with encryption enabled, using the given data key.
    pub fn open_encrypted(root: PathBuf, dek: &Dek) -> Result<Self> {
        Self::open_inner(root, Some(dek.cipher()))
    }

    fn open_inner(root: PathBuf, cipher: Option<XChaCha20Poly1305>) -> Result<Self> {
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join(".incoming"))?;
        Ok(Self {
            root,
            cipher,
            compress: AtomicBool::new(false),
        })
    }

    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    pub fn compress_enabled(&self) -> bool {
        self.compress.load(Ordering::Relaxed)
    }

    /// Set the store-wide compression mode (the engine sets this from config at
    /// open, and may toggle it while the store is still empty).
    pub fn set_compress(&self, on: bool) {
        self.compress.store(on, Ordering::Relaxed);
    }

    /// Whether a fast raw-copy path applies (no encryption, no compression).
    fn raw_mode(&self) -> bool {
        self.cipher.is_none() && !self.compress_enabled()
    }

    fn object_path(&self, hash: &str) -> PathBuf {
        // Guard against a malformed hash producing a path outside the store.
        let prefix = if hash.len() >= 2 { &hash[..2] } else { "00" };
        self.root.join(prefix).join(hash)
    }

    pub fn exists(&self, hash: &str) -> bool {
        self.object_path(hash).is_file()
    }

    /// Store a file in the CAS. The object key is BLAKE3 of the plaintext; the
    /// bytes on disk are plaintext or encrypted depending on the store mode.
    /// If an object with the same hash already exists we dedup. Returns
    /// `(hash, plaintext_size)`.
    pub fn put_file(&self, src: &Path) -> Result<(Hash, u64)> {
        if self.raw_mode() {
            self.put_file_plain(src)
        } else {
            self.put_file_transform(src)
        }
    }

    /// Plaintext path: stream copy + hash, no buffering of the whole file.
    fn put_file_plain(&self, src: &Path) -> Result<(Hash, u64)> {
        let tmp = self.root.join(".incoming").join(format!("in-{}", new_id()));
        let mut reader = fs::File::open(src)?;
        let mut writer = fs::File::create(&tmp)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK];
        let mut total: u64 = 0;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n])?;
            total += n as u64;
        }
        writer.sync_all()?;
        drop(writer);

        let hash = hasher.finalize().to_hex().to_string();
        self.commit_temp(&tmp, &hash)?;
        Ok((hash, total))
    }

    /// Transform path (compression and/or encryption): hash the plaintext, then
    /// compress (LZMA2) and/or AEAD-seal it. Whole-object in memory — fine for
    /// save-sized files. The on-disk layout is, with the plaintext hash as key:
    ///   compress only:        `lzma2(plaintext)`
    ///   encrypt only:         `GSE1 || nonce || enc(plaintext)`
    ///   compress + encrypt:   `GSE1 || nonce || enc(lzma2(plaintext))`
    fn put_file_transform(&self, src: &Path) -> Result<(Hash, u64)> {
        let plain = fs::read(src)?;
        let total = plain.len() as u64;
        let hash = blake3::hash(&plain).to_hex().to_string();
        if self.object_path(&hash).is_file() {
            return Ok((hash, total));
        }

        let mut payload = if self.compress_enabled() {
            lzma_compress(&plain)?
        } else {
            plain
        };
        if let Some(cipher) = &self.cipher {
            let nonce = random_bytes(NONCE_LEN);
            let ct = cipher
                .encrypt(XNonce::from_slice(&nonce), payload.as_ref())
                .map_err(|_| Error::Integrity("encryption failed".into()))?;
            let mut blob = Vec::with_capacity(ENC_MAGIC.len() + NONCE_LEN + ct.len());
            blob.extend_from_slice(ENC_MAGIC);
            blob.extend_from_slice(&nonce);
            blob.extend_from_slice(&ct);
            payload = blob;
        }

        let tmp = self.root.join(".incoming").join(format!("in-{}", new_id()));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&payload)?;
            f.sync_all()?;
        }
        self.commit_temp(&tmp, &hash)?;
        Ok((hash, total))
    }

    /// Decode an on-disk object back to plaintext using the store's modes.
    fn decode(&self, blob: &[u8]) -> Result<Vec<u8>> {
        let inner = match &self.cipher {
            Some(cipher) => decrypt_blob(cipher, blob)?,
            None => blob.to_vec(),
        };
        if self.compress_enabled() {
            lzma_decompress(&inner)
        } else {
            Ok(inner)
        }
    }

    /// Move a finished temp file into its content-addressed slot, deduping if a
    /// concurrent writer beat us to it.
    fn commit_temp(&self, tmp: &Path, hash: &str) -> Result<()> {
        let dest = self.object_path(hash);
        if dest.is_file() {
            let _ = fs::remove_file(tmp);
            return Ok(());
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        // rename is atomic within the same filesystem (store + .incoming share it).
        fs::rename(tmp, &dest)?;
        Ok(())
    }

    /// Copy/decrypt an object out of the store to `dest` (creating parent dirs).
    pub fn copy_to(&self, hash: &str, dest: &Path) -> Result<()> {
        let src = self.object_path(hash);
        if !src.is_file() {
            return Err(Error::Integrity(format!("missing object {hash}")));
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        if self.raw_mode() {
            fs::copy(&src, dest)?;
        } else {
            let plain = self.decode(&fs::read(&src)?)?;
            fs::write(dest, &plain)?;
        }
        Ok(())
    }

    /// Read an object's raw on-disk bytes (encrypted form if the store is
    /// encrypted). Used to replicate objects to a remote verbatim.
    pub fn object_bytes(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.object_path(hash);
        if !path.is_file() {
            return Err(Error::Integrity(format!("missing object {hash}")));
        }
        Ok(fs::read(path)?)
    }

    /// Write raw object bytes received from a remote, then verify they are sound
    /// (plaintext re-hash, or decrypt-and-hash when encrypted). Rejects bytes
    /// that don't match the claimed hash so a corrupt/incompatible remote can't
    /// poison the store.
    pub fn ingest_raw(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        if self.exists(hash) {
            return Ok(());
        }
        let tmp = self.root.join(".incoming").join(format!("in-{}", new_id()));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        self.commit_temp(&tmp, hash)?;
        if !self.verify_object(hash)? {
            let _ = self.remove_object(hash);
            return Err(Error::Integrity(format!(
                "object {hash} from remote failed verification (wrong key or corrupt)"
            )));
        }
        Ok(())
    }

    /// List the hashes of every object currently stored (ignoring `.incoming`).
    pub fn list_objects(&self) -> Result<Vec<Hash>> {
        let mut out = Vec::new();
        // store/<2-hex>/<hash>
        for shard in fs::read_dir(&self.root)? {
            let shard = shard?;
            let name = shard.file_name();
            if name == ".incoming" || !shard.file_type()?.is_dir() {
                continue;
            }
            for obj in fs::read_dir(shard.path())? {
                let obj = obj?;
                if obj.file_type()?.is_file() {
                    if let Some(h) = obj.file_name().to_str() {
                        out.push(h.to_string());
                    }
                }
            }
        }
        Ok(out)
    }

    /// Size of a stored object in bytes (0 if missing).
    pub fn object_size(&self, hash: &str) -> u64 {
        fs::metadata(self.object_path(hash))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Remove an object from the store. Used only by the garbage collector after
    /// it has confirmed nothing references the hash.
    pub fn remove_object(&self, hash: &str) -> Result<()> {
        let path = self.object_path(hash);
        if path.is_file() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// Re-hash a file on disk and compare against `expected`.
    pub fn verify_file(path: &Path, expected: &str) -> Result<bool> {
        let mut reader = fs::File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().to_hex().to_string() == expected)
    }

    /// Verify a stored object matches its own hash (detects bit-rot, and for
    /// encrypted stores also that it decrypts and the plaintext is intact).
    pub fn verify_object(&self, hash: &str) -> Result<bool> {
        let path = self.object_path(hash);
        if !path.is_file() {
            return Ok(false);
        }
        if self.raw_mode() {
            return Self::verify_file(&path, hash);
        }
        match self.decode(&fs::read(&path)?) {
            Ok(plain) => Ok(blake3::hash(&plain).to_hex().to_string() == hash),
            Err(_) => Ok(false),
        }
    }
}

/// LZMA2 (xz) compression — the codec family used by 7-Zip.
fn lzma_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut enc = xz2::write::XzEncoder::new(Vec::new(), LZMA_LEVEL);
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

fn lzma_decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    xz2::read::XzDecoder::new(data).read_to_end(&mut out)?;
    Ok(out)
}

/// Decrypt an on-disk encrypted object blob (`GSE1 || nonce || ciphertext`).
fn decrypt_blob(cipher: &XChaCha20Poly1305, blob: &[u8]) -> Result<Vec<u8>> {
    let header = ENC_MAGIC.len();
    if blob.len() < header + NONCE_LEN || &blob[..header] != ENC_MAGIC {
        return Err(Error::Integrity("not a valid encrypted object".into()));
    }
    let nonce = &blob[header..header + NONCE_LEN];
    let ct = &blob[header + NONCE_LEN..];
    cipher
        .decrypt(XNonce::from_slice(nonce), ct)
        .map_err(|_| Error::Integrity("object failed to decrypt".into()))
}
