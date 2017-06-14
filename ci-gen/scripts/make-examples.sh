#!/bin/sh
set -eu

O=$(readlink -f ../tests/)

T=$(mktemp -d)
(
    cd "$T"
    mkdir -p a/b/c
    printf 123456789 > foo
    printf 123456789 > a/bar

    zip -r "${O}/simple.zip" .
    tar cf "${O}/simple.tar" .
    tar zcf "${O}/simple.tar.gz" .
    tar jcf "${O}/simple.tar.bz2" .
    tar Jcf "${O}/simple.tar.xz" .
)

strip-nondeterminism "${O}"/*

rm -rf "$T"
