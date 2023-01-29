#![feature(once_cell, cstr_from_bytes_until_nul)]
use std::collections::HashSet;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::thread;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::path::Component;
use std::path::Path;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use byteorder::LE;

mod bundle;
use bundle::BundleFd;
mod hash;
use hash::FILE_EXTENSION;
use hash::MurmurHash;
mod oodle;
mod read;
use read::ChunkReader;

macro_rules! write_help {
    ($dst:expr, $($arg:tt)*) => {{
        let len = $dst.len();
        let mut into = &mut $dst[..];
        write!(&mut into, $($arg)*).unwrap();
        let size = len - into.len();
        std::str::from_utf8(&$dst[..size])
    }}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dictionary = fs::read_to_string("dictionary.txt");
    let (dictionary, skip_unknown) = if let Ok(data) = dictionary.as_ref() {
        let mut dict = HashMap::with_capacity(0x1000);
        for key in data.lines() {
            if !key.is_empty() {
                dict.insert(MurmurHash::new(key), key);
            }
        }
        (dict, true)
    } else {
        (HashMap::new(), false)
    };

    let mut args = std::env::args_os();
    args.next();
    let arg = args.next();
    let path = arg.as_ref().map(Path::new);
    if let Some(path) = path {
        let oodle = match oodle::Oodle::load("oo2core_8_win64.dll") {
            Ok(oodle) => oodle,
            Err(e) => {
                if let Some(oodle) = path.parent().map(|p| p.join("binaries/oo2core_8_win64.dll"))
                    .and_then(|p| oodle::Oodle::load(p).ok())
                {
                    oodle
                } else {
                    eprintln!("{e:?}");
                    eprintln!();
                    eprintln!("oo2core_8_win64.dll could not be loaded");
                    eprintln!("copy the dll from the Darktide binaries folder next to limn");
                    return Ok(());
                }
            }
        };

        let filter = args.next().as_ref()
            .and_then(|a| a.to_str())
            .map(|s| hash::murmurhash64(s.as_bytes()));

        let start = std::time::Instant::now();
        let num_files = if let Ok(read_dir) = fs::read_dir(path) {
            let read_dir = read_dir.collect::<Vec<_>>();
            let num_files = AtomicU32::new(0);
            let count = AtomicUsize::new(0);
            let file_i = AtomicUsize::new(0);

            let duplicates = &Mutex::new(HashSet::with_capacity(0x10000));
            let num_threads = thread::available_parallelism()
                .map(|i| i.get())
                .unwrap_or(0)
                .saturating_sub(1)
                .max(1);
            thread::scope(|s| {
                for _ in 0..num_threads {
                    s.spawn(|| {
                        let mut pool = Pool::new();
                        let mut buffer_reader = vec![0_u8; 0x80000];

                        while let Some(fd) = read_dir.get(file_i.fetch_add(1, Ordering::SeqCst)) {
                            let fd = fd.as_ref().unwrap();
                            let meta = fd.metadata().unwrap();
                            let path = fd.path();
                            let bundle_hash = bundle_hash_from(&path);
                            if meta.is_file() && bundle_hash.is_some() && path.extension().is_none() {
                                let bundle = File::open(&path).unwrap();
                                let mut rdr = ChunkReader::new(&mut buffer_reader, bundle);
                                let num = extract_bundle(
                                    &path,
                                    &oodle,
                                    &mut pool,
                                    bundle_hash,
                                    &mut rdr,
                                    Path::new("./out"),
                                    &dictionary,
                                    skip_unknown,
                                    false,
                                    Some(duplicates),
                                    filter,
                                ).unwrap();
                                num_files.fetch_add(num, Ordering::SeqCst);
                                let count = count.fetch_add(1, Ordering::SeqCst);
                                if count > 0 && count % 10 == 0 {
                                    println!("{count}");
                                }
                            }
                        }
                    });
                }
            });

            let count = count.into_inner();
            if count % 10 != 0 {
                println!("{count}");
            }

            num_files.into_inner()
        } else if let Ok(bundle) = File::open(path) {
            let bundle_hash = bundle_hash_from(path);
            let mut buf = vec![0; 0x80000];
            let mut rdr = ChunkReader::new(&mut buf, bundle);
            extract_bundle(
                path,
                &oodle,
                &mut Pool::new(),
                bundle_hash,
                &mut rdr,
                Path::new("./out"),
                &dictionary,
                skip_unknown,
                false,
                None,
                filter,
            )?
        } else {
            panic!("PATH argument was invalid");
        };
        println!();
        println!("DONE");
        println!("took {}ms", start.elapsed().as_millis());
        println!("extracted {num_files} files");
    } else {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        println!("{}", env!("CARGO_PKG_REPOSITORY"));
        println!();
        println!("limn extracts files from resource bundles used in Darktide.");
        println!();
        println!("limn uses oo2core_8_win64.dll to decompress the bundle. If it fails to load");
        println!("oo2core_8_win64.dll then copy it from the Darktide binaries folder next to limn.");
        println!();
        println!("USAGE:");
        println!("limn.exe <PATH> <FILTER>");
        println!();
        println!("ARGS:");
        println!("    <PATH>    A bundle or directory of bundles to extract.");
        println!("    <FILTER>  If present only extract files with this extension.");
    }

    Ok(())
}

// second shared buffer is necessary for resizing after slices have been made
// TODO: refactor for single buffer
struct Pool {
    oodle: Vec<u8>,
    shared_buffer: Vec<u8>,
    shared_buffer2: Vec<u8>,
}

impl Pool {
    fn new() -> Self {
        Self {
            oodle: Vec::new(),
            shared_buffer: Vec::new(),
            shared_buffer2: Vec::new(),
        }
    }
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

fn extract_bundle(
    target: &Path,
    oodle: &oodle::Oodle,
    pool: &mut Pool,
    bundle_hash: Option<u64>,
    mut rdr: impl Read + Seek,
    root: &Path,
    dictionary: &HashMap<MurmurHash, &str>,
    skip_unknown: bool,
    as_blob: bool,
    duplicates: Option<&Mutex<HashSet<(u64, u64)>>>,
    filter: Option<u64>,
) -> io::Result<u32> {
    let shared_buffer = &mut pool.shared_buffer;
    let scratch = &mut pool.oodle;
    let shared_buffer2 = &mut pool.shared_buffer2;
    if shared_buffer2.len() < 0x40000 {
        shared_buffer2.resize(0x40000, 0);
    }

    scratch.clear();
    let mut bundle = BundleFd::new(bundle_hash, &mut rdr)?;
    let mut num_targets = if let Some(filter_ext) = filter {
        let mut pass = false;
        let mut count = 0;
        for file in bundle.index() {
            if file.ext == filter_ext {
                pass = true;
                count += 1;
            }
        }

        if !pass {
            return Ok(0);
        } else {
            Some(count)
        }
    } else {
        None
    };

    let mut count = 0;
    let mut files = bundle.files(oodle, scratch);
    fs::create_dir_all(root)?;
    while let Ok(Some(mut file)) = files.next_file().map_err(|e| panic!("{:016x} - {}", bundle_hash.unwrap_or(0), e)) {
        let mut shared_buffer2 = &mut shared_buffer2[..];
        if let Some(filter_ext) = filter {
            let num_targets = num_targets.as_mut().unwrap();
            if file.ext != filter_ext {
                if *num_targets > 0 {
                    continue;
                } else {
                    break;
                }
            } else {
                *num_targets -= 1;
            }
        }

        let file_name = dictionary.get(&MurmurHash::from(file.name))
            .map(|s| *s)
            .ok_or(file.name);

        if skip_unknown && file_name.is_err() && file.ext != /* lua */0xa14e8dfa2cd117e2 {
            continue;
        }

        let file_name = match file_name {
            Ok(s) => s,
            Err(hash) => {
                let buf;
                (buf, shared_buffer2) = shared_buffer2.split_at_mut(0x10);
                write_help!(buf, "{hash:016x}").unwrap()
            }
        };

        let ext_name = match FILE_EXTENSION.binary_search_by(|probe| probe.0.cmp(&file.ext)) {
            Ok(i) => FILE_EXTENSION[i].1,
            Err(_) => {
                let hash = file.ext;
                let buf;
                (buf, shared_buffer2) = shared_buffer2.split_at_mut(0x10);
                write_help!(buf, "{hash:016x}").unwrap()
            }
        };

        if let Some(duplicates) = duplicates {
            let key = (file.name, file.ext);
            let mut duplicates = duplicates.lock().unwrap();
            if duplicates.get(&key).is_some() {
                continue;
            } else {
                duplicates.insert(key);
            }
        }

        let variants = file.variants();
        match (as_blob, file.ext) {
            // lua
            (false, 0xa14e8dfa2cd117e2) => {
                shared_buffer.clear();
                let lua;
                (lua, shared_buffer2) = shared_buffer2.split_at_mut(0x1000);

                assert_eq!(1, variants.len());
                for _ in 0..12 {
                    file.read_u8().unwrap();
                }

                let header = file.read_u32::<LE>().unwrap();
                assert!(header == 38423579 || header == 2186495515, "{:016x}.lua has unexpected header {header:08x}", file.name);

                assert_eq!(file.read_u8().unwrap(), 0);
                let path_len = leb128::read::unsigned(&mut file).unwrap();
                assert_eq!(file.read_u8().unwrap(), b'@');
                let len = path_len as usize - 1;
                for b in lua[..len].iter_mut() {
                    *b = file.read_u8().unwrap();
                }
                let lua_path = std::str::from_utf8(&lua[..len]).unwrap();

                // always write valid LuaJIT header
                shared_buffer.write_u32::<LE>(38423579).unwrap();
                shared_buffer.write_u8(0).unwrap();
                leb128::write::unsigned(&mut *shared_buffer, path_len).unwrap();
                shared_buffer.write_u8(b'@').unwrap();
                shared_buffer.extend(lua_path.as_bytes());
                //println!("{lua}");
                io::copy(&mut file, &mut *shared_buffer).unwrap();

                let slice;
                (slice, _) = shared_buffer2.split_at_mut(0x1000);
                let path = path_concat(root, slice, lua_path, None);
                //let path = root.join(Path::new(&lua));
                //assert!(path.starts_with(root), "does not start with:\n{}\n{}", root.display(), lua);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, &*shared_buffer).unwrap();
            }
            // texture
            (false, 0xcd4238c6a0c69e32) => {
                assert_eq!(1, variants.len());
                let prime = &variants[0];

                let has_high_res = prime.unknown1 == 0 && prime.tail_size > 0;
                let unknown1 = prime.unknown1;
                let mut rdr = match unknown1 {
                    0 => Ok(file),
                    1 => {
                        assert!(prime.tail_size == 0);

                        let mut buffer = [0_u8; 31];
                        file.read(&mut buffer).unwrap();
                        let data_path = data_path_from(&buffer);
                        let parent = target.parent().unwrap_or_else(|| &Path::new("."));
                        let path = path_concat(parent, shared_buffer2, data_path, None);
                        assert!(path.starts_with(parent));
                        let Ok(file) = File::open(path) else {
                            if cfg!(debug_assertions) {
                                panic!("failed to load resource file at {data_path}");
                            } else {
                                eprintln!("failed to load resource file at {data_path}");
                            }
                            continue;
                        };
                        let slice;
                        (slice, shared_buffer2) = shared_buffer2.split_at_mut(0x10000);
                        Err(ChunkReader::new(slice, file))
                    }
                    unk => panic!("unexpected Entry.unknown1 {unk}"),
                };

                let rdr: &mut dyn Read = match &mut rdr {
                    Ok(f) => f,
                    Err(f) => f,
                };

                let kind = rdr.read_u32::<LE>().unwrap();
                assert!(kind == 1 || kind == 0, "unexpected texture type {kind}");

                if kind == 1 {
                    let deflate_size = rdr.read_u32::<LE>().unwrap() as usize;
                    let inflate_size = rdr.read_u32::<LE>().unwrap() as usize;

                    if shared_buffer.len() < deflate_size + inflate_size + 0x100000 {
                        if shared_buffer.capacity() < deflate_size + inflate_size + 0x100000 {
                            shared_buffer.reserve((deflate_size + inflate_size + 0x100000) * 2);
                        }

                        shared_buffer.resize(deflate_size + inflate_size + 0x100000, 0);
                    }

                    assert!(inflate_size >= 148, "{inflate_size}");
                    let (mut in_buf, shared) = shared_buffer.split_at_mut(deflate_size);
                    let (mut out_buf, shared) = shared.split_at_mut(inflate_size);
                    let (mut scratch, _shared) = shared.split_at_mut(0x100000);
                    rdr.read_exact(&mut in_buf).unwrap();
                    let size = oodle.decompress(&in_buf, &mut out_buf, &mut scratch).unwrap();
                    assert_eq!(size, out_buf.len() as u64);

                    let fourcc = u32::from_le_bytes(<[u8; 4]>::try_from(&out_buf[84..88]).unwrap());

                    assert_eq!(67, rdr.read_u32::<LE>().unwrap());
                    rdr.read_u32::<LE>().unwrap();
                    let _num_mipmaps = rdr.read_u32::<LE>().unwrap();
                    let largest_width = rdr.read_u32::<LE>().unwrap();
                    let largest_height = rdr.read_u32::<LE>().unwrap();
                    let mut skip = [0; 128];

                    rdr.read_exact(&mut skip).unwrap();
                    let _image_size = u32::from_le_bytes(<[u8; 4]>::try_from(&skip[60..64]).unwrap());

                    let meta_size = u16::try_from(rdr.read_u32::<LE>().unwrap()).unwrap();

                    let slice;
                    (slice, _) = shared_buffer2.split_at_mut(0x400);
                    let out_path = path_concat(root, slice, file_name, Some("dds"));
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).unwrap();
                    }

                    if meta_size == 0 {
                        let _unknown = rdr.read_u32::<LE>().unwrap();
                        assert!(rdr.read_u8().is_err());

                        fs::write(out_path, &out_buf).unwrap();
                    } else {
                        assert!(has_high_res);
                        assert_eq!(0x44583130_u32.swap_bytes(), fourcc);

                        let mut dxt10 = &out_buf[128..148];
                        let _encoding_kind = dxt10.read_u32::<LE>().unwrap();
                        let dimension = dxt10.read_u32::<LE>().unwrap();
                        let _misc_flags = dxt10.read_u32::<LE>().unwrap();
                        let array_size = dxt10.read_u32::<LE>().unwrap();
                        let _misc_flags2 = dxt10.read_u32::<LE>().unwrap();
                        assert_eq!(3, dimension);
                        assert_eq!(1, array_size);

                        let num_chunks = u16::try_from(rdr.read_u32::<LE>().unwrap()).unwrap();
                        assert_eq!(8 + num_chunks * 4, meta_size);
                        assert_eq!(0, rdr.read_u16::<LE>().unwrap());
                        assert_eq!(num_chunks, rdr.read_u16::<LE>().unwrap());
                        let mut last = 0;
                        let mut chunks = Vec::with_capacity(num_chunks as usize);
                        for _ in 0..num_chunks {
                            let next = rdr.read_u32::<LE>().unwrap();
                            assert!(next > last);
                            chunks.push(next - last);
                            last = next;
                        }
                        let _unknown = rdr.read_u32::<LE>().unwrap();

                        let mut stream = [0; 31];
                        rdr.read_exact(&mut stream).unwrap();
                        assert!(rdr.read_u8().is_err());

                        let base_width = u32::from_le_bytes(<[u8; 4]>::try_from(&out_buf[16..20]).unwrap());
                        let base_pitch = u32::from_le_bytes(<[u8; 4]>::try_from(&out_buf[20..24]).unwrap());

                        // assume all textures by this point are block compressed
                        let block_size = 4 * base_pitch / base_width;

                        let pitch = largest_width / 4 * block_size;
                        let flags = u32::from_le_bytes(<[u8; 4]>::try_from(&out_buf[8..12]).unwrap());
                        let flags = flags & !0x20000;
                        out_buf[8..12].copy_from_slice(&flags.to_le_bytes());
                        out_buf[12..16].copy_from_slice(&largest_height.to_le_bytes());
                        out_buf[16..20].copy_from_slice(&largest_width.to_le_bytes());
                        out_buf[20..24].copy_from_slice(&pitch.to_le_bytes());
                        out_buf[28..32].copy_from_slice(&0_u32.to_le_bytes());
                        //out_buf[140..144].copy_from_slice(&0_u32.to_le_bytes());

                        let mut out_fd = File::create(out_path)?;
                        out_fd.write_all(&out_buf[..148]).unwrap();

                        let chunk_width_pixel = if block_size == 8 {
                            128
                        } else if block_size == 16 {
                            64
                        } else {
                            unreachable!()
                        };
                        let chunk_width = largest_width / chunk_width_pixel / 4;
                        let chunk_height = largest_height / 64 / 4;
                        let num_chunks = chunk_width * chunk_height;
                        assert!(chunks.len() >= num_chunks as usize);

                        let window_size = (pitch * 64) as usize;
                        if shared_buffer.len() < 0x11000 + 0x10000 + 0x100000 + window_size {
                            if shared_buffer.capacity() < 0x11000 + 0x10000 + 0x100000 + window_size {
                                shared_buffer.reserve((0x11000 + 0x10000 + 0x100000 + window_size) * 2);
                            }

                            shared_buffer.resize(0x11000 + 0x10000 + 0x100000 + window_size, 0);
                        }

                        let (in_buf, shared) = shared_buffer.split_at_mut(0x11000);
                        let (mut out_buf, shared) = shared.split_at_mut(0x10000);
                        let (mut scratch, shared) = shared.split_at_mut(0x100000);
                        let (window, _shared) = shared.split_at_mut(window_size);

                        let data_fd = {
                            let data_path = data_path_from(&stream);
                            let parent = target.parent().unwrap_or_else(|| &Path::new("."));
                            let data_path = path_concat(parent, shared_buffer2, data_path, None);
                            assert!(data_path.starts_with(parent));
                            File::open(data_path)?
                        };

                        let slice;
                        (slice, _) = shared_buffer2.split_at_mut(0x10000);
                        let mut data_rdr = ChunkReader::new(slice, data_fd);
                        for (i, &chunk) in chunks.iter().take(num_chunks as usize).enumerate() {
                            let in_buf = &mut in_buf[..chunk as usize];
                            data_rdr.read_exact(in_buf).unwrap();
                            let size = oodle.decompress(in_buf, &mut out_buf, &mut scratch).unwrap();
                            assert_eq!(size, out_buf.len() as u64);
                            assert_eq!(size, 0x10000);

                            let i = i as u32;
                            if i > 0 && i % chunk_width == 0 {
                                out_fd.write_all(&window).unwrap();
                            }

                            assert_eq!((pitch / chunk_width) as u64, size / 64);

                            let row_size = (chunk_width_pixel * block_size) as usize;
                            let chunk_x = (i % chunk_width) * chunk_width_pixel * block_size;
                            for (row_i, row) in out_buf.chunks_exact(row_size).enumerate() {
                                let row_i = row_i as u32;
                                let start = chunk_x + row_i * pitch;
                                window[start as usize..start as usize + row_size].copy_from_slice(row);
                            }
                        }
                        out_fd.write_all(&window).unwrap();
                    }
                } else if kind == 0 {
                    // looks to be uncompressed
                    // don't know how to handle
                    // only used in 6 texture files as of Darktide 1.0.21
                    continue;
                }
            }
            _ => {
                let path_slice;
                (path_slice, _) = shared_buffer2.split_at_mut(0x1000);
                let out_path = path_concat(root, path_slice, file_name, Some(ext_name));

                shared_buffer.clear();

                fs::create_dir_all(out_path.parent().unwrap())?;
                let mut fd = fs::File::create(&out_path).unwrap();
                shared_buffer.write_u64::<LE>(file.ext).unwrap();
                shared_buffer.write_u64::<LE>(file.name).unwrap();
                let variants = file.variants();
                shared_buffer.write_u32::<LE>(variants.len() as u32).unwrap();
                shared_buffer.write_u32::<LE>(0).unwrap();
                for variant in variants.iter() {
                    shared_buffer.write_u32::<LE>(variant.kind).unwrap();
                    shared_buffer.write_u8(variant.unknown1).unwrap();
                    shared_buffer.write_u32::<LE>(variant.body_size).unwrap();
                    shared_buffer.write_u8(variant.unknown2).unwrap();
                    shared_buffer.write_u32::<LE>(variant.tail_size).unwrap();
                }

                io::copy(&mut &shared_buffer[..], &mut fd).unwrap();
                io::copy(&mut file, &mut fd).unwrap();
            }
        }

        count += 1;
    }

    Ok(count)
}

fn path_concat<'a>(
    root: &Path,
    buffer: &'a mut [u8],
    file_name: &str,
    ext_name: Option<&str>,
) -> &'a Path {
    let root = root.to_str().unwrap();
    let total = buffer.len();
    let mut into = &mut buffer[..];
    write!(&mut into, "{root}/{file_name}").unwrap();
    if let Some(ext_name) = ext_name {
        write!(&mut into, ".{ext_name}").unwrap();
    }
    let len = total - into.len();
    let path = std::str::from_utf8(&buffer[..len]).unwrap();
    let path = Path::new(path);
    assert!(no_escape(path), "{}", path.display());
    path
}

fn data_path_from(buffer: &[u8]) -> &str {
    match std::ffi::CStr::from_bytes_until_nul(&buffer) {
        Ok(s) => s.to_str().unwrap(),
        Err(_) => std::str::from_utf8(&buffer).unwrap(),
    }
}

fn bundle_hash_from(path: &Path) -> Option<u64> {
    let name = path.file_stem()?;
    u64::from_str_radix(name.to_str()?, 16).ok()
}























