#![feature(once_cell, cstr_from_bytes_until_nul)]
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
use std::path::Path;

mod bundle;
use bundle::BundleFd;
mod file;
use file::ExtractOptions;
use file::Pool;
mod hash;
use hash::MurmurHash;
mod oodle;
mod read;
use read::ChunkReader;

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

        let options = ExtractOptions {
            target: path,
            out: Path::new("./out"),
            oodle: &oodle,
            dictionary: &dictionary,
            skip_unknown,
            as_blob: false,
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

            let duplicates = Mutex::new(HashMap::with_capacity(0x10000));
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
                        let mut bundle_buf = Vec::new();

                        while let Some(fd) = read_dir.get(file_i.fetch_add(1, Ordering::SeqCst)) {
                            let fd = fd.as_ref().unwrap();
                            let meta = fd.metadata().unwrap();
                            let path = fd.path();
                            let bundle_hash = bundle_hash_from(&path);
                            if meta.is_file() && bundle_hash.is_some() && path.extension().is_none() {
                                let bundle = File::open(&path).unwrap();
                                let mut rdr = ChunkReader::new(&mut buffer_reader, bundle);
                                let num = extract_bundle(
                                    &mut pool,
                                    &mut rdr,
                                    &mut bundle_buf,
                                    bundle_hash,
                                    Some(&duplicates),
                                    &options,
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
                &mut Pool::new(),
                &mut rdr,
                &mut Vec::new(),
                bundle_hash,
                None,
                &options,
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

fn extract_bundle(
    pool: &mut Pool,
    mut rdr: impl Read + Seek,
    bundle_buf: &mut Vec<u8>,
    bundle_hash: Option<u64>,
    duplicates: Option<&Mutex<HashMap<(u64, u64), u64>>>,
    options: &ExtractOptions<'_>,
    filter: Option<u64>,
) -> io::Result<u32> {
    bundle_buf.clear();
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
    let mut files = bundle.files(options.oodle, bundle_buf);
    fs::create_dir_all(options.out)?;
    while let Ok(Some(file)) = files.next_file().map_err(|e| panic!("{:016x} - {}", bundle_hash.unwrap_or(0), e)) {
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

        let in_dictionary = options.dictionary.get(&MurmurHash::from(file.name)).is_some();

        if options.skip_unknown && !in_dictionary && file.ext != /*lua*/0xa14e8dfa2cd117e2 {
            continue;
        }

        if let Some(duplicates) = duplicates {
            let key = (file.name, file.ext);
            let mut duplicates = duplicates.lock().unwrap();
            if let Some(num_dupes) = duplicates.get_mut(&key) {
                *num_dupes += 1;
                continue;
            } else {
                duplicates.insert(key, 1);
            }
        }

        if let Ok(_wrote) = file::extract(file, pool, options) {
            count += 1;
        }
    }

    Ok(count)
}

fn bundle_hash_from(path: &Path) -> Option<u64> {
    let name = path.file_stem()?;
    u64::from_str_radix(name.to_str()?, 16).ok()
}























