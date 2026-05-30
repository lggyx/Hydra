#!/bin/bash
# Codesign + notarize macOS `hydra` binaries for distribution.
#
# Only signs the `hydra` CLI. `hydra-daemon` is NOT signed (per project
# decision — daemon runs inside CI/user-controlled environments, user-facing
# Gatekeeper only inspects the `hydra` launcher).
#
# Notarization: requires a keychain profile pre-created with
#   xcrun notarytool store-credentials hydra-notary \
#     --apple-id "you@example.com" --team-id T949H383MF \
#     --password <app-specific-password>
#
# Override via env:
#   HYDRA_SIGN_IDENTITY     (default: the project's team Developer ID)
#   HYDRA_NOTARY_PROFILE    (default: hydra-notary)
#   HYDRA_SKIP_NOTARIZE=1   (codesign only, skip notarization)
set -euo pipefail

IDENTITY="${HYDRA_SIGN_IDENTITY:-Developer ID Application: Chongqing Kaiyuan Gongchuang Technology Co., Ltd. (T949H383MF)}"
NOTARY_PROFILE="${HYDRA_NOTARY_PROFILE:-hydra-notary}"
SKIP_NOTARIZE="${HYDRA_SKIP_NOTARIZE:-0}"

usage() {
    cat <<EOF
Usage: $0 <binary-path | dist-directory>

Examples:
    $0 target/release/hydra
    $0 dist/v2.5.0

Env:
    HYDRA_SIGN_IDENTITY   override signing identity
    HYDRA_NOTARY_PROFILE  override notarytool keychain profile (default: hydra-notary)
    HYDRA_SKIP_NOTARIZE=1 codesign only, skip notarization
EOF
    exit 1
}

[ $# -eq 1 ] || usage

TARGET="$1"

# --- Preflight: identity must exist in keychain ---
if ! security find-identity -v -p codesigning | grep -q "$IDENTITY"; then
    echo "Error: signing identity not found in keychain:"
    echo "  $IDENTITY"
    echo ""
    echo "Available identities:"
    security find-identity -v -p codesigning
    exit 1
fi

# --- Preflight: notarytool profile exists (if we'll notarize) ---
if [ "$SKIP_NOTARIZE" != "1" ]; then
    if ! xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1; then
        cat <<EOF
Error: notarytool keychain profile "$NOTARY_PROFILE" not found or invalid.

Create it once with:
    xcrun notarytool store-credentials $NOTARY_PROFILE \\
      --apple-id "YOUR_APPLE_ID" \\
      --team-id T949H383MF \\
      --password "APP_SPECIFIC_PASSWORD"

Or skip notarization: HYDRA_SKIP_NOTARIZE=1 $0 $1
EOF
        exit 1
    fi
fi

sign_one() {
    local bin="$1"
    echo "[sign] $(basename "$bin")"
    # --force: replace Rust's adhoc linker-signed signature
    # --timestamp: RFC 3161 timestamp (required for notarization)
    # --options=runtime: hardened runtime (required for notarization)
    codesign \
        --force \
        --timestamp \
        --options=runtime \
        --sign "$IDENTITY" \
        "$bin"
    codesign --verify --strict "$bin"
}

notarize_one() {
    local bin="$1"
    local tmpzip
    tmpzip=$(mktemp -d)/"$(basename "$bin").zip"
    # ditto produces a zip that notarytool accepts for bare binaries.
    /usr/bin/ditto -c -k --keepParent "$bin" "$tmpzip"

    echo "[notarize] submitting $(basename "$bin")..."
    # --wait blocks until Apple finishes processing (usually < 2 min for CLI bin).
    xcrun notarytool submit "$tmpzip" \
        --keychain-profile "$NOTARY_PROFILE" \
        --wait

    rm -f "$tmpzip"
    # Note: tar.gz / bare Mach-O cannot carry a stapled ticket. Gatekeeper
    # queries Apple online on first launch. For offline-valid tickets you'd
    # need a .dmg or .pkg container + `xcrun stapler staple`.
}

is_macho_hydra_bin() {
    local path="$1"
    local base
    base=$(basename "$path")
    # Must be a regular file named `hydra` or `hydra-<version>-darwin-<arch>`,
    # and must NOT be the daemon.
    case "$base" in
        hydra-daemon*) return 1 ;;
        hydra|hydra-*-darwin-*) ;;
        *) return 1 ;;
    esac
    [ -f "$path" ] || return 1
    # Skip packaged artifacts (the tar.gz would match the glob otherwise).
    case "$base" in
        *.tar.gz|*.zip|*.sig|*.txt) return 1 ;;
    esac
    file "$path" 2>/dev/null | grep -q "Mach-O"
}

process_one() {
    local bin="$1"
    sign_one "$bin"
    if [ "$SKIP_NOTARIZE" != "1" ]; then
        notarize_one "$bin"
    fi
}

if [ -f "$TARGET" ]; then
    if is_macho_hydra_bin "$TARGET"; then
        process_one "$TARGET"
    else
        echo "Error: $TARGET is not a macOS hydra binary."
        exit 1
    fi
elif [ -d "$TARGET" ]; then
    found=0
    for bin in "$TARGET"/hydra*; do
        [ -e "$bin" ] || continue
        if is_macho_hydra_bin "$bin"; then
            process_one "$bin"
            found=$((found + 1))
        fi
    done
    if [ $found -eq 0 ]; then
        echo "No signable hydra binaries found in $TARGET"
        exit 1
    fi
    echo ""
    echo "Signed $found binary(ies)."
else
    echo "Error: $TARGET not found"
    exit 1
fi
