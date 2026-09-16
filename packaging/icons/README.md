# Application icons

`condr.svg` is the platform-neutral source for Condr's application logo. Keep its
original colors; it is branding, not a theme-tinted action glyph. The SVG keeps
the full square canvas. The icon generator keeps PNG square for the GUI title bar
and X11, and applies the desktop platform's rounded corner mask to ICO and macOS
iconset assets; Linux packaging uses the source SVG directly.

After editing the SVG, regenerate and commit its derived assets:

```sh
cargo run --locked -p condr-gui --example generate_icons
```

- `condr.png`: embedded in the GUI title bars and Start Page, and supplied to X11.
- `condr.ico`: 16, 32, 48, 64, 128 and 256 pixel images, embedded as resource 1
  in both Windows executables and used by Inno Setup. Shortcuts and the uninstall
  entry use the GUI executable's icon.
- `condr.iconset/`: standard macOS 1x/2x images through 1024 pixels. The macOS
  packaging script uses the system `iconutil` to produce `Contents/Resources/condr.icns`,
  referenced by `CFBundleIconFile` for Finder, Dock and the application switcher.
- Linux packaging installs `condr.svg` through linuxdeploy. `condr.desktop`,
  `StartupWMClass` and GPUI's window `app_id` all use `condr` for desktop grouping.

Normal builds consume the committed assets. Windows builds need the Windows SDK
resource compiler; image conversion needs no Python, ImageMagick or Zig.
