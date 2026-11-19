#!/usr/bin/env bash
# Tests agent CLIs end to end in throwaway Docker containers: each one gets a fresh home,
# a condr, and the model the host's Codex config names. Nothing is installed on the host.
#
#   script/agent-e2e/run.sh                         # every agent the image carries
#   script/agent-e2e/run.sh pi grok                 # just these
#   script/agent-e2e/run.sh --refresh pi            # install the CLIs' latest releases
#   CONDR_VERSION=local script/agent-e2e/run.sh     # build this checkout for Linux first
#   CONDR_VERSION=v0.2.4 script/agent-e2e/run.sh    # test a release; nightly by default
#
# `local` builds in a throwaway Rust container; its cache stays in the Docker volumes
# condr-linux-target and condr-linux-cargo, so a rebuild only compiles what changed.
#
# Results: target/agent-e2e/<time>/report.md, each agent's screen and Server log beside it.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
image=condr-agent-e2e
codex_config=${CODEX_HOME:-$HOME/.codex}/config.toml

refresh=()
if [[ ${1:-} == --refresh ]]; then
    refresh=(--build-arg "CLIS=$(date +%s)")
    shift
fi
agents=("$@")
[[ ${#agents[@]} -gt 0 ]] || agents=(pi omp grok copilot opencode kimi qwen goose kilo codebuddy minimax vibe)
[[ -f $codex_config ]] || { echo "no Codex config at $codex_config" >&2; exit 1; }

version=${CONDR_VERSION:-nightly}
condr=()
if [[ $version == local ]]; then
    toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$repo/rust-toolchain.toml")
    zig=$(sed -n 's/^zig = "\(.*\)"/\1/p' "$repo/crates/condr-ghostty-vt/ghostty.toml")
    echo "building condr from this checkout for Linux…"
    docker run --rm -v "$repo:/src:ro" -v condr-linux-target:/target -v condr-linux-cargo:/cargo \
        -e CARGO_TARGET_DIR=/target -e CARGO_HOME=/cargo -e XDG_CACHE_HOME=/cargo/cache \
        -e ZIG_GLOBAL_CACHE_DIR=/cargo/cache/zig -e "RUSTUP_TOOLCHAIN=$toolchain" -e "ZIG=$zig" \
        "rust:$toolchain-trixie" bash -c 'set -e
            dir=/cargo/zig-$ZIG
            if [ ! -x "$dir/zig" ]; then
                part=$(mktemp -d /cargo/zig-part.XXXXXX)
                curl -fsSL --retry 3 \
                    "https://ziglang.org/download/$ZIG/zig-$(uname -m)-linux-$ZIG.tar.xz" \
                    | tar -xJ --strip-components=1 -C "$part"
                rm -rf "$dir" && mv "$part" "$dir"
            fi
            cd /src && PATH=$dir:$PATH cargo build --locked --quiet -p condr-server --bin condr'
    # Over the downloaded nightly; the test image is Debian trixie too, so its glibc matches.
    condr=(--mount type=volume,src=condr-linux-target,dst=/opt/condr-headless,volume-subpath=debug,readonly)
    version=nightly
fi

docker build ${refresh[@]+"${refresh[@]}"} --build-arg "CONDR_VERSION=$version" \
    --build-arg "FETCHED=$(date +%s)" -t "$image" "$here"
out=$repo/target/agent-e2e/$(date +%Y%m%d-%H%M%S)
mkdir -p "$out"
run=(docker run --rm --init ${condr[@]+"${condr[@]}"}
    -v "$here/e2e.py:/e2e.py:ro" -v "$codex_config:/run/codex.toml:ro" -v "$out:/out" "$image")

# One container per agent, three at a time: a gateway may cap concurrent requests.
failed=0
printf '%s\n' "${agents[@]}" | xargs -P 3 -I{} "${run[@]}" python3 /e2e.py {} || failed=1
echo
"${run[@]}" python3 /e2e.py --report | tee "$out/report.md"
echo "screens and Server logs: $out"
exit "$failed"
