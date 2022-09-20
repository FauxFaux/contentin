use std::fmt;
use std::fs;
use std::io;
use std::path;

use std::collections::HashMap;

use ext4;
use gzip;
use output_capnp;

use errors::*;
use simple_time::*;
use tee::*;

use Options;

use ci_capnp;
use ci_capnp::ItemType;
use ci_capnp::Meta;

use filetype::FileType;

use slist::SList;

use std::io::Seek;
use std::io::Write;

pub struct Unpacker<'a> {
    options: &'a Options,
    current: ::EntryBuilder,
}

impl<'a> Unpacker<'a> {
    // TODO: shouldn't be pub
    pub fn log<T: fmt::Display, F>(&self, level: u8, msg: F) -> Result<()>
    where
        F: FnOnce() -> T,
    {
        if self.options.verbose < level {
            return Ok(());
        }

        let name = match level {
            0 => "error",
            1 => "warn",
            2 => "info",
            3 => "debug",
            _ => unreachable!(),
        };

        writeln!(io::stderr(), "{}: {}", name, msg()).map(|_| ())?;
        Ok(())
    }

    fn complete(&self, mut file: Box<Tee>) -> Result<()> {
        let size = file.len_and_reset()?;
        self.complete_details(file, size)
    }

    fn complete_details<R: io::Read>(&self, mut src: R, size: u64) -> Result<()> {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        output_capnp::write_capnp(
            &mut stdout,
            &self.current,
            self.options.content_output,
            size,
        )?;

        if self.options.content_output {
            let written = io::copy(&mut src, &mut stdout)?;
            if written != size {
                bail!(format!("expecting to write {} but wrote {}", size, written));
            }
        }
        Ok(())
    }

    fn from_file<'b>(path: &str, meta: fs::Metadata, options: &'b Options) -> Result<Unpacker<'b>> {
        use stat::Stat;
        use users;

        let stat: Stat = Stat::from(&meta);

        let item_type = if meta.is_dir() {
            unreachable!()
        } else if meta.file_type().is_symlink() {
            match fs::read_link(path)?.to_str() {
                Some(dest) => ItemType::SymbolicLink(dest.to_string()),
                None => bail!(ErrorKind::UnsupportedFeature(format!(
                    "{} is a symlink to an invaild utf-8 sequence",
                    path
                ))),
            }
        } else if meta.is_file() {
            ItemType::RegularFile
        } else {
            if 0 == stat.mode {
                // "mode" will be zero on platforms that don't do modes (i.e. Windows).
                // TODO: detect symlinks (and DEVices) on Windows?
                ItemType::Unknown
            } else {
                // TODO: use libc here, or assume the constants are the same everywhere?
                let mode_type = (stat.mode >> 12) & 0b1111;
                match mode_type {
                    0b0100 => unreachable!(), // S_IFDIR
                    0b1000 => unreachable!(), // S_IFREG
                    0b1010 => unreachable!(), // S_IFLNK

                    0b0001 => ItemType::Fifo,   // S_IFIFO
                    0b1100 => ItemType::Socket, // S_IFSOCK

                    0b0010 => panic!("TODO: char device"), // S_IFCHR
                    0b0110 => panic!("TODO: block device"), // S_IFBLK

                    _ => bail!(ErrorKind::UnsupportedFeature(format!(
                        "unrecognised unix mode type {:b}",
                        mode_type
                    ),)),
                }
            }
        };

        let meta = Meta {
            atime: meta.accessed().map(simple_time_sys)?,
            mtime: meta.modified().map(simple_time_sys)?,
            ctime: simple_time_ctime(&stat),
            btime: simple_time_btime(&meta)?,
            ownership: ci_capnp::Ownership::Posix {
                user: Some(ci_capnp::PosixEntity {
                    id: u64::from(stat.uid),
                    name: users::get_user_by_uid(stat.uid)
                        .map(|user| user.name().to_string())
                        .unwrap_or(String::new()),
                }),
                group: Some(ci_capnp::PosixEntity {
                    id: u64::from(stat.gid),
                    name: users::get_group_by_gid(stat.gid)
                        .map(|group| group.name().to_string())
                        .unwrap_or(String::new()),
                }),
                mode: stat.mode,
            },
            container: ci_capnp::Container::Unrecognised,
            item_type,
            // TODO: extract xattrs?
            xattrs: HashMap::new(),
        };

        Ok(Unpacker {
            options,
            current: ::EntryBuilder {
                depth: 0,
                path: SList::head(path.to_string()),
                meta,
                failure: None,
            },
        })
    }

    fn with_path(&self, path: &str) -> Unpacker {
        let meta = Meta {
            atime: 0,
            mtime: 0,
            ctime: 0,
            btime: 0,
            ownership: ci_capnp::Ownership::Unknown,
            item_type: ItemType::Unknown,
            container: ci_capnp::Container::Unrecognised,
            xattrs: HashMap::new(),
        };

        Unpacker {
            options: self.options,
            current: ::EntryBuilder {
                path: self.current.path.plus(path.to_string()),
                depth: self.current.depth + 1,
                meta,
                failure: None,
            },
        }
    }

    fn strip_compression_suffix(&self, suffix: &str) -> &str {
        let our_name = self.current.path.inner().as_str();
        if our_name.ends_with(suffix) {
            &our_name[..our_name.len() - suffix.len()]
        } else {
            ""
        }
    }

    fn process_zip<T>(&self, from: T) -> Result<()>
    where
        T: io::Read + io::Seek,
    {
        use zip;
        let mut zip = zip::ZipArchive::new(from).chain_err(|| "opening zip")?;

        for i in 0..zip.len() {
            let unpacker = {
                let entry: zip::read::ZipFile = zip
                    .by_index(i)
                    .chain_err(|| format!("opening entry {}", i))?;
                let mut unpacker = self.with_path(entry.name());

                unpacker.current.meta.mtime = simple_time_tm(entry.last_modified());
                unpacker.current.meta.ownership = match entry.unix_mode() {
                    Some(mode) => ci_capnp::Ownership::Posix {
                        user: None,
                        group: None,
                        mode,
                    },
                    None => ci_capnp::Ownership::Unknown,
                };

                unpacker
            };

            let res = {
                let entry = zip.by_index(i)?;
                let mut failing: Box<Tee> = Box::new(FailingTee::new(entry));
                unpacker.unpack_or_die(&mut failing)
            };

            if self.is_format_error_result(&res)? {
                let new_entry = zip.by_index(i)?;
                let size = new_entry.size();
                unpacker
                    .complete_details(new_entry, size)
                    .chain_err(|| "..after rollback")?;
                continue;
            }

            if res.is_err() {
                return res;
            }
        }
        Ok(())
    }

    fn process_partition<T>(&self, inner: T) -> Result<()>
    where
        T: io::Read + io::Seek,
    {
        let mut settings = ext4::Options::default();
        settings.checksums = ext4::Checksums::Enabled;
        let mut fs = ext4::SuperBlock::new_with_options(inner, &settings)
            .chain_err(|| "opening filesystem")?;
        let root = &fs.root().chain_err(|| "loading root")?;
        fs.walk(root, "".to_string(), &mut |fs, path, inode, enhanced| {
            if let Err(e) = self.process_regular_inode(fs, inode, enhanced, path) {
                return Err(ext4::Error::with_chain(e, "reading file"));
            }
            Ok(true)
        })?;

        Ok(())
    }

    fn process_regular_inode<T>(
        &self,
        fs: &mut ext4::SuperBlock<T>,
        inode: &ext4::Inode,
        enhanced: &ext4::Enhanced,
        path: &str,
    ) -> Result<()>
    where
        T: io::Read + io::Seek,
    {
        let mut unpacker = self.with_path(path);
        {
            let current = &mut unpacker.current;
            let stat: &ext4::Stat = &inode.stat;
            current.meta.ownership = ci_capnp::Ownership::Posix {
                user: Some(ci_capnp::PosixEntity {
                    id: u64::from(stat.uid),
                    name: String::new(),
                }),
                group: Some(ci_capnp::PosixEntity {
                    id: u64::from(stat.gid),
                    name: String::new(),
                }),
                mode: stat.file_mode as u32,
            };

            current.meta.atime = simple_time_ext4(&stat.atime);
            current.meta.mtime = simple_time_ext4(&stat.mtime);
            current.meta.ctime = simple_time_ext4(&stat.ctime);
            current.meta.btime = match stat.btime.as_ref() {
                Some(btime) => simple_time_ext4(btime),
                None => 0,
            };

            current.meta.item_type = match *enhanced {
                ext4::Enhanced::RegularFile => ItemType::RegularFile,
                ext4::Enhanced::Directory(_) => ItemType::Directory,
                ext4::Enhanced::Socket => ItemType::Socket,
                ext4::Enhanced::Fifo => ItemType::Fifo,
                ext4::Enhanced::SymbolicLink(ref dest) => ItemType::SymbolicLink(dest.to_string()),
                ext4::Enhanced::CharacterDevice(major, minor) => ItemType::CharacterDevice {
                    major: major as u32,
                    minor,
                },
                ext4::Enhanced::BlockDevice(major, minor) => ItemType::BlockDevice {
                    major: major as u32,
                    minor,
                },
            };

            current.meta.xattrs = inode.stat.xattrs.clone();
        }

        match unpacker.current.meta.item_type {
            ItemType::RegularFile => {
                // TODO: this should be a BufReaderTee, but BORROWS. HORRIBLE INEFFICIENCY
                let tee = TempFileTee::if_necessary(fs.open(inode)?, &unpacker)
                    .map_err(|e| ext4::Error::with_chain(e, "tee"))?;
                unpacker
                    .unpack(tee)
                    .map_err(|e| ext4::Error::with_chain(e, "unpacking"))?;
            }
            _ => {
                unpacker.complete_details(io::Cursor::new(&[]), 0)?;
            }
        };

        Ok(())
    }

    fn process_tar<'c>(&self, fd: &mut Box<Tee + 'c>) -> Result<()> {
        use tar;
        let mut decoder = tar::Archive::new(fd);
        for entry in decoder.entries()? {
            let entry = entry.map_err(tar_err).chain_err(|| "parsing header")?;

            let mut unpacker = {
                let path = entry.path().map_err(tar_err)?;
                let path = path.to_str().ok_or_else(|| {
                    ErrorKind::UnsupportedFeature(format!(
                        "invalid path utf-8: {:?}",
                        entry.path_bytes()
                    ))
                })?;

                if "pax_global_header" == path {
                    // TODO: maybe expose this data?
                    // Just ignoring the entry is more consistent with "tar xf".
                    continue;
                }

                self.with_path(path)
            };

            {
                let current = &mut unpacker.current;
                let header = entry.header();

                current.meta.ownership = ci_capnp::Ownership::Posix {
                    user: Some(ci_capnp::PosixEntity {
                        id: header.uid().map_err(tar_err).chain_err(|| "reading uid")?,
                        name: header
                            .username()
                            .and_then(|x| Ok(x.unwrap_or("")))
                            .map_err(|e| {
                                ErrorKind::UnsupportedFeature(format!(
                                    "invalid username utf-8: {} {:?}",
                                    e,
                                    header.username_bytes()
                                ))
                            })?
                            .to_string(),
                    }),
                    group: Some(ci_capnp::PosixEntity {
                        id: header.gid().map_err(tar_err).chain_err(|| "reading gid")?,
                        name: header
                            .groupname()
                            .and_then(|x| Ok(x.unwrap_or("")))
                            .map_err(|e| {
                                ErrorKind::UnsupportedFeature(format!(
                                    "invalid groupname utf-8: {} {:?}",
                                    e,
                                    header.username_bytes()
                                ))
                            })?
                            .to_string(),
                    }),
                    mode: header.mode()?,
                };

                current.meta.mtime = simple_time_epoch_seconds(
                    header
                        .mtime()
                        .map_err(tar_err)
                        .chain_err(|| "reading mtime")?,
                );
            }

            unpacker
                .unpack(TempFileTee::if_necessary(entry, &unpacker)?)
                .chain_err(|| format!("processing tar entry: {}", unpacker.current.path.inner()))?;
        }
        Ok(())
    }

    fn with_gzip(&self, header: &gzip::Header) -> Result<Unpacker> {
        let mtime = simple_time_epoch_seconds(header.modification_time() as u64);
        let name = match header.filename() {
            Some(c_str) => c_str.to_str().map_err(|not_utf8| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "gzip member's name must be valid utf-8: {} {:?}",
                        not_utf8,
                        c_str.as_bytes()
                    ),
                )
            })?,
            None => self.strip_compression_suffix(".gz"),
        };

        let mut unpacker = self.with_path(name);
        unpacker.current.meta.mtime = mtime;
        Ok(unpacker)
    }

    fn unpack_or_die<'b>(&self, fd: &mut Box<Tee + 'b>) -> Result<()> {
        if self.current.depth >= self.options.max_depth {
            bail!(ErrorKind::Rewind);
        }

        let identity = FileType::identify(fd.fill_buf()?);
        self.log(2, || {
            format!("identified '{}' as {}", self.current.path.inner(), identity)
        })?;
        match identity {
            FileType::GZip => {
                let (attempt, unpacker) = {
                    let br = BoxReader { inner: fd };
                    let dec = gzip::Decoder::new(br)?;

                    let unpacker = self.with_gzip(dec.header())?;

                    let mut failing: Box<Tee> = Box::new(FailingTee::new(dec));
                    (
                        unpacker
                            .unpack_or_die(&mut failing)
                            .chain_err(|| "streaming gzip"),
                        unpacker,
                    )
                };

                if self.is_format_error_result(&attempt)? {
                    fd.reset()?;
                    unpacker.complete(TempFileTee::if_necessary(
                        gzip::Decoder::new(fd)?,
                        &unpacker,
                    )?)?;
                    Ok(())
                } else {
                    attempt
                }
            }

            // xz and bzip2 have *nothing* in their header; no mtime, no name, no source OS, no nothing.
            FileType::Xz => self
                .with_path(self.strip_compression_suffix(".xz"))
                .unpack_stream_xz(fd)
                .chain_err(|| "unpacking xz"),
            FileType::BZip2 => self
                .with_path(self.strip_compression_suffix(".bz2"))
                .unpack_stream_bz2(fd)
                .chain_err(|| "unpacking bz2"),

            FileType::Deb => {
                use ar;
                let mut decoder = ar::Archive::new(fd);
                while let Some(entry) = decoder.next_entry() {
                    let entry = entry?;
                    let unpacker = self.with_path(entry.header().identifier());
                    unpacker
                        .unpack(TempFileTee::if_necessary(entry, &unpacker)?)
                        .chain_err(|| format!("unpacking deb entry {}", unpacker.current.path))?;
                }
                Ok(())
            }
            FileType::Tar => self.process_tar(fd).chain_err(|| "unpacking tar"),
            FileType::Zip => self
                .process_zip(fd.as_seekable()?)
                .chain_err(|| "reading zip file"),
            FileType::Other => Err(ErrorKind::Rewind.into()),
            FileType::DiskImage => {
                use bootsector;
                let mut fd = fd.as_seekable()?;
                for partition in
                    bootsector::list_partitions(&mut fd, &bootsector::Options::default())?
                {
                    let unpacker = self.with_path(format!("p{}", partition.id).as_str());
                    let mut part_reader = bootsector::open_partition(&mut fd, &partition)?;

                    let attempt = {
                        let mut failing: Box<Tee> = Box::new(FailingTee::new(&mut part_reader));
                        unpacker.unpack_or_die(&mut failing)
                    };

                    if self.is_format_error_result(&attempt)? {
                        part_reader.seek(io::SeekFrom::Start(0))?;
                        unpacker.complete_details(part_reader, partition.len)?;
                    } else {
                        attempt?;
                    }
                }
                Ok(())
            }
            FileType::Ext4 => self.process_partition(fd.as_seekable()?),
        }
    }

    // TODO: Work out how to generic these copy-pastes
    fn unpack_stream_xz<'c>(&self, fd: &mut Box<Tee + 'c>) -> Result<()> {
        use xz2;
        let attempt = {
            let br = BoxReader { inner: fd };
            let mut failing: Box<Tee> = Box::new(FailingTee::new(xz2::bufread::XzDecoder::new(br)));
            self.unpack_or_die(&mut failing)
        };

        if self.is_format_error_result(&attempt)? {
            fd.reset()?;
            self.complete(TempFileTee::if_necessary(
                xz2::bufread::XzDecoder::new(fd),
                self,
            )?)?;
            Ok(())
        } else {
            attempt
        }
    }

    // TODO: copy-paste of unpack_stream_xz
    fn unpack_stream_bz2<'c>(&self, fd: &mut Box<Tee + 'c>) -> Result<()> {
        use bzip2;
        let attempt = {
            let br = BoxReader { inner: fd };
            let mut failing: Box<Tee> = Box::new(FailingTee::new(bzip2::read::BzDecoder::new(br)));
            self.unpack_or_die(&mut failing)
        };

        if self.is_format_error_result(&attempt)? {
            fd.reset()?;
            self.complete(TempFileTee::if_necessary(
                bzip2::read::BzDecoder::new(fd),
                self,
            )?)?;
            Ok(())
        } else {
            attempt
        }
    }

    fn is_format_error_result<T>(&self, res: &Result<T>) -> Result<bool> {
        let problem = classify_format_error_result(res);

        if let Some(specific) = problem {
            match specific {
                FormatErrorType::Other => {
                    let error = res.as_ref().err().unwrap();
                    self.log(1, || {
                        format!(
                            "thought we could unpack '{}' but we couldn't: {:?} {}",
                            self.current.path, error, error
                        )
                    })?;
                }
                FormatErrorType::Rewind => {}
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn unpack(&self, mut fd: Box<Tee>) -> Result<()> {
        let res = self.unpack_or_die(&mut fd).chain_err(|| "unpacking failed");

        if self.is_format_error_result(&res)? {
            self.complete(fd)?;
            return Ok(());
        }

        res
    }
}

pub fn process_real_path<P: AsRef<path::Path>>(path: P, options: &Options) -> Result<()> {
    let path = path.as_ref();

    if !path.is_dir() {
        let metadata = fs::symlink_metadata(path)?;

        let unpacker = Unpacker::from_file(
            path.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("non-utf-8 filename found: {:?}", path),
                )
            })?,
            metadata,
            options,
        )?;

        return match unpacker.current.meta.item_type {
            ItemType::Directory => unreachable!(),

            ItemType::SymbolicLink(_)
            | ItemType::HardLink(_)
            | ItemType::CharacterDevice { .. }
            | ItemType::BlockDevice { .. }
            | ItemType::Fifo
            | ItemType::Socket => {
                // can't actually read from these guys
                unpacker.complete_details(io::Cursor::new(&[]), 0)
            }

            ItemType::Unknown | ItemType::RegularFile => {
                let file = fs::File::open(path)?;
                unpacker.unpack(Box::new(BufReaderTee::new(file)))
            }
        };
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        process_real_path(path, options)?;
    }
    Ok(())
}

fn tar_err(e: io::Error) -> Error {
    if io::ErrorKind::Other != e.kind() {
        return Error::with_chain(e, "reading tar");
    }

    ErrorKind::Tar(format!("{}", e)).into()
}
