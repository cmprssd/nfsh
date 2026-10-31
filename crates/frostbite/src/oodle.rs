//! Oodle decompression through the install's own `oo2core_6_win64.dll`, loaded at run
//! time from the game folder. nfsh never ships the library: it is the owner's copy, as
//! the game data is. Only Windows can load it; elsewhere [`Oodle::load`] fails.

use std::path::Path;

use crate::cas::Decoder;
use crate::{Result, bail};

/// The library's file name, in the game folder.
pub const DLL: &str = "oo2core_6_win64.dll";

/// `OodleLZ_Decompress`. Arguments after the buffers: fuzz safety, CRC check,
/// verbosity, dictionary base and size, progress callback and its data, decoder
/// memory and its size, thread phase. Returns the bytes written, 0 on failure.
#[cfg_attr(not(windows), allow(dead_code))]
type Decompress = unsafe extern "system" fn(
    *const u8,
    isize,
    *mut u8,
    isize,
    i32,
    i32,
    i32,
    *mut u8,
    isize,
    *const (),
    *const (),
    *mut u8,
    isize,
    i32,
) -> isize;

/// The loaded library. It stays loaded for the rest of the process.
pub struct Oodle {
    #[cfg_attr(not(windows), allow(dead_code))]
    decompress: Decompress,
}

impl Oodle {
    /// Loads the library from the game folder `root`.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(DLL);
        if !path.is_file() {
            bail!("{} is missing", path.display());
        }
        Self::load_file(&path)
    }

    #[cfg(windows)]
    fn load_file(path: &Path) -> Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut ();
            fn GetProcAddress(module: *mut (), name: *const u8) -> *const ();
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is a NUL-terminated path; the library is never unloaded, so
        // the function pointer stays valid.
        let decompress = unsafe {
            let module = LoadLibraryW(wide.as_ptr());
            if module.is_null() {
                bail!("{}: {}", path.display(), std::io::Error::last_os_error());
            }
            let f = GetProcAddress(module, c"OodleLZ_Decompress".as_ptr().cast());
            if f.is_null() {
                bail!("{} has no OodleLZ_Decompress", path.display());
            }
            std::mem::transmute::<*const (), Decompress>(f)
        };
        Ok(Self { decompress })
    }

    #[cfg(not(windows))]
    fn load_file(path: &Path) -> Result<Self> {
        bail!(
            "{}: Oodle can only be loaded on Windows for now",
            path.display()
        )
    }
}

impl Decoder for Oodle {
    #[cfg(windows)]
    fn decompress(&self, src: &[u8], dst: &mut [u8]) -> Result<()> {
        const FUZZ_SAFE: i32 = 1;
        const NO_CRC: i32 = 0;
        const SILENT: i32 = 0;
        const UNTHREADED: i32 = 3;
        // SAFETY: both buffers are valid for the lengths given; with fuzz safety on,
        // the decoder reads and writes only within them, even on corrupt input.
        let written = unsafe {
            (self.decompress)(
                src.as_ptr(),
                src.len() as isize,
                dst.as_mut_ptr(),
                dst.len() as isize,
                FUZZ_SAFE,
                NO_CRC,
                SILENT,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                UNTHREADED,
            )
        };
        if written != dst.len() as isize {
            bail!(
                "Oodle wrote {written} of {} bytes from {} stored",
                dst.len(),
                src.len()
            );
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn decompress(&self, _src: &[u8], _dst: &mut [u8]) -> Result<()> {
        bail!("Oodle can only be loaded on Windows for now")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_the_library_is_an_error() {
        let err = Oodle::load(Path::new("/no/such/heat")).err().unwrap();
        assert!(err.0.ends_with("oo2core_6_win64.dll is missing"), "{err}");
    }
}
