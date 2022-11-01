#![feature(once_cell)]
use std::io;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::fs;
use std::fs::File;
use std::path::Path;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use byteorder::LE;

mod bundle;
use bundle::BundleFd;
mod hash;
use hash::FILE_EXTENSION;
mod oodle;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os();
    args.next();
    let mut scratch = Vec::new();
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
            .and_then(|s| FILE_EXTENSION.iter().find(|probe| probe.1 == s).map(|(hash, _)| *hash));

        let start = std::time::Instant::now();
        let num_files = if let Some(read_dir) = match fs::read_dir(path) {
            Ok(dir) => Some(dir),
            Err(_) => None,
        } {
            let mut num_files = 0;
            let mut count = 0;
            for fd in read_dir {
                let fd = fd?;
                let meta = fd.metadata()?;
                let path = fd.path();
                let bundle_hash = bundle_hash_from(&path);
                if meta.is_file() && bundle_hash.is_some() {
                    let bundle = File::open(path)?;
                    let mut rdr = BufReader::with_capacity(0x80000, bundle);
                    num_files += extract_bundle(&oodle, &mut scratch, bundle_hash, &mut rdr, Path::new("./out"), filter)?;
                    count += 1;
                    if count % 10 == 0 {
                        println!("{count}");
                    }
                }
            }

            if count % 10 != 0 {
                println!("{count}");
            }
            num_files
        } else if let Ok(bundle) = File::open(path) {
            let bundle_hash = bundle_hash_from(path);
            let mut rdr = BufReader::with_capacity(0x80000, bundle);
            extract_bundle(&oodle, &mut scratch, bundle_hash, &mut rdr, Path::new("./out"), filter)?
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

fn extract_bundle(
    oodle: &oodle::Oodle,
    scratch: &mut Vec<u8>,
    bundle_hash: Option<u64>,
    mut rdr: impl Read + Seek,
    root: &Path,
    filter: Option<u64>,
) -> io::Result<u32> {
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
    let mut lua = String::new();
    let mut out = Vec::new();
    fs::create_dir_all(root)?;
    while let Ok(Some(mut file)) = files.next_file().map_err(|e| panic!("{:016x} - {}", bundle_hash.unwrap_or(0), e)) {
        lua.clear();
        out.clear();

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
                count += 1;
            }
        } else {
            count += 1;
        }

        match file.ext {
            0xa14e8dfa2cd117e2 => {
                for _ in 0..12 {
                    file.read_u8().unwrap();
                }

                let header = file.read_u32::<LE>().unwrap();
                assert_eq!(header, 38423579);

                assert_eq!(file.read_u8().unwrap(), 0);
                let path_len = leb128::read::unsigned(&mut file).unwrap();
                assert_eq!(file.read_u8().unwrap(), b'@');
                for _ in 0..path_len - 1 {
                    lua.push(file.read_u8().unwrap() as char);
                }

                out.write_u32::<LE>(header).unwrap();
                out.write_u8(0).unwrap();
                leb128::write::unsigned(&mut out, path_len).unwrap();
                out.write_u8(b'@').unwrap();
                out.extend(lua.as_bytes());
                println!("{}", &lua);
                io::copy(&mut file, &mut out).unwrap();

                let path = root.join(Path::new(&lua));
                assert!(path.starts_with(root));
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, &out).unwrap();
            }
            _ => {
                let path = if let Some((_, ext)) = FILE_EXTENSION
                    .binary_search_by(|probe| probe.0.cmp(&file.ext))
                    .ok()
                    .and_then(|i| FILE_EXTENSION.get(i))
                {
                    root.join(format!("{:016x}.{}", file.name, ext))
                } else {
                    root.join(format!("{:016x}.{:016x}", file.name, file.ext))
                };
                let mut fd = fs::File::create(&path).unwrap();
                out.write_u64::<LE>(file.ext).unwrap();
                out.write_u64::<LE>(file.name).unwrap();
                let variants = file.variants();
                out.write_u32::<LE>(variants.len() as u32).unwrap();
                out.write_u32::<LE>(0).unwrap();
                for variant in variants.iter() {
                    out.write_u32::<LE>(variant.kind).unwrap();
                    out.write_u8(variant.unknown1).unwrap();
                    out.write_u32::<LE>(variant.body_size).unwrap();
                    out.write_u8(variant.unknown2).unwrap();
                    out.write_u32::<LE>(variant.tail_size).unwrap();
                }

                io::copy(&mut &*out, &mut fd).unwrap();
                io::copy(&mut file, &mut fd).unwrap();
            }
        }
    }

    Ok(count)
}

fn bundle_hash_from(path: &Path) -> Option<u64> {
    let name = path.file_stem()?;
    u64::from_str_radix(name.to_str()?, 16).ok()
}























