#!/bin/bash
# SilverJS's A/B of the micro-benches (the lead's method, 2026-11-23): a
# base revision (A, built from a temporary worktree) against the working
# tree (B), each built into its own copy of the qjs-bench harness with
# the harness's `bench-ship` profile (QuickJS-ng's C at /O2),
# then run alternately A B A B. Run it whenever a step moves a bench
# line by more than 10%: placement alone moves the default build's
# ratios by that much, so only an A/B of the same build tells.
#
#   bash tools/abbench.sh <harness dir> <bench dir> [base revision] [rounds]
#
# <harness dir> is the qjs-bench crate (its Cargo.toml names SilverJS by
# a relative path, rewritten here for each copy); <bench dir> holds the
# bench scripts (*.js). The base defaults to HEAD, the rounds to 2. With
# QUIET_SH set to a quiet-machine script, the whole run holds its lock.
set -u
HARNESS=$(cd "$1" && pwd) || exit 1
BENCH=$(cd "$2" && pwd) || exit 1
BASE=${3:-HEAD}
ROUNDS=${4:-2}
REPO=$(cd "$(dirname "$0")/.." && pwd)
if [ -n "${QUIET_SH:-}" ] && [ -z "${QUIET_HELD:-}" ]; then
  exec bash "$QUIET_SH" run "silverjs abbench ($$)" -- bash "$0" "$@"
fi
WORK=$(mktemp -d)
# A directory as cargo reads it from a manifest: Windows form under Git
# Bash (pwd -W), as it is elsewhere.
native() { (cd "$1" && (pwd -W 2>/dev/null || pwd)); }
trap 'git -C "$REPO" worktree remove --force "$WORK/head" 2>/dev/null; rm -rf "$WORK"' EXIT
git -C "$REPO" worktree add --detach "$WORK/head" "$BASE" -q || exit 1
# QuickJS-ng's C is built at /O2 (cc's mapping of opt-level 3 under
# MSVC), without LTO: rustc cannot link /GL objects bundled in an rlib.
for v in a b; do
  cp -r "$HARNESS" "$WORK/harness-$v"
  src=$(native "$REPO")
  [ "$v" = a ] && src=$(native "$WORK/head")
  sed -i "s#^silverjs = { path = \"[^\"]*\" }#silverjs = { path = \"$src\" }#" \
    "$WORK/harness-$v/Cargo.toml"
  (cd "$WORK/harness-$v" && CARGO_TARGET_DIR="$WORK/target-$v" \
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} \
    cargo build --profile bench-ship --offline -q) || exit 1
done
echo "A: $(git -C "$REPO" rev-parse --short "$BASE")  B: the working tree"
for round in $(seq 1 "$ROUNDS"); do
  for v in a b; do
    echo "== $v round $round"
    "$WORK/target-$v/bench-ship/qjs-bench" run "$BENCH"/*.js
  done
done
