use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;

// std::io::BufReader doesn't allow reusing inner buffer
pub struct ChunkReader<'a, R: Read + Seek> {
    inner: R,
    buffer: &'a mut [u8],
    offset: usize,
    len: usize,
    read_chunk: bool,
}

impl<'a, R: Read + Seek> ChunkReader<'a, R> {
    pub fn new(buffer: &'a mut [u8], inner: R) -> Self {
        Self {
            inner,
            buffer,
            offset: 0,
            len: 0,
            read_chunk: true,
        }
    }

    fn next_chunk(&mut self) -> io::Result<()> {
        self.offset = 0;
        self.len = self.inner.read(&mut self.buffer[..]).unwrap();
        Ok(())
    }
}

impl<'a, R: Read + Seek> Read for ChunkReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.read_chunk {
            self.read_chunk = false;
            self.next_chunk()?;
        }
        let mut fill = buf.len();
        let mut read = 0;

        while fill > 0 {
            let copy = (self.len - self.offset).min(fill);
            if copy == 0 {
                if self.offset == self.len {
                    self.next_chunk()?;
                    if self.len != 0 {
                        continue;
                    }
                }
                break;
            } else {
                buf[read..read + copy].copy_from_slice(&self.buffer[self.offset..self.offset + copy]);

                read += copy;
                fill -= copy;
                self.offset += copy;
            }
        }

        Ok(read)
    }
}

impl<'a, R: Read + Seek> Seek for ChunkReader<'a, R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let seek_to = match pos {
            SeekFrom::Current(offset) => self.offset as i64 + offset,
            SeekFrom::Start(offset) => {
                let start = self.inner.seek(SeekFrom::Current(0))? - self.len as u64;
                offset as i64 - start as i64
            }
            SeekFrom::End(_offset) => unimplemented!(),
        };

        if seek_to < 0 || seek_to > self.len as i64 {
            let seek_to = seek_to - self.len as i64;
            self.offset = 0;
            self.len = 0;
            self.read_chunk = true;
            self.inner.seek(SeekFrom::Current(seek_to))
        } else {
            self.offset = seek_to as usize;//(self.offset as i64 + offset) as usize;
            let current = self.inner.seek(SeekFrom::Current(0))?;
            let next = self.inner.seek(SeekFrom::Current(self.offset as i64 - self.len as i64))?;
            self.inner.seek(SeekFrom::Start(current)).map(|_| next)
        }
    }
}
