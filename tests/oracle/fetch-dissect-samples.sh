#!/bin/sh
# Fetch the real Windows caches dissect.thumbcache tests with (Fox-IT,
# AGPL-3.0) into tests/fixtures-real/dissect/, which git ignores: they are
# never committed, only read by the ignored test `real_windows_caches`.
# Needs git-lfs.
set -eu
here=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
git clone --quiet https://github.com/fox-it/dissect.thumbcache.git "$work/repo" &&
git -C "$work/repo" checkout --quiet c73ae487161720794542c13b1170186a9a64d156 &&
git -C "$work/repo" lfs pull &&
mkdir -p "$here/tests/fixtures-real/dissect" &&
cp -R "$work/repo/tests/data/." "$here/tests/fixtures-real/dissect/" &&
rm -rf "$work"
