#!/usr/bin/env bash
# Moves condr-ghostty-vt to another Ghostty commit (ADR 0051): downloads the lib-vt
# source tarball upstream publishes for it, regenerates the bindings from its headers
# and rewrites the manifest `build.rs` checks against.
#
#   script/update-libghostty-vt.sh <full ghostty commit>
#
# Commit the manifest, the bindings and the safe layer's changes together. A function
# the safe layer starts calling goes into `functions` below first.
set -euo pipefail

commit="${1:?usage: script/update-libghostty-vt.sh <full ghostty commit>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
crate="$root/crates/condr-ghostty-vt"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Only what the safe layer calls; bindgen pulls in the types they name. `types` adds the
# ones only a callback or an option's `void*` names.
functions=(
  ghostty_cell_get
  ghostty_free
  ghostty_formatter_format_alloc
  ghostty_formatter_free
  ghostty_formatter_terminal_new
  ghostty_grid_ref_hyperlink_uri
  ghostty_render_state_free
  ghostty_render_state_get
  ghostty_render_state_new
  ghostty_render_state_row_cells_free
  ghostty_render_state_row_cells_get
  ghostty_render_state_row_cells_new
  ghostty_render_state_row_cells_next
  ghostty_render_state_row_get
  ghostty_render_state_row_iterator_free
  ghostty_render_state_row_iterator_new
  ghostty_render_state_row_iterator_next
  ghostty_render_state_row_set
  ghostty_render_state_set
  ghostty_render_state_update
  ghostty_row_get
  ghostty_terminal_compress
  ghostty_terminal_compression_activity
  ghostty_terminal_free
  ghostty_terminal_get
  ghostty_terminal_grid_ref
  ghostty_terminal_new
  ghostty_terminal_resize
  ghostty_terminal_scroll_viewport
  ghostty_terminal_select_line
  ghostty_terminal_select_word
  ghostty_terminal_selection_format_alloc
  ghostty_terminal_set
  ghostty_terminal_vt_write
)
types=(
  GhosttyBuffer
  GhosttyCellContentTag
  GhosttyCellData
  GhosttyCellWide
  GhosttyClipboardWrite
  GhosttyClipboardWriteReply
  GhosttyColorPaletteIndex
  GhosttyColorRgb
  GhosttyDeviceAttributes
  GhosttyRenderStateCursor
  GhosttyRenderStateCursorVisualStyle
  GhosttyRenderStateDirty
  GhosttyRenderStateRowSelection
  GhosttyRowData
  GhosttySgrUnderline
  GhosttySizeReportSize
  GhosttyString
  GhosttyStyle
  GhosttyTerminalDesktopNotification
  GhosttyTerminalModeConfig
  GhosttyTerminalProgramStatus
  GhosttyTerminalScreen
  GhosttyTerminalScrollbar
  GhosttyTerminalSemanticPrompt
)

curl -fsSL --retry 3 -o "$work/source.tar.gz" \
  "https://tip.files.ghostty.org/$commit/libghostty-vt-source.tar.gz"
tar -xzf "$work/source.tar.gz" -C "$work"
source_dir="$(echo "$work"/libghostty-vt-*)"

allowlist="$(IFS='|'; echo "^(${functions[*]})$")"
type_allowlist="$(IFS='|'; echo "^(${types[*]})$")"
# Doc comments left out so another libclang cannot change the output; the layout tests
# bindgen emits run at compile time on every target.
mise exec cargo:bindgen-cli@0.73.2 -- bindgen "$source_dir/include/ghostty/vt.h" \
  --output "$crate/src/bindings.rs" \
  --rust-target 1.85 --rust-edition 2024 \
  --no-doc-comments \
  --default-enum-style newtype \
  --allowlist-function "$allowlist" \
  --allowlist-type "$type_allowlist" \
  -- -I"$source_dir/include" -DGHOSTTY_STATIC

zig="$(sed -n 's/^ *\.minimum_zig_version = "\(.*\)",$/\1/p' "$source_dir/build.zig.zon")"
version="$(cat "$source_dir/VERSION")"
source_sha256="$(shasum -a 256 "$work/source.tar.gz" | cut -d' ' -f1)"
# Every header's bytes, in byte order of their paths; `build/checks.rs` hashes the same.
headers_sha256="$(cd "$source_dir/include" && find ghostty -name '*.h' | LC_ALL=C sort \
  | xargs cat | shasum -a 256 | cut -d' ' -f1)"

cat >"$crate/ghostty.toml" <<TOML
# The libghostty-vt condr-ghostty-vt builds (ADR 0051). Written by
# script/update-libghostty-vt.sh; move it only with that script.
version = "$version"
commit = "$commit"
source_sha256 = "$source_sha256"
zig = "$zig"
headers_sha256 = "$headers_sha256"
TOML
echo "libghostty-vt $version: bindings and manifest updated"
