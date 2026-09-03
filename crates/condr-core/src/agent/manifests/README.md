# Agent detection manifests

These TOML files are copied verbatim from [herdr](https://github.com/herdrdev/herdr)
`src/detect/manifests/` at commit `5158adab10b6dcfea9370782043392f80fa0643c`
(Apache-2.0, Copyright the herdr contributors). Keep them byte-identical to upstream
so they can be refreshed with a plain copy; Condr-specific tweaks belong in a local
override at `<config dir>/agent-detection/<id>.toml`, which replaces the bundled
manifest of the same id.

The engine that reads them lives in `../manifest.rs` and understands manifest engine
version 3 (`min_engine_version`).
