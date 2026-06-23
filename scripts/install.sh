#!/bin/sh
# emede installer — user-local, no sudo.
#
#   curl -fsSL https://raw.githubusercontent.com/jmejia8/emede/main/scripts/install.sh | sh
#
# Installs the emede binary to ~/.local/bin and registers a desktop entry + icon
# so it can open markdown files from your file manager.
#
# Environment overrides:
#   EMEDE_VERSION             Pin a version (e.g. 0.1.5). Default: latest release.
#   EMEDE_INSTALL_DIR         Binary install dir. Default: ~/.local/bin
#   EMEDE_ALLOW_UNVERIFIED=1  Continue when no .sha256 checksum is published.
#
# Flags:
#   --uninstall   Remove emede, its desktop entry and icon.
#   --help        Show this help.

set -eu

REPO="jmejia8/emede"
INSTALL_DIR="${EMEDE_INSTALL_DIR:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
DESKTOP_DIR="$DATA_DIR/applications"
ICON_DIR="$DATA_DIR/icons/hicolor/128x128/apps"
DESKTOP_FILE="$DESKTOP_DIR/emede.desktop"
ICON_FILE="$ICON_DIR/emede.png"

# --- logging helpers --------------------------------------------------------
if [ -t 1 ]; then
  BOLD="$(printf '\033[1m')"; RED="$(printf '\033[31m')"
  GREEN="$(printf '\033[32m')"; YELLOW="$(printf '\033[33m')"
  RESET="$(printf '\033[0m')"
else
  BOLD=""; RED=""; GREEN=""; YELLOW=""; RESET=""
fi
info()  { printf '%s>%s %s\n' "$GREEN" "$RESET" "$*"; }
warn()  { printf '%s!%s %s\n' "$YELLOW" "$RESET" "$*" >&2; }
err()   { printf '%serror:%s %s\n' "$RED" "$RESET" "$*" >&2; }
die()   { err "$*"; exit 1; }

usage() {
  sed -n '2,/^set -eu/p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//; /^set -eu/d' || true
  cat <<EOF
Usage: install.sh [--uninstall] [--help]
EOF
}

# --- tool detection ---------------------------------------------------------
have() { command -v "$1" >/dev/null 2>&1; }

# Download URL to file ($1=url $2=dest). Returns non-zero on failure.
download() {
  if have curl; then
    curl -fsSL "$1" -o "$2"
  elif have wget; then
    wget -qO "$2" "$1"
  else
    die "need curl or wget to download files"
  fi
}

# Fetch URL to stdout ($1=url).
fetch() {
  if have curl; then
    curl -fsSL "$1"
  elif have wget; then
    wget -qO- "$1"
  else
    die "need curl or wget to download files"
  fi
}

refresh_caches() {
  if have update-desktop-database; then
    update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
  fi
  if have gtk-update-icon-cache; then
    gtk-update-icon-cache "$DATA_DIR/icons/hicolor" >/dev/null 2>&1 || true
  fi
}

# --- uninstall --------------------------------------------------------------
uninstall() {
  info "Removing emede..."
  removed=0
  for f in "$INSTALL_DIR/emede" "$DESKTOP_FILE" "$ICON_FILE"; do
    if [ -e "$f" ]; then
      rm -f "$f" && { info "removed $f"; removed=1; }
    fi
  done
  [ "$removed" -eq 1 ] || warn "nothing to remove (emede not found in default locations)"
  refresh_caches
  info "Done."
}

# --- parse args -------------------------------------------------------------
for arg in "$@"; do
  case "$arg" in
    --uninstall) uninstall; exit 0 ;;
    -h|--help)   usage; exit 0 ;;
    *)           die "unknown argument: $arg (try --help)" ;;
  esac
done

# --- platform check ---------------------------------------------------------
os="$(uname -s)"
[ "$os" = "Linux" ] || die "this installer supports Linux only (detected: $os)"

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) asset_arch="x86_64" ;;
  *) die "unsupported architecture: $arch (only x86_64 binaries are published)" ;;
esac

# --- resolve version --------------------------------------------------------
if [ -n "${EMEDE_VERSION:-}" ]; then
  VERSION="${EMEDE_VERSION#v}"
else
  info "Resolving latest release..."
  tag="$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
    | grep -m1 '"tag_name"' \
    | sed -e 's/.*"tag_name"[[:space:]]*:[[:space:]]*"//' -e 's/".*//')"
  [ -n "$tag" ] || die "could not determine latest version (GitHub API request failed)"
  VERSION="${tag#v}"
fi
info "Installing emede ${BOLD}v$VERSION${RESET} ($asset_arch)"

ARCHIVE="emede-v$VERSION-$asset_arch-linux.tar.gz"
BASE_URL="https://github.com/$REPO/releases/download/v$VERSION"

# --- work in a temp dir -----------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

info "Downloading $ARCHIVE..."
download "$BASE_URL/$ARCHIVE" "$TMP/$ARCHIVE" \
  || die "download failed: $BASE_URL/$ARCHIVE"

# --- verify checksum --------------------------------------------------------
if download "$BASE_URL/$ARCHIVE.sha256" "$TMP/$ARCHIVE.sha256" 2>/dev/null; then
  info "Verifying checksum..."
  # The .sha256 file references the archive by name; verify from inside TMP.
  ( cd "$TMP" && {
      if have sha256sum; then sha256sum -c "$ARCHIVE.sha256" >/dev/null
      elif have shasum;  then shasum -a 256 -c "$ARCHIVE.sha256" >/dev/null
      else
        warn "no sha256sum/shasum available; skipping verification"
        exit 0
      fi
    } ) || die "checksum verification failed for $ARCHIVE"
  info "Checksum OK"
else
  if [ "${EMEDE_ALLOW_UNVERIFIED:-0}" = "1" ]; then
    warn "no checksum published for this release; continuing (EMEDE_ALLOW_UNVERIFIED=1)"
  else
    warn "no checksum (.sha256) published for v$VERSION."
    warn "Set EMEDE_ALLOW_UNVERIFIED=1 to install without verification."
    die "aborting unverified install"
  fi
fi

# --- extract ----------------------------------------------------------------
info "Extracting..."
tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
bin_src="$(find "$TMP" -name emede -type f | head -n1)"
[ -n "$bin_src" ] || die "could not find 'emede' binary inside $ARCHIVE"

# --- install binary ---------------------------------------------------------
mkdir -p "$INSTALL_DIR"
install -m 755 "$bin_src" "$INSTALL_DIR/emede"
info "Installed binary to $INSTALL_DIR/emede"

# --- install icon -----------------------------------------------------------
icon_src="$(find "$TMP" -name '128x128.png' -type f | head -n1)"
mkdir -p "$ICON_DIR"
if [ -n "$icon_src" ]; then
  install -m 644 "$icon_src" "$ICON_FILE"
elif download "https://raw.githubusercontent.com/$REPO/v$VERSION/src-tauri/icons/128x128.png" "$ICON_FILE" 2>/dev/null; then
  chmod 644 "$ICON_FILE"
else
  warn "could not obtain app icon; desktop entry will use a generic icon"
fi
[ -f "$ICON_FILE" ] && info "Installed icon to $ICON_FILE"

# --- desktop entry ----------------------------------------------------------
mkdir -p "$DESKTOP_DIR"
cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=emede
Comment=Immersive markdown reader
Exec=$INSTALL_DIR/emede %F
Icon=emede
Terminal=false
Categories=Utility;Viewer;TextEditor;
MimeType=text/markdown;text/x-markdown;
StartupWMClass=emede
EOF
info "Created desktop entry at $DESKTOP_FILE"

refresh_caches

# --- runtime dependency check ----------------------------------------------
if have ldconfig && ! ldconfig -p 2>/dev/null | grep -q 'libwebkit2gtk-4\.1'; then
  warn "runtime dependency 'webkit2gtk 4.1' not detected. Install it:"
  warn "  Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0"
  warn "  Fedora:        sudo dnf install webkit2gtk4.1"
  warn "  Arch/Manjaro:  sudo pacman -S webkit2gtk-4.1"
fi

# --- PATH advice ------------------------------------------------------------
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    warn "$INSTALL_DIR is not on your PATH."
    warn "Add this to your shell rc (~/.bashrc, ~/.zshrc, ...):"
    warn "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac

# --- done -------------------------------------------------------------------
printf '\n%semede v%s installed.%s\n' "$GREEN$BOLD" "$VERSION" "$RESET"
printf 'Open a file with:  %semede file.md%s\n' "$BOLD" "$RESET"
printf 'Uninstall with:    %scurl -fsSL https://raw.githubusercontent.com/%s/main/scripts/install.sh | sh -s -- --uninstall%s\n' "$BOLD" "$REPO" "$RESET"
