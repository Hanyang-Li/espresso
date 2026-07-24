#!/bin/sh
# espresso installer
#
#   curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh | sh
#
# Env overrides:
#   ESPRESSO_VERSION=v0.2.0        pin a specific release (default: latest)
#   ESPRESSO_INSTALL_DIR=/path     install location (default: ~/.local/bin)
#   ESPRESSO_NO_MODIFY_PATH=1      skip editing your shell rc; print the manual
#                                  PATH instructions instead

set -eu

REPO="Hanyang-Li/espresso"
BIN="espresso"
ASSET="espresso-aarch64-apple-darwin.tar.gz"
INSTALL_DIR="${ESPRESSO_INSTALL_DIR:-$HOME/.local/bin}"

err() { printf 'error: %s\n' "$1" >&2; exit 1; }

# Print the manual PATH instructions (used as the opt-out / fallback path).
path_hint() {
    echo "Note: ${INSTALL_DIR} is not on your PATH. Add it with:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo "Or run it directly: ${INSTALL_DIR}/${BIN}"
}

# Put $INSTALL_DIR on PATH by appending to the user's shell rc, idempotently.
# Honors ESPRESSO_NO_MODIFY_PATH; unknown shells fall back to path_hint.
ensure_on_path() {
    if [ -n "${ESPRESSO_NO_MODIFY_PATH:-}" ]; then
        path_hint
        return
    fi

    marker="# added by espresso installer"
    case "$(basename "${SHELL:-}")" in
        zsh)  rc="$HOME/.zshrc";                   line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
        bash) rc="$HOME/.bash_profile";            line="export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
        fish) rc="$HOME/.config/fish/config.fish"; line="fish_add_path ${INSTALL_DIR}" ;;
        *)    path_hint; return ;;
    esac

    if [ -f "$rc" ] && grep -qF "$marker" "$rc" 2>/dev/null; then
        echo "PATH already configured for espresso in $rc"
        echo "Restart your shell or run: source $rc"
        return
    fi

    mkdir -p "$(dirname "$rc")" 2>/dev/null || true
    if ! { printf '\n%s\n%s\n' "$marker" "$line" >> "$rc"; } 2>/dev/null; then
        path_hint
        return
    fi

    echo "Added ${INSTALL_DIR} to PATH in $rc"
    echo "Restart your shell or run: source $rc"
}

# --- environment checks ---
[ "$(uname -s)" = "Darwin" ] || err "espresso only supports macOS."
[ "$(uname -m)" = "arm64" ] || err "espresso currently only supports Apple Silicon (arm64); detected $(uname -m)."

command -v curl >/dev/null 2>&1 || err "curl is required."
command -v shasum >/dev/null 2>&1 || err "shasum is required."

# --- resolve download URL ---
if [ -n "${ESPRESSO_VERSION:-}" ]; then
    base="https://github.com/${REPO}/releases/download/${ESPRESSO_VERSION}"
else
    base="https://github.com/${REPO}/releases/latest/download"
fi

# --- download to a temp dir, cleaned up on exit ---
tmp="$(mktemp -d)"
SUDO=""
STAGE=""
cleanup() {
    rm -rf "$tmp"
    [ -n "$STAGE" ] && $SUDO rm -f "$STAGE" 2>/dev/null
    return 0
}
trap cleanup EXIT

echo "Downloading ${ASSET}..."
curl -fSL --proto '=https' "${base}/${ASSET}" -o "${tmp}/${ASSET}" \
    || err "download failed: ${base}/${ASSET}"
curl -fSL --proto '=https' "${base}/${ASSET}.sha256" -o "${tmp}/${ASSET}.sha256" \
    || err "download failed: ${base}/${ASSET}.sha256"

# --- verify checksum ---
echo "Verifying checksum..."
( cd "$tmp" && shasum -a 256 -c "${ASSET}.sha256" >/dev/null ) \
    || err "checksum verification failed."

# --- extract ---
tar -xzf "${tmp}/${ASSET}" -C "$tmp" || err "extraction failed."
[ -f "${tmp}/${BIN}" ] || err "archive did not contain ${BIN}."

# --- decide whether elevation is needed ---
# The default target (~/.local/bin) lives under $HOME and is always user-
# writable, so installation needs no sudo. Only an overridden, non-writable
# directory (e.g. ESPRESSO_INSTALL_DIR=/usr/local/bin) falls back to sudo.
mkdir -p "$INSTALL_DIR" 2>/dev/null || true
if [ -d "$INSTALL_DIR" ] && [ -w "$INSTALL_DIR" ]; then
    SUDO=""
else
    echo "Installing to ${INSTALL_DIR} requires elevated permissions."
    SUDO="sudo"
    $SUDO mkdir -p "$INSTALL_DIR"
fi

# --- install via same-directory atomic rename ---
# Stage the binary INSIDE $INSTALL_DIR so the final `mv` is always a same-
# filesystem atomic rename. rename(2) swaps the directory entry without opening
# or truncating the destination, so it can replace a currently-running binary
# without ETXTBSY — unlike a cross-filesystem mv, which falls back to copy and
# would truncate the target.
STAGE="${INSTALL_DIR}/.${BIN}.tmp.$$"
$SUDO cp "${tmp}/${BIN}" "$STAGE" || err "failed to stage binary in ${INSTALL_DIR}."
$SUDO chmod 0755 "$STAGE"
$SUDO mv "$STAGE" "${INSTALL_DIR}/${BIN}" || err "failed to install binary to ${INSTALL_DIR}."
STAGE=""

echo ""
echo "Installed $("${INSTALL_DIR}/${BIN}" --version 2>/dev/null || echo "$BIN") to ${INSTALL_DIR}/${BIN}"

# --- make sure it's runnable from the shell ---
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) echo "Run: ${BIN}" ;;
    *) ensure_on_path ;;
esac

echo ""
echo "Next step (optional, for lid-closed wake):"
echo "  sudo ${BIN} daemon install"
