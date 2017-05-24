use std::fs;
use std::io;

use tempfile::tempfile;

// magic
use std::io::Seek;

pub trait Tee: io::BufRead {
    fn reset(&mut self) -> io::Result<()>;
    fn len_and_reset(&mut self) -> io::Result<u64>;
    fn as_seekable(&mut self) -> Option<&mut Seeker>;
}

pub struct TempFileTee {
    inner: io::BufReader<fs::File>,
}

impl TempFileTee {
    pub fn new<U: io::Read>(from: U) -> io::Result<TempFileTee> {
        // TODO: take a size hint, and consider using memory, or shm,
        // TODO: or take a temp file path, or..
        let mut tmp = tempfile()?;

        {
            let mut reader = io::BufReader::new(from);
            let mut writer = io::BufWriter::new(&tmp);
            io::copy(&mut reader, &mut writer)?;
        }

        tmp.seek(BEGINNING)?;

        Ok(TempFileTee {
            inner: io::BufReader::new(tmp),
        })
    }
}

const BEGINNING: io::SeekFrom = io::SeekFrom::Start(0);
const END: io::SeekFrom = io::SeekFrom::End(0);

impl Tee for TempFileTee {
    fn reset(&mut self) -> io::Result<()> {
        self.inner.seek(BEGINNING).map(|_| ())
    }

    fn len_and_reset(&mut self) -> io::Result<u64> {
        let len = self.inner.seek(END)?;
        self.reset()?;
        Ok(len)
    }

    fn as_seekable(&mut self) -> Option<&mut Seeker> {
        Some(&mut self.inner)
    }
}

// Look, I didn't want to implement these. I wanted to return the implementation.
// But I couldn't make it compile, and I might care enough eventually.
impl io::Read for TempFileTee {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl io::BufRead for TempFileTee {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

pub struct BufReaderTee<R: io::Read> {
    inner: Box<io::BufReader<R>>,
}

impl<R: io::Read> BufReaderTee<R> {
    pub fn new(from: R) -> Self {
        BufReaderTee {
            inner: Box::new(io::BufReader::new(from))
        }
    }
}

impl<R: io::Read> Tee for BufReaderTee<R>
    where R: io::Seek + 'static
{
    fn reset(&mut self) -> io::Result<()> {
        self.inner.seek(io::SeekFrom::Start(0)).map(|_| ())
    }

    fn len_and_reset(&mut self) -> io::Result<u64> {
        let len = self.inner.seek(END)?;
        self.reset()?;
        Ok(len)
    }

    fn as_seekable(&mut self) -> Option<&mut Seeker> {
        Some(&mut *self.inner)
    }
}

impl<R: io::Read> io::Read for BufReaderTee<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: io::Read> io::BufRead for BufReaderTee<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

pub struct FailingTee<T> {
    inner: Box<T>,
}

impl<U: io::Read> FailingTee<io::BufReader<U>> {
    pub fn new(from: U) -> Self {
        FailingTee {
            inner: Box::new(io::BufReader::new(from))
        }
    }
}

impl<T> Tee for FailingTee<T>
    where T: io::BufRead
{
    fn reset(&mut self) -> io::Result<()> {
        unreachable!();
    }

    fn len_and_reset(&mut self) -> io::Result<u64> {
        unreachable!();
    }

    fn as_seekable(&mut self) -> Option<&mut Seeker> {
        None
    }
}

impl<T> io::Read for FailingTee<T>
    where T: io::Read {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<T> io::BufRead for FailingTee<T>
    where T: io::BufRead {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

pub trait Seeker: io::Seek + io::Read {
}

impl<R: io::Read> Seeker for io::BufReader<R>
    where R: io::Seek {
}
