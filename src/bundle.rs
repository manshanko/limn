use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use byteorder::ReadBytesExt;
use byteorder::LE;

use crate::oodle;

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

pub struct BundleFd<'a> {
    rdr: &'a mut dyn ReadSeek,
    pub name: Option<u64>,
    pub num_files: u32,
}

impl<'a> BundleFd<'a> {
    pub fn new(name: Option<u64>, rdr: &'a mut dyn ReadSeek) -> io::Result<Self> {
        let mut header = [0_u8; 8];
        rdr.read_exact(&mut header)?;
        if header != [
            // bundle version
            0x07, 0x00,

            // unknown
            0x00, 0xF0,
            0x03, 0x00,
            0x00, 0x00,
        ] {
            panic!();
        }

        let num_files = rdr.read_u32::<LE>()?;

        Ok(Self {
            rdr,
            name,
            num_files,
        })
    }

    pub fn index(&mut self) -> IndexIter<'_> {
        IndexIter::new(self.rdr, self.num_files)
    }

    fn reader<'b>(&'b mut self, oodle: &'b oodle::Oodle, scratch: &'b mut Vec<u8>) -> OodleRead<'b> {
        let needed = oodle.memory_size_needed().unwrap() as usize;
        if scratch.len() < CHUNK_SIZE * 2 + needed {
            scratch.resize(CHUNK_SIZE * 2 + needed, 0);
        }
        let (in_buf, scratch) = scratch.split_at_mut(CHUNK_SIZE);
        let (out_buf, scratch) = scratch.split_at_mut(CHUNK_SIZE);
        let (scratch, _) = scratch.split_at_mut(needed);
        match OodleRead::new(
            oodle,
            self.rdr,
            self.num_files,
            <&mut [u8; CHUNK_SIZE]>::try_from(in_buf).unwrap(),
            <&mut [u8; CHUNK_SIZE]>::try_from(out_buf).unwrap(),
            scratch,
        ) {
            Ok(o) => o,
            Err(e) => if let Some(bundle_hash) = self.name {
                panic!("{bundle_hash:016x} {e}");
            } else {
                panic!("anonymous bundle {e}");
            }
        }
    }

    pub fn files<'b>(&'b mut self, oodle: &'b oodle::Oodle, scratch: &'b mut Vec<u8>) -> FilesIter<'_> {
        let num_files = self.num_files;
        FilesIter::new(self.reader(oodle, scratch), num_files)
    }
}

pub struct IndexIter<'a> {
    rdr: &'a mut dyn ReadSeek,
    num_files: u32,
    offset: u32,
}

impl<'a> IndexIter<'a> {
    fn new(rdr: &'a mut dyn ReadSeek, num_files: u32) -> Self {
        rdr.seek(SeekFrom::Start(12 + 256)).unwrap();

        Self {
            rdr,
            num_files,
            offset: 0,
        }
    }
}

impl<'a> Iterator for IndexIter<'a> {
    type Item = IndexEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset < self.num_files {
            self.offset += 1;

            Some(IndexEntry {
                ext: self.rdr.read_u64::<LE>().ok()?,
                name: self.rdr.read_u64::<LE>().ok()?,
                mode: self.rdr.read_u32::<LE>().ok()?,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub ext: u64,
    pub name: u64,
    pub mode: u32,
}

fn align_16(offset: u64) -> i64 {
    let offset = (offset % 16) as i64;
    if offset != 0 {
        16 - offset
    } else {
        0
    }
}

const CHUNK_SIZE: usize = 0x80000;

struct OodleRead<'a> {
    oodle: &'a oodle::Oodle,
    rdr: &'a mut dyn ReadSeek,
    in_buf: &'a mut [u8; CHUNK_SIZE],
    out_buf: &'a mut [u8; CHUNK_SIZE],
    scratch: &'a mut [u8],
    offset: usize,
    total_out: usize,
    total_size: u64,
    num_chunks: u32,
    current: u32,
}

impl<'a> OodleRead<'a> {
    fn new(
        oodle: &'a oodle::Oodle,
        rdr: &'a mut dyn ReadSeek,
        num_files: u32,
        in_buf: &'a mut [u8; CHUNK_SIZE],
        out_buf: &'a mut [u8; CHUNK_SIZE],
        scratch: &'a mut [u8],
    ) -> io::Result<Self> {
        rdr.seek(SeekFrom::Start(12 + 256 + u64::from(num_files) * 20)).unwrap();
        let num_chunks = rdr.read_u32::<LE>().unwrap();
        for _ in 0..num_chunks {
            let _chunk_size = rdr.read_u32::<LE>().unwrap();
        }

        let padding = align_16(rdr.stream_position().unwrap());
        if padding > 0 {
            rdr.seek(SeekFrom::Current(padding)).unwrap();
        }
        let total_size = rdr.read_u32::<LE>().unwrap() as u64;
        let zero = rdr.read_u32::<LE>().unwrap();

        if 0 != zero {
            eprintln!("{}: {:08x} (padding {})", rdr.stream_position()? - 4, zero.swap_bytes(), padding);
            Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected non-zero"))
        } else {
            let mut s = Self {
                oodle,
                rdr,
                in_buf,
                out_buf,
                scratch,
                offset: 0,
                total_out: 0,
                total_size,
                num_chunks,
                current: 0,
            };
            s.next()?;
            Ok(s)
        }
    }

    fn next(&mut self) -> io::Result<bool> {
        if self.current < self.num_chunks {
            self.current += 1;
            self.offset = 0;
            let chunk_size = self.rdr.read_u32::<LE>()? as usize;

            let padding = align_16(self.rdr.stream_position().unwrap());
            if padding > 0 {
                self.rdr.seek(SeekFrom::Current(padding)).unwrap();
            }

            self.rdr.read_exact(&mut self.in_buf[..chunk_size])?;

            if chunk_size == CHUNK_SIZE {
                self.out_buf.copy_from_slice(self.in_buf);
            } else {
                let size = self.oodle.decompress(
                    &self.in_buf[..chunk_size],
                    &mut self.out_buf[..CHUNK_SIZE],
                    self.scratch,
                )?;
                assert_eq!(size, CHUNK_SIZE as u64);
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl<'a> Read for OodleRead<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut fill = buf.len();
        let len = fill;
        let mut read = 0;

        while fill > 0 {
            if self.offset == CHUNK_SIZE && !self.next()?{
                return Ok(buf.len() - fill);
            }

            let copy = if self.current == self.num_chunks {
                let rem =  self.total_size as usize % CHUNK_SIZE;
                fill.min(if rem == 0 {
                    CHUNK_SIZE
                } else {
                    rem
                } - self.offset)
            } else {
                fill.min(self.out_buf.len() - self.offset)
            };
            let offset = len - fill;
            buf[offset..offset + copy].copy_from_slice(&self.out_buf[self.offset..self.offset + copy]);
            fill -= copy;

            self.offset += copy;
            self.total_out += copy;
            read += copy;

            if copy == 0 {
                break;
            }
        }

        Ok(read)
    }
}

pub struct FilesIter<'a> {
    oodle: OodleRead<'a>,
    num_files: u32,
    current: u32,
}

impl<'a, 'b: 'a> FilesIter<'b> {
    fn new(oodle: OodleRead<'b>, num_files: u32) -> Self {
        Self {
            oodle,
            num_files,
            current: 0,
        }
    }

    pub fn next_file(&'a mut self) -> io::Result<Option<Entry<'a, 'b>>> {
        if self.current < self.num_files {
            self.current += 1;
            let ext = self.oodle.read_u64::<LE>()?;
            let name = self.oodle.read_u64::<LE>()?;

            let num_variants = self.oodle.read_u32::<LE>().unwrap();

            let mut failed = false;
            for _ in 0..4 {
                if 0 != self.oodle.read_u8().unwrap() {
                    failed = true;
                }
            }

            let mut variants = Vec::new();
            let mut total_size = 0;
            for _ in 0..num_variants {
                let variant_kind = self.oodle.read_u32::<LE>()?;
                let unknown1 = self.oodle.read_u8()?;
                if unknown1 > 1 {
                    failed = true;
                }

                if failed {
                    eprintln!("{} {} - unexpected values",
                        self.oodle.rdr.stream_position().unwrap(),
                        self.oodle.total_out - 13);
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected value"));
                }

                let file_size = self.oodle.read_u32::<LE>()?;
                let unknown2 = self.oodle.read_u8()?;
                assert_eq!(1, unknown2);
                let tail_size = self.oodle.read_u32::<LE>()?;

                total_size += file_size + tail_size;

                variants.push(Variant {
                    kind: variant_kind,
                    unknown1,
                    body_size: file_size,
                    unknown2,
                    tail_size,
                })
            }

            if self.current == self.num_files {
                let size = self.oodle.total_out as u64 + total_size as u64;
                assert_eq!(size, self.oodle.total_size);
            }

            Ok(Some(Entry {
                rdr: &mut self.oodle,
                variants,
                remaining: total_size as usize,
                ext,
                name,
            }))
        } else {
            Ok(None)
        }
    }
}

pub struct Variant {
    pub kind: u32,
    pub unknown1: u8,
    pub body_size: u32,
    pub unknown2: u8,
    pub tail_size: u32,
}

pub struct Entry<'a, 'b: 'a> {
    rdr: &'a mut OodleRead<'b>,
    variants: Vec<Variant>,
    remaining: usize,
    pub ext: u64,
    pub name: u64,
}

impl<'a, 'b: 'a> Entry<'a, 'b> {
    pub fn variants(&self) -> &[Variant] {
        &self.variants
    }
}

impl<'a, 'b: 'a> Read for Entry<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let copy = self.remaining.min(buf.len());
        self.remaining -= copy;
        self.rdr.read(&mut buf[..copy])
    }
}

impl<'a, 'b: 'a> Drop for Entry<'a, 'b> {
    fn drop(&mut self) {
        // TODO add empty method
        io::copy(&mut self.rdr.take(self.remaining as u64), &mut io::sink()).unwrap();
    }
}