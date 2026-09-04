# Built-in terminal color schemes

The `*.toml` files here are copied verbatim from the `alacritty/` directory of
[iTerm2-Color-Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes)
at commit `752a9c079396cc9939b86e893578ed81e80c140f`, under the MIT license in
`LICENSE`. The collection's license notes that each individual scheme remains
the work of its own author.

It is a curated subset, not the whole collection: the default schemes of common
terminals (Kitty, Ghostty, iTerm2, macOS Terminal, Windows Terminal, WezTerm,
Xcode, JetBrains) plus widely used families (Dracula, Gruvbox, Nord, Catppuccin,
Tokyo Night, Solarized, Monokai, Material, Rosé Pine, Kanagawa, Everforest,
GitHub, ...). Add a scheme by copying its upstream file here.

`build.rs` embeds every `*.toml` here; the file stem is the scheme name shown in
Settings. To update, refresh the kept files from a newer upstream copy and bump
the commit above.
