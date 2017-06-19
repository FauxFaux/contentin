#!/bin/dash
set -eu

T=$(mktemp --suffix=.ci.log)

if ci-gen -h capnp $1 >/dev/null 2>${T}; then
    rm ${T}
else
    echo $1 ${T} >> failure.lst
fi
