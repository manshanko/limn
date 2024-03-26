#![allow(dead_code)]
use std::io;
use std::path::Path;
use std::ptr;
use libloading::Symbol;
use libloading::Library;

// https://github.com/gildor2/UEViewer/blob/c444911a6ad65bff5266f273dd5bdf7dd6fb506e/Unreal/UnCoreCompression.cpp#L272
// https://github.com/gildor2/UEViewer/blob/c444911a6ad65bff5266f273dd5bdf7dd6fb506e/Unreal/UnCoreCompression.cpp#L205
#[allow(non_camel_case_types)]
type OodleLZ_Decompress = extern "C" fn(
    arg1: *const u8,
    arg2: u64,
    arg3: *mut u8,
    arg4: u64,
    arg5: ::std::os::raw::c_int,
    arg6: ::std::os::raw::c_int,
    arg7: ::std::os::raw::c_int,
    arg8: *mut u8,
    arg9: u64,
    arg10: *mut ::std::os::raw::c_void,
    arg11: *mut ::std::os::raw::c_void,
    arg12: *mut u8,
    arg13: u64,
    arg14: ::std::os::raw::c_int,
) -> u64;

// https://github.com/jamesbloom/ozip/blob/master/ozip.cpp
#[allow(non_camel_case_types)]
type OodleLZDecoder_MemorySizeNeeded = unsafe extern fn(i32, i64) -> u64;

pub struct Oodle {
    lib: Library,
}

impl Oodle {
    fn load_(path: &Path) -> io::Result<Self> {
        let lib = unsafe {
            Library::new(path).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
        };
        Ok(Self {
            lib,
        })
    }

    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::load_(path.as_ref())
    }

    pub fn memory_size_needed(&self) -> io::Result<u64> {
        unsafe {
            let msn: Symbol<OodleLZDecoder_MemorySizeNeeded> = self.lib.get(b"OodleLZDecoder_MemorySizeNeeded")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            Ok(msn(-1, -1))
        }
    }

    pub fn decompress(&self, data: &[u8], out: &mut [u8], scratch: &mut [u8]) -> io::Result<u64> {
        let ret = unsafe {
            // TODO cache
            let decompress: Symbol<OodleLZ_Decompress> = self.lib.get(b"OodleLZ_Decompress")
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            decompress(
                data.as_ptr(), data.len() as u64,
                out.as_mut_ptr(), out.len() as u64,
                1/*true*/, 0/*false*/, 3,
                ptr::null_mut(), 0, ptr::null_mut(),
                //ptr::null_mut(), ptr::null_mut(), 0,
                ptr::null_mut(), scratch.as_mut_ptr(), scratch.len() as u64,
                3)
        };

        if ret != out.len() as u64 {
            Err(io::Error::new(io::ErrorKind::Other, "failed to decompress data"))
        } else {
            Ok(ret)
        }
    }
}
