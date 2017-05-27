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