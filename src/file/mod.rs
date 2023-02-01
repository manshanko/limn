use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Component;
use std::path::Path;
use crate::bundle::Entry;
use crate::oodle::Oodle;
use crate::hash::MurmurHash;
use crate::hash::FILE_EXTENSION;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use byteorder::LE;

mod lua;
mod texture;

macro_rules! write_help {
    ($dst:expr, $($arg:tt)*) => {{
        let len = $dst.len();
        let mut into = &mut $dst[..];
        write!(&mut into, $($arg)*).unwrap();
        let size = len - into.len();
        let slice;
        (slice, $dst) = $dst.split_at_mut(size);
        std::str::from_utf8(slice).unwrap()
    }}
}

trait Extractor {
    fn extract(
        &self,
        entry: &mut Entry<'_, '_>,
        file_path: &Path,
        shared: &mut [u8],
        shared2: &mut Vec<u8>,
        options: &ExtractOptions<'_>,
    ) -> io::Result<u64>;
}

pub(crate) struct ExtractOptions<'a> {
    pub(crate) target: &'a Path,
    pub(crate) out: &'a Path,
    pub(crate) oodle: &'a Oodle,
    pub(crate) dictionary: &'a HashMap<MurmurHash, &'a str>,
    pub(crate) skip_unknown: bool,
    pub(crate) as_blob: bool,
}

pub(crate) fn extract(
    mut entry: Entry<'_, '_>,
    pool: &mut Pool,
    options: &ExtractOptions<'_>,
) -> io::Result<u64> {
    let extractor: Option<&'static dyn Extractor> = 'res: {Some(match entry.ext {
        0xa14e8dfa2cd117e2 => &lua::LuaParser,
        0xcd4238c6a0c69e32 => &texture::TextureParser,
        _ => break 'res None,
    })};

    let Pool {
        shared,
        shared2,
    } = pool;
    if shared.len() < 0x40000 {
        shared.resize(0x40000, 0);
    }
    let mut shared = &mut shared[..];

    let file_name = match options.dictionary.get(&MurmurHash::from(entry.name)) {
        Some(s) => s,
        None => write_help!(shared, "{:016x}", entry.name),
    };

    let ext_name = match FILE_EXTENSION.binary_search_by(|probe| probe.0.cmp(&entry.ext)) {
        Ok(i) => FILE_EXTENSION[i].1,
        Err(_) => write_help!(shared, "{:016x}", entry.ext),
    };

    if options.as_blob || extractor.is_none() {
        let (out, _) = path_concat(options.out, &mut shared, file_name, Some(ext_name));

        shared2.clear();
        shared2.reserve(0x1000);

        fs::create_dir_all(out.parent().unwrap())?;
        shared2.write_u64::<LE>(entry.ext).unwrap();
        shared2.write_u64::<LE>(entry.name).unwrap();
        let variants = entry.variants();
        shared2.write_u32::<LE>(variants.len() as u32).unwrap();
        shared2.write_u32::<LE>(0).unwrap();
        for variant in variants.iter() {
            shared2.write_u32::<LE>(variant.kind).unwrap();
            shared2.write_u8(variant.unknown1).unwrap();
            shared2.write_u32::<LE>(variant.body_size).unwrap();
            shared2.write_u8(variant.unknown2).unwrap();
            shared2.write_u32::<LE>(variant.tail_size).unwrap();
        }

        let mut fd = fs::File::create(&out).unwrap();
        io::copy(&mut &shared2[..], &mut fd).unwrap();
        io::copy(&mut entry, &mut fd).map(|copied| copied + shared2.len() as u64)
    } else {
        let (out, shared) = path_concat(options.out, &mut shared, file_name, Some(ext_name));

        let extractor = extractor.unwrap();
        extractor.extract(&mut entry, out, shared, shared2, options)
    }
}

// second shared buffer is necessary for resizing after slices have been made
pub(crate) struct Pool {
    shared: Vec<u8>,
    shared2: Vec<u8>,
}

impl Pool {
    pub(crate) fn new() -> Self {
        Self {
            shared: Vec::new(),
            shared2: Vec::new(),
        }
    }
}

fn split_vec<'a, const N: usize>(
    buffer: &'a mut Vec<u8>,
    parts: [usize; N],
) -> ([&'a mut [u8]; N], &'a mut [u8]) {
    let len_needed = parts.iter().sum();
    if len_needed > buffer.len() {
        if len_needed > buffer.capacity() {
            buffer.reserve(len_needed * 2);
        }

        buffer.resize(len_needed, 0);
    }

    let mut buffer = &mut buffer[..];
    let mut bufs = parts.map(|_| None);
    for (i, buf) in bufs.iter_mut().enumerate() {
        let len = parts[i];
        let slice;
        (slice, buffer) = buffer.split_at_mut(len);
        *buf = Some(slice);
    }
    (bufs.map(|buf| buf.unwrap()), buffer)
}

fn no_escape(path: &Path) -> bool {
    for part in path.components() {
        match part {
            //Component::RootDir => return false,
            Component::ParentDir => return false,
            _ => (),
        }
    }
    true
}

fn path_concat<'a>(
    root: &Path,
    buffer: &'a mut [u8],
    file_name: &str,
    ext_name: Option<&str>,
) -> (&'a Path, &'a mut [u8]) {
    let root = root.to_str().unwrap();
    let total = buffer.len();
    let mut into = &mut buffer[..];
    write!(&mut into, "{root}/{file_name}").unwrap();
    if let Some(ext_name) = ext_name {
        write!(&mut into, ".{ext_name}").unwrap();
    }
    let len = total - into.len();
    let (slice, buf) = buffer.split_at_mut(len);
    let path = std::str::from_utf8(slice).unwrap();
    let path = Path::new(path);
    assert!(no_escape(path), "{}", path.display());
    (path, buf)
}

fn data_path_from(buffer: &[u8]) -> Option<&str> {
    match std::ffi::CStr::from_bytes_until_nul(&buffer) {
        Ok(s) => s.to_str().ok(),
        Err(_) => std::str::from_utf8(buffer).ok(),
    }
}




