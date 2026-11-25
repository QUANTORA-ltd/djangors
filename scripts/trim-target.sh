#!/usr/bin/env bash
set -euo pipefail

# Enforces a hard disk-space cap on Cargo's `target/` directory without
# disturbing active builds. Intended for both manual invocation and a
# periodic systemd --user timer.
#
# Usage: scripts/trim-target.sh [MAX_TARGET_MB] [TARGET_DIR]

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

MAX_TARGET_MB="${1:-${MAX_TARGET_MB:-4096}}"
TARGET_DIR="${2:-${TARGET_DIR:-${REPO_ROOT}/target}}"

if [[ ! -d "${TARGET_DIR}" ]]; then
    exit 0
fi

# Resolve canonical path for accurate process matching.
TARGET_REAL="$(readlink -f "${TARGET_DIR}" 2>/dev/null || printf '%s' "${TARGET_DIR}")"

# Build-safety guard 1: never touch target/ while Cargo holds a profile lock.
for lockfile in "${TARGET_REAL}/debug/.cargo-lock" "${TARGET_REAL}/release/.cargo-lock"; do
    if [[ -e "${lockfile}" ]]; then
        if ! flock -n "${lockfile}" true 2>/dev/null; then
            exit 0
        fi
    fi
done

# Build-safety guard 2: never touch target/ while cargo/rustc/nextest/rust-analyzer
# is actively referencing it.
if command -v pgrep >/dev/null 2>&1; then
    while IFS= read -r pid; do
        [[ -z "${pid}" ]] && continue
        if [[ "${pid}" != "$$" && "${pid}" != "${PPID:-0}" ]]; then
            comm="$(ps -p "${pid}" -o comm= 2>/dev/null || true)"
            case "${comm}" in
                cargo|rustc|cargo-nextest|rustdoc|clippy-driver)
                    exit 0
                    ;;
            esac
        fi
    done < <(pgrep -f "${TARGET_REAL}" 2>/dev/null || true)
fi

dir_mb() {
    du -sm "${TARGET_REAL}" 2>/dev/null | awk '{print $1}'
}

BEFORE_MB="$(dir_mb)"
if [[ -z "${BEFORE_MB}" || "${BEFORE_MB}" -le "${MAX_TARGET_MB}" ]]; then
    exit 0
fi

# Stage 1: Ephemeral artifacts (docs, tmp, package staging, rust-analyzer flycheck).
rm -rf "${TARGET_REAL}/doc" "${TARGET_REAL}/tmp" "${TARGET_REAL}/package" "${TARGET_REAL}"/flycheck* 2>/dev/null || true
if [[ "$(dir_mb)" -le "${MAX_TARGET_MB}" ]]; then
    AFTER_MB="$(dir_mb)"
    echo "trimmed target/: ${BEFORE_MB} MiB -> ${AFTER_MB} MiB (cap: ${MAX_TARGET_MB} MiB)"
    exit 0
fi

# Stage 2: Nested test sub-targets (trybuild & cli-test-cache).
rm -rf "${TARGET_REAL}/tests/trybuild" "${TARGET_REAL}/cli-test-cache" 2>/dev/null || true
if [[ "$(dir_mb)" -le "${MAX_TARGET_MB}" ]]; then
    AFTER_MB="$(dir_mb)"
    echo "trimmed target/: ${BEFORE_MB} MiB -> ${AFTER_MB} MiB (cap: ${MAX_TARGET_MB} MiB)"
    exit 0
fi

# Stage 3: Incremental compilation caches (keep older session dirs first, then drop all).
python3 - "${TARGET_REAL}" "${MAX_TARGET_MB}" <<'PY'
import os
import re
import shutil
import sys

target = sys.argv[1]
cap_mb = int(sys.argv[2])

def total_bytes(path: str) -> int:
    total = 0
    for root, _, files in os.walk(path, followlinks=False):
        for name in files:
            fp = os.path.join(root, name)
            try:
                total += os.lstat(fp).st_size
            except OSError:
                pass
    return total

def under_cap() -> bool:
    return total_bytes(target) <= cap_mb * 1024 * 1024

# 3a: Keep only the newest incremental session dir per crate.
for profile in ("debug", "release"):
    inc_root = os.path.join(target, profile, "incremental")
    if not os.path.isdir(inc_root):
        continue
    for crate_dir in os.listdir(inc_root):
        crate_path = os.path.join(inc_root, crate_dir)
        if not os.path.isdir(crate_path):
            continue
        sessions = []
        for entry in os.listdir(crate_path):
            if entry.startswith("s-"):
                ep = os.path.join(crate_path, entry)
                try:
                    sessions.append((os.lstat(ep).st_mtime, ep))
                except OSError:
                    pass
        sessions.sort(reverse=True)
        for _, old_ep in sessions[1:]:
            shutil.rmtree(old_ep, ignore_errors=True)

if under_cap():
    sys.exit(0)

# 3b: Still over cap — drop target/*/incremental while keeping .rlib/.rmeta warm.
for profile in ("debug", "release"):
    inc_root = os.path.join(target, profile, "incremental")
    if os.path.isdir(inc_root):
        shutil.rmtree(inc_root, ignore_errors=True)

if under_cap():
    sys.exit(0)

# Stage 4: Stale duplicate hashed artifacts in deps/ (keep only newest mtime per <stem>.<ext>).
hash_re = re.compile(r"^(.+)-[0-9a-f]{16}(\.[^.]+)?$")
for profile in ("debug", "release"):
    deps_dir = os.path.join(target, profile, "deps")
    if not os.path.isdir(deps_dir):
        continue
    groups = {}
    for name in os.listdir(deps_dir):
        m = hash_re.match(name)
        if not m:
            continue
        fp = os.path.join(deps_dir, name)
        if not os.path.isfile(fp) or os.path.islink(fp):
            continue
        key = (m.group(1), m.group(2) or "")
        try:
            st = os.lstat(fp)
            groups.setdefault(key, []).append((st.st_mtime, st.st_size, fp))
        except OSError:
            pass
    for entries in groups.values():
        if len(entries) <= 1:
            continue
        entries.sort(reverse=True)
        for _, _, old_fp in entries[1:]:
            try:
                os.unlink(old_fp)
            except OSError:
                pass

if under_cap():
    sys.exit(0)

# Stage 5: LRU eviction by mtime until total_bytes <= cap.
cur_bytes = total_bytes(target)
cap_bytes = cap_mb * 1024 * 1024

def evict_oldest(paths):
    global cur_bytes
    items = []
    for p in paths:
        if os.path.isdir(p):
            for root, _, files in os.walk(p, followlinks=False):
                for name in files:
                    fp = os.path.join(root, name)
                    try:
                        st = os.lstat(fp)
                        items.append((st.st_mtime, st.st_size, fp))
                    except OSError:
                        pass
    items.sort()  # oldest mtime first
    for _, sz, fp in items:
        if cur_bytes <= cap_bytes:
            return True
        try:
            os.unlink(fp)
            cur_bytes -= sz
        except OSError:
            pass
    return cur_bytes <= cap_bytes

# 5a: release deps/build (preserves top-level compiled binaries like target/release/dj).
if evict_oldest([os.path.join(target, "release", "deps"), os.path.join(target, "release", "build")]):
    sys.exit(0)

# 5b: oldest test executables in target/debug/deps (extensionless executables).
debug_deps = os.path.join(target, "debug", "deps")
if os.path.isdir(debug_deps):
    execs = []
    others = []
    for name in os.listdir(debug_deps):
        fp = os.path.join(debug_deps, name)
        if not os.path.isfile(fp) or os.path.islink(fp):
            continue
        try:
            st = os.lstat(fp)
        except OSError:
            continue
        if "." not in name and os.access(fp, os.X_OK):
            execs.append((st.st_mtime, st.st_size, fp))
        else:
            others.append((st.st_mtime, st.st_size, fp))
    execs.sort()
    for _, sz, fp in execs:
        if cur_bytes <= cap_bytes:
            sys.exit(0)
        try:
            os.unlink(fp)
            cur_bytes -= sz
        except OSError:
            pass
    # 5c: oldest .rlib/.rmeta/.d files in target/debug/deps.
    others.sort()
    for _, sz, fp in others:
        if cur_bytes <= cap_bytes:
            sys.exit(0)
        try:
            os.unlink(fp)
            cur_bytes -= sz
        except OSError:
            pass
PY

AFTER_MB="$(dir_mb)"
echo "trimmed target/: ${BEFORE_MB} MiB -> ${AFTER_MB} MiB (cap: ${MAX_TARGET_MB} MiB)"
