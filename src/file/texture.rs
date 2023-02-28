use std::fs::File;
use crate::read::ChunkReader;
use super::*;

pub(crate) struct TextureParser;

impl Extractor for TextureParser {
    fn extract(
        &self,
        entry: &mut Entry<'_, '_>,
        file_path: &Path,
        mut shared: &mut [u8],
        shared_flex: &mut Vec<u8>,
        options: &ExtractOptions<'_>,
    ) -> io::Result<u64> {
        let variants = entry.variants();
        assert_eq!(1, variants.len());
        let prime = &variants[0];

        let has_high_res = prime.unknown1 == 0 && prime.tail_size > 0;
        let unknown1 = prime.unknown1;
        let mut rdr = match unknown1 {
            0 => Ok(entry),
            1 => {
                assert!(prime.tail_size == 0);

                let mut buffer = [0_u8; 31];
                entry.read(&mut buffer).unwrap();
                let data_path = data_path_from(&buffer).unwrap();
                let path = path_concat(options.target, &mut shared, data_path, None);
                assert!(path.starts_with(options.target));
                let Ok(file) = File::open(path) else {
                    if cfg!(debug_assertions) {
                        panic!("failed to load resource file at {data_path}");
                    } else {
                        eprintln!("failed to load resource file at {data_path}");
                    }
                    return Err(io::Error::new(io::ErrorKind::NotFound,
                        "failed to find texture resource file under data/**/*"));
                };
                let slice;
                (slice, shared) = shared.split_at_mut(0x10000);
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
            assert!(inflate_size >= 148, "{inflate_size}");

            let ([in_buf, out_buf, scratch], _) = split_vec(shared_flex,
                [deflate_size, inflate_size, 0x100000]);
            rdr.read_exact(in_buf).unwrap();
            let size = options.oodle.decompress(in_buf, out_buf, scratch).unwrap();
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

            let parent = file_path.parent().unwrap_or(Path::new("."));
            let file_name = file_path.file_stem().unwrap().to_str().unwrap();
            let out_path = path_concat(parent, &mut shared, file_name, Some("dds"));

            let wrote = if meta_size == 0 {
                let _unknown = rdr.read_u32::<LE>().unwrap();
                assert!(rdr.read_u8().is_err());

                options.out.write(out_path, &out_buf)?;
                out_buf.len() as u64
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

                let mut out_fd = options.out.create(out_path)?;
                out_fd.write_all(&out_buf[..148]).unwrap();

                let mut wrote = 148;

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
                let ([in_buf, out_buf, scratch, window], _) = split_vec(shared_flex,
                    [0x11000, 0x10000, 0x100000, window_size]);

                let data_fd = {
                    let data_path = data_path_from(&stream).unwrap();
                    let data_path = path_concat(options.target, &mut shared, data_path, None);
                    assert!(data_path.starts_with(options.target));
                    let Ok(data_fd) = File::open(data_path) else {
                        if cfg!(debug_assertions) {
                            panic!("failed to load resource file at {}", data_path.display());
                        } else {
                            eprintln!("failed to load resource file at {}", data_path.display());
                        }
                        return Err(io::Error::new(io::ErrorKind::NotFound,
                            "failed to find texture resource file under data/**/*"));
                    };
                    data_fd
                };

                let slice;
                (slice, _) = shared.split_at_mut(0x10000);
                let mut data_rdr = ChunkReader::new(slice, data_fd);
                for (i, &chunk) in chunks.iter().take(num_chunks as usize).enumerate() {
                    let in_buf = &mut in_buf[..chunk as usize];
                    data_rdr.read_exact(in_buf).unwrap();
                    let size = options.oodle.decompress(in_buf, out_buf, scratch).unwrap();
                    assert_eq!(size, out_buf.len() as u64);
                    assert_eq!(size, 0x10000);

                    let i = i as u32;
                    if i > 0 && i % chunk_width == 0 {
                        out_fd.write_all(&window).unwrap();
                        wrote += window.len();
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
                wrote += window.len();
                wrote as u64
            };
            Ok(wrote)
        } else if kind == 0 {
            //for _ in 0..8 {
            //    rdr.read_u8().unwrap();
            //}
            //let mut fd = File::create(out_path).unwrap();
            //io::copy(rdr, &mut fd)
            Err(io::Error::new(io::ErrorKind::InvalidData, "unknown texture file kind"))
        } else {
            unreachable!()
        }
    }
}