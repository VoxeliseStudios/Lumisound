//! Files the app hands over as an open descriptor instead of a path.
//!
//! On Android a song in the media library is a `content://` URI: the app can
//! open it, but there is no path the engine may open itself (re-opening the
//! descriptor through `/proc/self/fd` is refused for media-store files). The
//! app passes `fd:<number>:<file name>` wherever a path is expected; the
//! name is only there for its extension (decoder hint).
//!
//! The engine opens a track more than once (the deck, the gapless header
//! read, every seek), and descriptors duplicated from one another share a
//! single file position. So nothing here ever moves that position: each
//! `FdFile` keeps its own and reads with `pread`.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The descriptor in an `fd:<number>:<name>` pseudo-path.
pub fn descriptor(path: &Path) -> Option<i32> {
    let s = path.to_str()?.strip_prefix("fd:")?;
    s.split(':').next()?.parse().ok().filter(|n| *n >= 0)
}

pub struct FdFile {
    #[cfg(unix)]
    file: std::fs::File,
    pos: u64,
    len: u64,
}

impl FdFile {
    /// Opens the descriptor named by `path` (see the module docs). The
    /// caller's descriptor stays the caller's: this works on a duplicate.
    #[cfg(unix)]
    pub fn open(path: &Path) -> Option<std::io::Result<Self>> {
        use std::os::fd::{BorrowedFd, FromRawFd, IntoRawFd};
        let fd = descriptor(path)?;
        // SAFETY: the app guarantees `fd` is open for as long as the track is queued.
        let owned = match unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned() {
            Ok(o) => o,
            Err(e) => return Some(Err(e)),
        };
        let file = unsafe { std::fs::File::from_raw_fd(owned.into_raw_fd()) };
        let len = match file.metadata() {
            Ok(m) => m.len(),
            Err(e) => return Some(Err(e)),
        };
        Some(Ok(Self { file, pos: 0, len }))
    }

    #[cfg(not(unix))]
    pub fn open(path: &Path) -> Option<std::io::Result<Self>> {
        descriptor(path).map(|_| Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "descriptor sources need a unix platform")))
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Read for FdFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            let n = self.file.read_at(buf, self.pos)?;
            self.pos += n as u64;
            Ok(n)
        }
        #[cfg(not(unix))]
        {
            let _ = buf;
            Ok(0)
        }
    }
}

impl Seek for FdFile {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if target < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before the start"));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }
}

impl symphonia::core::io::MediaSource for FdFile {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn two_readers_on_one_descriptor_do_not_disturb_each_other() {
        let p = std::env::temp_dir().join(format!("lumi-fdfile-{}", std::process::id()));
        std::fs::write(&p, (0u8..200).collect::<Vec<u8>>()).unwrap();
        let f = std::fs::File::open(&p).unwrap();
        let name = format!("fd:{}:Song.mp3", f.as_raw_fd());
        let path = Path::new(&name);
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("mp3"));
        let mut a = FdFile::open(path).unwrap().unwrap();
        let mut b = FdFile::open(path).unwrap().unwrap();
        assert_eq!(a.len(), 200);
        let mut x = [0u8; 10];
        a.read_exact(&mut x).unwrap();
        b.seek(SeekFrom::End(-5)).unwrap();
        let mut y = [0u8; 5];
        b.read_exact(&mut y).unwrap();
        a.read_exact(&mut x).unwrap();
        assert_eq!(x[0], 10);
        assert_eq!(y, [195, 196, 197, 198, 199]);
        assert!(FdFile::open(Path::new("/tmp/plain.mp3")).is_none());
        std::fs::remove_file(p).ok();
    }
}
