#!/bin/sh
set -eu

O=$(readlink -f ../tests/examples/)

T=$(mktemp -d)
(
    cd "$T"
    mkdir -p a/b/c
    printf 123456789 > foo
    printf 123456789 > a/bar

    rm -f "${O}/simple.zip"
    zip -r "${O}/simple.zip" *
    tar cf "${O}/simple.tar" *
    tar zcf "${O}/simple.tar.gz" *
    tar jcf "${O}/simple.tar.bz2" *
    tar Jcf "${O}/simple.tar.xz" *
)

# dd if=/dev/urandom of=crap bs=1 count=3k
# dd if=/dev/urandom of=crap2 bs=1 count=3k

rm -f "${O}/byte_flip.zip"
zip -r "${O}/byte_flip.zip" crap crap2
tar cf "${O}/byte_flip.tar" crap crap2
tar zcf "${O}/byte_flip.tar.gz" crap crap2
tar jcf "${O}/byte_flip.tar.bz2" crap crap2
tar Jcf "${O}/byte_flip.tar.xz" crap crap2

strip-nondeterminism "${O}"/*

dd if=/dev/zero bs=1 conv=notrunc count=2 seek=5000 of="${O}/byte_flip.zip"
dd if=/dev/zero bs=1 conv=notrunc count=2 seek=5000 of="${O}/byte_flip.tar"
dd if=/dev/zero bs=1 conv=notrunc count=2 seek=5000 of="${O}/byte_flip.tar.gz"
dd if=/dev/zero bs=1 conv=notrunc count=2 seek=5000 of="${O}/byte_flip.tar.bz2"
dd if=/dev/zero bs=1 conv=notrunc count=2 seek=5000 of="${O}/byte_flip.tar.xz"

rm -rf "$T"
