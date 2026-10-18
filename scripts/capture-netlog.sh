#!/usr/bin/env bash
# Captures a bounded Chrome netlog while driving the viewer, for QUIC-layer
# forensics on a transport incident (stalled video, wedged connection).
#
# Three properties this enforces, each learned from a capture that lacked it:
#
#   1. A SIZE BOUND. Chrome writes a netlog until the disk fills; an
#      unbounded capture of a stream-per-frame video session reached 1.6 GB
#      in under an hour. --net-log-max-size-mb keeps it to a stated ceiling.
#   2. A CAPTURE MODE that defaults to the LEAST sensitive setting that
#      still names transport events. IncludeSensitive records cookies,
#      auth headers, and session credentials in plaintext, so it is opt-in
#      (--sensitive) and the file is written 0600 either way.
#   3. A PATH INSIDE target/, never the home directory, so captures are
#      gitignored, pruned here, and thrown away with the build tree.
#
# Chrome ignores these flags when it attaches to an already-running
# instance, so this always launches a DEDICATED instance with its own
# throwaway profile. Your normal browser session is untouched.
#
# Usage: scripts/capture-netlog.sh <viewer-url> [--sensitive] [--max-mb N]
#        Ctrl-C (or closing the window) ends the capture and flushes it.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
CAPTURE_DIR="${REPO_ROOT}/target/netlog"
# Captures kept; older ones are pruned so a forensics habit cannot fill a disk.
KEEP=3
MAX_MB=256
MODE="Default"

if [[ $# -lt 1 ]]; then
  echo "usage: scripts/capture-netlog.sh <viewer-url> [--sensitive] [--max-mb N]" >&2
  exit 2
fi
URL="$1"
shift
while [[ $# -gt 0 ]]; do
  case "$1" in
    --sensitive) MODE="IncludeSensitive"; shift ;;
    --max-mb) MAX_MB="${2:?--max-mb needs a value}"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ ! -x "${CHROME}" ]]; then
  echo "Chrome not found at ${CHROME}; set CHROME=<path> and retry." >&2
  exit 1
fi

mkdir -p "${CAPTURE_DIR}"
# Prune oldest first, leaving room for the capture about to start.
while [[ "$(find "${CAPTURE_DIR}" -maxdepth 1 -name 'netlog-*.json' | wc -l)" -ge "${KEEP}" ]]; do
  OLDEST="$(find "${CAPTURE_DIR}" -maxdepth 1 -name 'netlog-*.json' | sort | sed -n 1p)"
  [[ -n "${OLDEST}" ]] || break
  echo "pruning older capture $(basename "${OLDEST}")"
  rm -f "${OLDEST}"
done

CAPTURE="${CAPTURE_DIR}/netlog-$(date -u +%Y%m%dT%H%M%SZ).json"
PROFILE="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-netlog-profile.XXXXXX")"
# Pre-create 0600: Chrome creates it world-readable by default, and a
# sensitive capture must not be readable by other accounts on the machine.
umask 077
: > "${CAPTURE}"

if [[ "${MODE}" == "IncludeSensitive" ]]; then
  echo "WARNING: sensitive mode records cookies, auth headers, and session"
  echo "         credentials in plaintext. Treat ${CAPTURE} as a secret and"
  echo "         delete it once the incident is understood."
fi
echo "capturing to ${CAPTURE} (mode=${MODE}, max=${MAX_MB} MB)"
echo "reproduce the incident in the window that opens, then close it."

"${CHROME}" \
  --user-data-dir="${PROFILE}" \
  --no-first-run \
  --no-default-browser-check \
  --log-net-log="${CAPTURE}" \
  --net-log-capture-mode="${MODE}" \
  --net-log-max-size-mb="${MAX_MB}" \
  "${URL}" >/dev/null 2>&1 || true

rm -rf "${PROFILE}"
SIZE="$(du -h "${CAPTURE}" 2>/dev/null | cut -f1)"
echo "capture complete: ${CAPTURE} (${SIZE:-unknown})"
