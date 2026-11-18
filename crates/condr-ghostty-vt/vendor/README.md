# Vendored Zig packages

Zig packages libghostty-vt needs that Ghostty's own mirror (`deps.files.ghostty.org`)
does not carry, one `<package hash>.tar.gz` each. `build.rs` extracts them into the
source's `zig-pkg/`, so Zig fetches nothing for them. `script/update-libghostty-vt.sh`
writes them; each keeps its own license inside.

- `aro-…`: [arocc](https://codeberg.org/ziglang/arocc), MIT and Unicode V3, a
  dependency of Ghostty's `pkg/translate-c`, otherwise fetched from Codeberg over git.
