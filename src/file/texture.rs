//! Extractor for Darktide `texture` files.
//!
//! The `texture` format in Darktide is a small wrapper around a DDS texture.
//! DDS textures with mipmaps will stored larger mipmaps at a `data/*/*` path.
//! Larger mipmaps are chunked to maintain width of smallest mipmap.
//!
//! Only the largest mipmap in a `texture` is extracted. When extracting the
//! mipmap chunks are sorted to restore image dimensions.

use std::fs::File;
use crate::read::ChunkReader;
use super::*;

const DDSD_MIPMAPCOUNT: u32 = 0x20000;

pub(crate) struct TextureParser;

impl Extractor for TextureParser {
    fn extract(
        &self,
        entry: &mut Entry<'_, '_>,
        file_path: &Path,
        mut shared: &mut [u8],
        memory_pool: &mut Vec<u8>,
        options: &ExtractOptions<'_>,
    ) -> io::Result<u64> {
        let variants = entry.variants();
        assert_eq!(1, variants.len());
        let prime = &variants[0];
        let body_size = prime.body_size;
        let tail_size = prime.tail_size;
        assert!(tail_size <= 31);

        let has_high_res = prime.unknown1 == 0 && tail_size > 0;
        let unknown1 = prime.unknown1;
        let mut either_rdr = match unknown1 {
            0 => Ok(entry),
            1 => {
                assert_eq!(0, tail_size);

                let mut data_path = [0_u8; 31];
                entry.read(&mut data_path[..body_size as usize]).unwrap();
                let file = data_path_from_buffer(shared, options.target, &data_path).unwrap();
                let slice;
                (slice, shared) = shared.split_at_mut(0x10000);
                Err(ChunkReader::new(slice, file))
            }
            unk => panic!("unexpected Entry.unknown1 {unk}"),
        };

        let rdr: &mut dyn Read = match &mut either_rdr {
            Ok(f) => f,
            Err(f) => f,
        };

        let kind = rdr.read_u32::<LE>().unwrap();
        assert!(kind == 1 || kind == 0, "unexpected texture type {kind}");

        if kind == 1 {
            let deflate_size = rdr.read_u32::<LE>().unwrap() as usize;
            let inflate_size = rdr.read_u32::<LE>().unwrap() as usize;
            assert!(inflate_size >= 148, "{inflate_size}");

            let ([in_buf, out_buf, scratch], _) = split_vec(memory_pool,
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

                check_dxt10(&out_buf[128..148]);

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

                let mut data_path = [0; 31];
                rdr.read_exact(&mut data_path[..tail_size as usize]).unwrap();
                assert!(rdr.read_u8().is_err());

                let base_width = u32::from_le_bytes(<[u8; 4]>::try_from(&out_buf[16..20]).unwrap());
                let base_pitch = u32::from_le_bytes(<[u8; 4]>::try_from(&out_buf[20..24]).unwrap());

                // assume all textures by this point are block compressed
                let block_size = 4 * base_pitch / base_width;

                let pitch = largest_width / 4 * block_size;
                let mut flags = u32::from_le_bytes(<[u8; 4]>::try_from(&out_buf[8..12]).unwrap());

                // disable flag DDSD_MIPMAPCOUNT for output
                // since only the largest mipmap is extracted
                flags &= !DDSD_MIPMAPCOUNT;

                // patch DDS header to use with largest mipmap
                out_buf[8..12].copy_from_slice(&flags.to_le_bytes());
                out_buf[12..16].copy_from_slice(&largest_height.to_le_bytes());
                out_buf[16..20].copy_from_slice(&largest_width.to_le_bytes());
                out_buf[20..24].copy_from_slice(&pitch.to_le_bytes());
                out_buf[28..32].copy_from_slice(&0_u32.to_le_bytes());
                //out_buf[140..144].copy_from_slice(&0_u32.to_le_bytes());

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

                let data_fd = data_path_from_buffer(shared, options.target, &data_path).unwrap();
                let slice;
                (slice, _) = shared.split_at_mut(0x10000);
                let mut data_rdr = ChunkReader::new(slice, data_fd);

                let mut out_fd = options.out.create(out_path)?;
                out_fd.write_all(&out_buf[..148]).unwrap();
                148 + sort_write_texture_chunks(
                    memory_pool,
                    &options.oodle,
                    &mut data_rdr,
                    &chunks[..num_chunks as usize],
                    chunk_width,
                    chunk_width_pixel,
                    block_size,
                    pitch,
                    &mut out_fd,
                )
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

fn data_path_from_buffer(
    mut shared: &mut [u8],
    target: &Path,
    path: &[u8],
) -> io::Result<File> {
    let path = path.split(|b| *b == 0).next().unwrap();
    let path = data_path_from(&*path).unwrap();
    let path = path_concat(target, &mut shared, path, None);
    assert!(path.starts_with(target));
    let Ok(fd) = File::open(path) else {
        if cfg!(debug_assertions) {
            panic!("failed to load {}", path.display());
        }
        return Err(io::Error::new(io::ErrorKind::NotFound,
            "failed to find texture resource file under data/*/*"));
    };
    Ok(fd)
}

fn check_dxt10(mut dxt10: &[u8]) {
    let _encoding_kind = dxt10.read_u32::<LE>().unwrap();
    let dimension = dxt10.read_u32::<LE>().unwrap();
    let _misc_flags = dxt10.read_u32::<LE>().unwrap();
    let array_size = dxt10.read_u32::<LE>().unwrap();
    let _misc_flags2 = dxt10.read_u32::<LE>().unwrap();
    assert_eq!(3, dimension);
    assert_eq!(1, array_size);
}

// Write out DDS texture while sorting chunks to restore texture dimensions.
//
// For example chunks like
// ```
// 1
// 2
// 3
// 4
// ```
// will be sorted into
// ```
// 12
// 34
// ```
fn sort_write_texture_chunks(
    memory_pool: &mut Vec<u8>,
    oodle: &Oodle,
    data_rdr: &mut ChunkReader<File>,
    chunks: &[u32],
    chunk_width: u32,
    chunk_width_pixel: u32,
    block_size: u32,
    pitch: u32,
    out_fd: &mut File,
) -> u64 {
    let window_size = (pitch * 64) as usize;
    let ([in_buf, out_buf, scratch, window], _) = split_vec(memory_pool,
        [0x11000, 0x10000, 0x100000, window_size]);

    let mut wrote = 0;
    for (i, &chunk) in chunks.iter().enumerate() {
        let in_buf = &mut in_buf[..chunk as usize];
        data_rdr.read_exact(in_buf).unwrap();
        let size = oodle.decompress(in_buf, out_buf, scratch).unwrap();
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
}