use std;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileType {
    GZip,
    Zip,
    Tar,
    BZip2,
    Xz,
    Deb,
    DiskImage,
    Ext4,
    Other,
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

fn read_octal(bytes: &[u8]) -> Option<u32> {
    let mut start = 0;
    while start < bytes.len() && b' ' == bytes[start] {
        start += 1;
    }

    let mut end = bytes.len() - 1;
    while end > start && (b' ' == bytes[end] || 0 == bytes[end]) {
        end -= 1;
    }

    if let Ok(string) = std::str::from_utf8(&bytes[start..(end + 1)]) {
        if let Ok(val) = u32::from_str_radix(string, 8) {
            return Some(val);
        }
    }
    None
}

fn is_probably_tar(header: &[u8]) -> bool {
    if header.len() < 512 {
        return false;
    }

    if let Some(checksum) = read_octal(&header[148..156]) {
        let mut sum: u32 = (b' ' as u32) * 8;
        for i in 0..148 {
            sum += header[i] as u32;
        }
        for i in 156..512 {
            sum += header[i] as u32;
        }

        if checksum == sum {
            return true;
        }
    }

    return false;
}

const DEB_PREFIX: &[u8] = b"!<arch>\ndebian-binary ";

impl FileType {
    #[cfg_attr(rustfmt, rustfmt_skip)]
    pub fn identify<'a>(header: &[u8]) -> FileType {
        if header.len() >= 20
            && 0x1f == header[0] && 0x8b == header[1] {
            FileType::GZip
        } else if header.len() >= 152
            && b'P' == header[0] && b'K' == header[1]
            && 0x03 == header[2] && 0x04 == header[3] {
            FileType::Zip
        } else if header.len() > 257 + 10
            && b'u' == header[257] && b's' == header[258]
            && b't' == header[259] && b'a' == header[260]
            && b'r' == header[261]
            && (
            (0 == header[262] && b'0' == header[263] && b'0' == header[264]) ||
                (b' ' == header[262] && b' ' == header[263] && 0 == header[264])
        ) {
            FileType::Tar
        } else if header.len() > 70
            && header[0..DEB_PREFIX.len()] == DEB_PREFIX[..]
            && header[66..70] == b"`\n2."[..] {
            FileType::Deb
        } else if header.len() > 40
            && b'B' == header[0] && b'Z' == header[1]
            && b'h' == header[2] // [3]: compression level
            && 0x31 == header[4] && 0x41 == header[5]
            && 0x59 == header[6] && 0x26 == header[7]
            && 0x53 == header[8] && 0x59 == header[9] {
            FileType::BZip2
        } else if header.len() > 6
            && 0xfd == header[0] && b'7' == header[1]
            && b'z' == header[2] && b'X' == header[3]
            && b'Z' == header[4] && 0 == header[5] {
            FileType::Xz
        } else if is_probably_tar(header) {
            FileType::Tar
        } else if header.len() > 512
            && 0x55 == header[510] && 0xaa == header[511] {
            FileType::DiskImage
        } else if header.len() > 2048
            && 0x53 == header[0x438] && 0xef == header[0x439] {
            FileType::Ext4
        } else {
            FileType::Other
        }
    }
}
