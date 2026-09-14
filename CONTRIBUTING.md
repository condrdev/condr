# Contributing to Condr

Thanks for your interest. Condr is early and moving fast, so please open an issue before starting anything larger than a small fix.

## Before you start

- Read [`AGENTS.md`](AGENTS.md) for the architecture, decided tech choices and performance requirements. They apply to humans and coding agents alike.
- Read [`CONTEXT.md`](CONTEXT.md) for vocabulary and [`docs/adr/`](docs/adr/) for the decisions behind the current design. If your change contradicts an ADR, say so in the issue instead of working around it.
- The project is unreleased: breaking changes are fine, compatibility shims are not.

## Building

Rust `1.95.0`, pinned in `rust-toolchain.toml` so `rustup` picks it up automatically. CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `clippy` and `cargo test` on Linux and Windows for every pull request; see `docs/releases.md` for how builds are published.

```bash
cargo build                 # everything
cargo run -p condr-gui      # the GUI (needs a display)
cargo run -p condr-server -- server --help
cargo test                  # all tests, headless-safe
cargo clippy --all-targets
cargo fmt
```

Linux GUI builds need:

```bash
sudo apt-get install -y clang cmake pkg-config libfontconfig1-dev libfreetype6-dev \
  libxcb1-dev libxkbcommon-x11-dev libwayland-dev libssl-dev libvulkan1
```

`condr-core` and `condr-server` build and test on a headless machine. The GUI does not cross-compile; build it natively on the target OS. Windows changes must also be checked against the ConPTY path.

## Making changes

- One concern per pull request. Keep the diff as small as the change allows.
- Follow [Conventional Commits](https://www.conventionalcommits.org/): `feat(gui): …`, `fix(server): …`, `docs: …`, `refactor(core): …`.
- Add or update tests in the same PR. Terminal/performance changes must cover the cases listed in `AGENTS.md`.
- Update `CONTEXT.md`, the relevant ADR or `docs/` when behaviour or a decision changes. New architectural decisions get a new ADR in `docs/adr/`.
- `cargo fmt` and `cargo clippy --all-targets` must be clean.
- Do not add dependencies for something the standard library or an existing dependency already does. Never add `git2` or shell out to `git`; use `gix`.

## Licensing

Condr is Apache-2.0. By contributing you agree your contribution is licensed under the same terms. Do not copy code from GPL-licensed projects; ideas only.

## Reporting issues

Use the issue templates. Bug reports need the Condr build (release name or commit), OS, and which agent CLI was running in the pane. Security issues go through [`SECURITY.md`](SECURITY.md), not the public tracker.
