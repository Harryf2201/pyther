#!/usr/bin/env bash
# Claude's ARM handoff. Never run against deployment target directories.
set -euo pipefail
cd "$(dirname "$0")/.."
HANDOFF=$PWD
: "${REPO:?Set REPO to the existing local engine git clone}"
: "${PAWNSTORM_LAB:?Set PAWNSTORM_LAB to the existing testing directory}"
: "${ARM_LAB:?Set ARM_LAB to a NEW directory for these worktrees}"
[[ $(uname -m) == aarch64 ]] || { echo 'This script is for the ARM server'; exit 1; }
test -r "$PAWNSTORM_LAB/engines/x25-big7.nnue"
test -r "$PAWNSTORM_LAB/engines/x25-big7.options"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 RUST_TEST_THREADS=1
export BENCH_CPU=${BENCH_CPU:-$(python3 -c 'import os; print(min(os.sched_getaffinity(0)))')}
export ARMSPEED_BUILD_AFFINITY=1
names=(base kit tail pair dot final)
refs=(4f58f61 4ec7bf4 15e9bc1 768fa45 893844f c900ff0)
health() {
  while [[ -r "$PAWNSTORM_LAB/health/control.txt" ]] &&
        grep -qi pause "$PAWNSTORM_LAB/health/control.txt"; do sleep 2; done
}
case ${1:-build} in
build)
  test ! -e "$ARM_LAB" || { echo 'ARM_LAB already exists; choose a new directory'; exit 1; }
  mkdir -p "$ARM_LAB"
  for i in "${!names[@]}"; do
    name=${names[$i]}; ref=${refs[$i]}
    health
    git -C "$REPO" worktree add --detach "$ARM_LAB/$name" "$ref"
    for kind in normal frozen; do
      features=(); [[ $kind != frozen ]] || features=(--features frozen)
      python3 validation/armspeed.py build "arm-$name-$kind" --source "$ARM_LAB/$name" --test "${features[@]}"
      if [[ $name != base ]]; then
        python3 validation/armspeed.py check "arm-$name-$kind" "arm-base-$kind"
      fi
    done
  done
  # Counted builds have their own target dirs and are excluded from A/B timings.
  for name in kit tail pair dot final; do
    python3 validation/armspeed.py build "arm-$name-counted" --source "$ARM_LAB/$name" --features frozen,profile-counters --test
    python3 validation/armspeed.py check "arm-$name-counted" arm-base-frozen
  done
  ;;
profile)
  # profile args: iterations depth net options. Hash=16, Threads=1, fresh TT/history/FEN.
  for name in kit tail pair dot final; do
    for kind in normal frozen counted; do
      python3 validation/armspeed.py profile "arm-$name-$kind" --iterations 100000 --depth 15
    done
  done
  ;;
speed)
  for kind in normal frozen; do
    for i in 1 2 3 4 5; do
      prev=${names[$((i-1))]}; name=${names[$i]}
      python3 validation/armspeed.py speed "arm-$name-$kind" "arm-$prev-$kind" --pairs 6 --depth 20
    done
    python3 validation/armspeed.py speed "arm-final-total-$kind" "arm-base-$kind" \
      --exe "$HANDOFF/validation/as-arm-final-$kind.engine" --pairs 6 --depth 20
  done
  ;;
ab)
  # This existing helper fetches origin. Claude must publish the branch first.
  test -x "$HOME/arm-ab.sh"
  for kind in normal frozen; do
    features=''; [[ $kind != frozen ]] || features=frozen
    for i in 1 2 3 4 5; do
      prev=${names[$((i-1))]}; name=${names[$i]}
      python3 validation/armspeed.py command "armab-$prev-$name-$kind" \
        "$HOME/arm-ab.sh" "as-$prev-$kind" "${refs[$((i-1))]}" "$features" \
        "as-$name-$kind" "${refs[$i]}" "$features" 6 20
    done
    python3 validation/armspeed.py command "armab-base-final-$kind" \
      "$HOME/arm-ab.sh" "as-base-total-$kind" 4f58f61 "$features" \
      "as-final-total-$kind" c900ff0 "$features" 6 20
  done
  ;;
*) echo 'Usage: bash validation/armspeed-arm.sh build|profile|speed|ab'; exit 1 ;;
esac
