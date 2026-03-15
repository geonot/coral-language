use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;

pub struct MmapReader {
    data: Arc<MmapInner>,
}

struct MmapInner {
    ptr: *mut u8,
    len: usize,
}

unsafe impl Send for MmapInner {}
unsafe impl Sync for MmapInner {}

impl MmapReader {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let len = metadata.len() as usize;

        if len == 0 {
            return Ok(Self {
                data: Arc::new(MmapInner {
                    ptr: std::ptr::null_mut(),
                    len: 0,
                }),
            });
        }

        #[cfg(unix)]
        let ptr = unsafe {
            let fd = {
                use std::os::unix::io::AsRawFd;
                file.as_raw_fd()
            };
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                fd,
                0,
            );
            if ptr == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            ptr as *mut u8
        };

        #[cfg(not(unix))]
        let ptr = {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "mmap not supported on this platform",
            ));
        };

        Ok(Self {
            data: Arc::new(MmapInner { ptr, len }),
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        if self.data.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.data.ptr, self.data.len) }
    }

    pub fn len(&self) -> usize {
        self.data.len
    }

    pub fn is_empty(&self) -> bool {
        self.data.len == 0
    }

    pub fn read_u32_le(&self, offset: usize) -> Option<u32> {
        if offset + 4 > self.data.len {
            return None;
        }
        let bytes: [u8; 4] = self.as_slice()[offset..offset + 4].try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    pub fn read_u64_le(&self, offset: usize) -> Option<u64> {
        if offset + 8 > self.data.len {
            return None;
        }
        let bytes: [u8; 8] = self.as_slice()[offset..offset + 8].try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    }

    pub fn read_slice(&self, offset: usize, len: usize) -> Option<&[u8]> {
        if offset + len > self.data.len {
            return None;
        }
        Some(&self.as_slice()[offset..offset + len])
    }

    pub fn advise_sequential(&self) {
        #[cfg(unix)]
        if self.data.len > 0 {
            unsafe {
                libc::madvise(
                    self.data.ptr as *mut libc::c_void,
                    self.data.len,
                    libc::MADV_SEQUENTIAL,
                );
            }
        }
    }

    pub fn advise_random(&self) {
        #[cfg(unix)]
        if self.data.len > 0 {
            unsafe {
                libc::madvise(
                    self.data.ptr as *mut libc::c_void,
                    self.data.len,
                    libc::MADV_RANDOM,
                );
            }
        }
    }
}

impl Drop for MmapInner {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.len > 0 {
            #[cfg(unix)]
            unsafe {
                libc::munmap(self.ptr as *mut libc::c_void, self.len);
            }
        }
    }
}

impl Clone for MmapReader {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn mmap_read_write_roundtrip() {
        let dir = std::env::temp_dir().join("coral_mmap_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.bin");

        {
            let mut f = File::create(&path).unwrap();
            let data: Vec<u8> = (0..=255u8).collect();
            f.write_all(&data).unwrap();
        }

        let reader = MmapReader::open(&path).unwrap();
        assert_eq!(reader.len(), 256);
        assert_eq!(reader.as_slice()[0], 0);
        assert_eq!(reader.as_slice()[255], 255);
        assert_eq!(
            reader.read_u32_le(0),
            Some(u32::from_le_bytes([0, 1, 2, 3]))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mmap_empty_file() {
        let dir = std::env::temp_dir().join("coral_mmap_empty_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("empty.bin");
        File::create(&path).unwrap();

        let reader = MmapReader::open(&path).unwrap();
        assert!(reader.is_empty());
        assert_eq!(reader.as_slice(), &[]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
