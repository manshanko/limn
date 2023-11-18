#![feature(lazy_cell)]
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::path::Path;
use std::path::PathBuf;

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
mod scoped_fs;
use scoped_fs::ScopedFs;

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

    let darktide_path = steam_find::get_steam_app(1361210).map(|app| app.path);
    let bundle_path;
    let path = if arg.as_ref().filter(|p| p == &"-").is_some() {
        match darktide_path {
            Ok(ref path) => {
                bundle_path = path.join("bundle");
                Some(bundle_path.as_ref())
            }
            Err(e) => {
                eprintln!("Darktide directory could not be found automatically");
                eprintln!();
                return Err(Box::new(e));
            }
        }
    } else {
        arg.as_ref().map(Path::new)
    };

    if let Some(path) = path {
        let oodle = match oodle::Oodle::load("oo2core_8_win64.dll") {
            Ok(oodle) => oodle,
            Err(e) => {
                if let Some(oodle) = path.parent().map(|p| p.join("binaries/oo2core_8_win64.dll"))
                    .and_then(|p| oodle::Oodle::load(p).ok())
                    .or_else(|| darktide_path.ok().map(|path| path.join("binaries/oo2core_8_win64.dll"))
                        .and_then(|p| oodle::Oodle::load(p).ok()))
                {
                    oodle
                } else {
                    eprintln!("oo2core_8_win64.dll could not be loaded");
                    eprintln!("copy the dll from the Darktide binaries folder next to limn");
                    eprintln!();
                    return Err(Box::new(e));
                }
            }
        };

        let options = ExtractOptions {
            target: path,
            out: ScopedFs::new(Path::new("./out")),
            oodle: &oodle,
            dictionary: &dictionary,
            dictionary_short: &dictionary.iter().map(|(k, v)| (k.clone_short(), *v)).collect(),
            skip_unknown,
            as_blob: false,
        };

        let filter = args.next().as_ref()
            .and_then(|a| a.to_str())
            .map(|s| hash::murmurhash64(s.as_bytes()));

        let start = Instant::now();
        let num_files = if let Ok(read_dir) = fs::read_dir(path) {
            let mut bundles = Vec::new();
            for fd in read_dir {
                let fd = fd.as_ref().unwrap();
                let meta = fd.metadata().unwrap();
                if meta.is_file() {
                    let path = fd.path();
                    if path.extension().is_some() {
                        continue;
                    }

                    if let Some(bundle_hash) = bundle_hash_from(&path) {
                        bundles.push((path, bundle_hash));
                    }
                }
            }

            let duplicates = Mutex::new(HashMap::with_capacity(0x10000));
            let num_threads = thread::available_parallelism()
                .map(|i| i.get())
                .unwrap_or(0)
                .saturating_sub(1)
                .max(1);

            batch_threads(
                num_threads,
                &bundles,
                &duplicates,
                &options,
                filter,
            )
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

        let ms = start.elapsed().as_millis();
        println!();
        println!("DONE");
        println!("took {}.{}s", ms / 1000, ms % 1000);
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

fn batch_threads(
    num_threads: usize,
    bundles: &[(PathBuf, u64)],
    duplicates: &Mutex<HashMap<(u64, u64), u64>>,
    options: &ExtractOptions,
    filter: Option<u64>,
) -> u32 {
    let bundle_index = AtomicUsize::new(0);

    thread::scope(|s| {
        let mut threads = Vec::with_capacity(num_threads);
        for _ in 0..num_threads {
            threads.push(s.spawn(|| thread_work(
                &bundles,
                &bundle_index,
                &duplicates,
                &options,
                filter,
            )));
        }

        let mut prev = (0, Instant::now());
        loop {
            thread::sleep(std::time::Duration::from_millis(1));

            let is_finished = threads.iter().all(|t| t.is_finished());
            if is_finished {
                if prev.0 != bundles.len() {
                    println!("{}", bundles.len());
                }
                break;
            } else if prev.1.elapsed().as_millis() > 50 {
                let count = bundle_index.load(Ordering::Relaxed).min(bundles.len());
                if count == prev.0 {
                    continue;
                }

                println!("{count}");
                prev = (count, Instant::now());
            }
        }

        let mut num_files = 0;
        for thread in threads {
            num_files += thread.join().unwrap();
        }
        num_files
    })
}

fn thread_work(
    bundles: &[(PathBuf, u64)],
    bundle_index: &AtomicUsize,
    duplicates: &Mutex<HashMap<(u64, u64), u64>>,
    options: &ExtractOptions,
    filter: Option<u64>,
) -> u32 {
    let mut pool = Pool::new();
    let mut buffer_reader = vec![0_u8; 0x80000];
    let mut bundle_buf = Vec::new();
    let mut num_files = 0;

    while let Some((path, bundle_hash)) =
        bundles.get(bundle_index.fetch_add(1, Ordering::Relaxed))
    {
        let bundle = File::open(&path).unwrap();
        let mut rdr = ChunkReader::new(&mut buffer_reader, bundle);
        num_files += extract_bundle(
            &mut pool,
            &mut rdr,
            &mut bundle_buf,
            Some(*bundle_hash),
            Some(&duplicates),
            &options,
            filter,
        ).unwrap();
    }

    num_files
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
        let mut count = 0;
        for file in bundle.index() {
            if file.ext == filter_ext {
                if options.skip_unknown
                    && !options.dictionary.contains_key(&MurmurHash::from(file.name))
                {
                    continue;
                }
                count += 1;
            }
        }

        if count == 0 {
            return Ok(0);
        } else {
            Some(count)
        }
    } else {
        None
    };

    let mut count = 0;
    let mut files = bundle.files(options.oodle, bundle_buf);
    while let Ok(Some(file)) = files.next_file().map_err(|e| panic!("{:016x} - {}", bundle_hash.unwrap_or(0), e)) {
        if options.skip_unknown
            && file.ext != /*lua*/0xa14e8dfa2cd117e2
            && !(filter == Some(file.ext) && file.ext == /*strings*/0x0d972bab10b40fd3)
            && !options.dictionary.contains_key(&MurmurHash::from(file.name))
        {
            continue;
        }

        if let Some(filter_ext) = filter {
            let num_targets = num_targets.as_mut().unwrap();
            if *num_targets == 0 {
                break;
            }

            if file.ext != filter_ext {
                continue;
            } else {
                *num_targets -= 1;
            }
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

        match file::extract(file, pool, options) {
            Ok(_wrote) => count += 1,
            Err(_e) => (),//eprintln!("{e}"),
        }
    }

    Ok(count)
}

fn bundle_hash_from(path: &Path) -> Option<u64> {
    let name = path.file_stem()?;
    u64::from_str_radix(name.to_str()?, 16).ok()
}
