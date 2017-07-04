use std::fs;
use std::io;

use tempfile::tempfile;

use unpacker::Unpacker;

use errors::*;

// magic
use std::io::Seek;
use std::io::Write;

pub trait Tee: io::BufRead {
    fn reset(&mut self) -> Result<()>;
    fn len_and_reset(&mut self) -> Result<u64>;
    fn as_seekable(&mut self) -> Result<&mut Seeker>;
}

pub struct TempFileTee {
    inner: io::BufReader<fs::File>,
}

pub fn read_all<R: io::Read>(mut reader: &mut R, mut buf: &mut [u8]) -> io::Result<usize> {
    let mut pos = 0;
    loop {
        match reader.read(&mut buf[pos..]) {
            Ok(0) => return Ok(pos),
            Ok(n) => {
                pos += n;
                if pos == buf.len() {
                    return Ok(pos);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

impl TempFileTee {
    pub fn if_necessary<U: io::Read>(mut from: U, log: &Unpacker) -> Result<Box<Tee>> {
        const MEM_LIMIT: usize = 32 * 1024;
        let mut buf = [0u8; MEM_LIMIT];
        let read = read_all(&mut from, &mut buf)?;
        if read < MEM_LIMIT {
            return Ok(Box::new(
                BufReaderTee::new(io::Cursor::new(buf[..read].to_vec())),
            ));
        }

        let mut tmp = tempfile()?;

        {
            let mut writer = io::BufWriter::new(&tmp);
            writer.write_all(&buf)?;
            let written = io::copy(&mut from, &mut writer)?;
            log.log(3, || {
                format!(
                    "file spills to temp file: {}kB",
                    (MEM_LIMIT as u64 + written) / 1024
                )
            })?;
        }

        tmp.seek(BEGINNING)?;

        Ok(Box::new(TempFileTee { inner: io::BufReader::new(tmp) }))
    }
}

const BEGINNING: io::SeekFrom = io::SeekFrom::Start(0);
const END: io::SeekFrom = io::SeekFrom::End(0);

impl Tee for TempFileTee {
    fn reset(&mut self) -> Result<()> {
        self.inner.seek(BEGINNING).map(|_| ()).chain_err(
            || "resetting TempFileTee",
        )
    }

    fn len_and_reset(&mut self) -> Result<u64> {
        let len = self.inner.seek(END)?;
        self.reset()?;
        Ok(len)
    }

    fn as_seekable(&mut self) -> Result<&mut Seeker> {
        Ok(&mut self.inner)
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
        BufReaderTee { inner: Box::new(io::BufReader::new(from)) }
    }
}

impl<R: io::Read> Tee for BufReaderTee<R>
where
    R: io::Seek + 'static,
{
    fn reset(&mut self) -> Result<()> {
        self.inner
            .seek(io::SeekFrom::Start(0))
            .map(|_| ())
            .chain_err(|| "resetting BufReaderTee")
    }

    fn len_and_reset(&mut self) -> Result<u64> {
        let len = self.inner.seek(END)?;
        self.reset()?;
        Ok(len)
    }

    fn as_seekable(&mut self) -> Result<&mut Seeker> {
        Ok(&mut *self.inner)
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
    temp: Option<io::BufReader<fs::File>>,
}

impl<U: io::Read> FailingTee<io::BufReader<U>> {
    pub fn new(from: U) -> Self {
        FailingTee {
            inner: Box::new(io::BufReader::new(from)),
            temp: None,
        }
    }
}

impl<T> Tee for FailingTee<T>
where
    T: io::BufRead,
{
    fn reset(&mut self) -> Result<()> {
        bail!(ErrorKind::UnsupportedFeature(
            "resetting a failing tee".to_string(),
        ))
    }

    fn len_and_reset(&mut self) -> Result<u64> {
        bail!(ErrorKind::UnsupportedFeature(
            "len-resetting a failing tee".to_string(),
        ))
    }

    fn as_seekable(&mut self) -> Result<&mut Seeker> {
        let mut temp = tempfile()?;
        {
            let mut fd = io::BufWriter::new(&mut temp);
            io::copy(self, &mut fd)?;
        }

        temp.seek(io::SeekFrom::Start(0))?;

        let reader = io::BufReader::new(temp);
        self.temp = Some(reader);
        Ok(self.temp.as_mut().unwrap())
    }
}

impl<T> io::Read for FailingTee<T>
where
    T: io::Read,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<T> io::BufRead for FailingTee<T>
where
    T: io::BufRead,
{
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}

pub trait Seeker: io::Seek + io::Read {}

impl<R: io::Read> Seeker for io::BufReader<R>
where
    R: io::Seek,
{
}

pub struct BoxReader<'a, R: io::Read + 'a> {
    pub inner: &'a mut R,
}

impl<'a, R: io::Read> io::Read for BoxReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<'a, R> io::BufRead for BoxReader<'a, R>
where
    R: io::BufRead,
{
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt)
    }
}



#[cfg(test)]
mod tests {
    use tee;

    use std::cmp::min;
    use std::io;

    struct Readie {
        limit: usize,
        len: usize,
    }

    impl io::Read for Readie {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let to_take = min(buf.len(), min(self.limit, self.len));

            for i in 0..to_take {
                buf[i] = (i + 1) as u8;
            }

            self.len -= to_take;
            Ok(to_take)
        }
    }

    #[test]
    fn repeated_read_short() {
        let mut r = Readie { limit: 1, len: 5 };
        let mut a = [0u8; 8];
        assert_eq!(5, tee::read_all(&mut r, &mut a).expect("read"));
        assert_eq!([1, 1, 1, 1, 1, 0, 0, 0], a);
    }


    #[test]
    fn repeated_read_over() {
        let mut r = Readie { limit: 2, len: 12 };
        let mut a = [0u8; 8];
        assert_eq!(8, tee::read_all(&mut r, &mut a).expect("read"));
        assert_eq!([1, 2, 1, 2, 1, 2, 1, 2], a);
    }

    #[test]
    fn repeated_read_whole() {
        let mut r = Readie { limit: 12, len: 8 };
        let mut a = [0u8; 8];
        assert_eq!(8, tee::read_all(&mut r, &mut a).expect("read"));
        assert_eq!([1, 2, 3, 4, 5, 6, 7, 8], a);
    }


}
