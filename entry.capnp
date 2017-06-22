@0xb3afa6ab952b49de;

struct Entry {
    magic @0 :UInt32;

    # the number of bytes of file content; typically following this header
    len @1 :UInt64;

    # the name of this file (paths[0]), and the things it is in.
    # Read as: main.c (0) is in code.tar (1) is in code.tar.gz (2) is in ...
    paths @2 :List(Text);

    # the access, modification, change and birth times of the file,
    # in nanoseconds since the UNIX epoch
    atime @3 :UInt64;
    mtime @4 :UInt64;
    ctime @5 :UInt64;
    btime @6 :UInt64;

    ownership :union {
        unknown   @7 :Void;

        posix :group {
            user  @8 :PosixEntity;
            group @9 :PosixEntity;
            mode  @10 :UInt32;
        }
    }

    type :union {
        normal      @11 :Void;
        directory   @12 :Void;
        fifo        @13 :Void;
        softLinkTo  @14 :Text;
        hardLinkTo  @15 :Text;
        charDevice  @16 :DeviceNumbers;
        blockDevice @17 :DeviceNumbers;
    }

    content :union {
        absent  @18 :Void;
        follows @19 :Void;
    }

    container :union {
        # we didn't expect to be able to unpack this,
        # we couldn't identify it as any supported type of archive
        unrecognised @20 :Void;

        # we successfully unpacked this, but included it anyway
        # in the stream, the entries for it will probably appear elsewhere too
        included     @21 :Void;

        # we expected to open this, but failed to parse its header or metadata;
        # so it's included as if it's unrecognised, and no entries were read from it.
        # e.g. a zip file with a corrupt central directory (~= header)
        openError    @22 :Text;

        # we opened this in a way that we thought was successful, but, after
        # reading it, we discovered there was a problem. Some or all of the entries
        # may be present in the stream, and they may individually be corrupt.
        # e.g. gzip with a checksum at the end, we detect the failure after emitting
        # all of the entries, which may be corrupt.
        readError    @23 :Text;
    }

    # Intended to carry filesystem xattrs, like acls and capabilities,
    # but I'm not going to stop you ramming crap in here.
    # For the filesystem, 'name' should be of the format namespace.name, or namespace.name.sub_name
    # The namespaces "user.", "system.", "trusted.", and "security." are defined in man:attr(5).
    # Note: Real (2017) filesystems support only around 5kb of attributes total,
    # including their names, data, and overhead.
    xattrs @24 :List(ExtendedAttribute);
}

struct PosixEntity {
    id   @0 :UInt32;
    name @1 :Text;
}

struct DeviceNumbers {
    # Even though I've never seen a device number over ~255,
    # man:mknod(2) defines these as mode_t and dev_t respectively,
    # which the current kernel has as:
    # mode_t: "unsigned short" (16-bits on all Debian platforms)
    # dev_t: "u32" (much less ambiguous)

    major @0 :UInt16;
    minor @1 :UInt32;
}

struct ExtendedAttribute {
    name  @0 :Text;
    value @1 :Data;
}
