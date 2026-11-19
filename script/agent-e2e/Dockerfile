# The throwaway machine script/agent-e2e/run.sh tests agent CLIs in: a released condr
# and the CLIs under test, nothing of the host's.
FROM node:24-trixie-slim

# ripgrep and fd as a developer's machine has them: without, Pi downloads them while its
# prompt already shows, and drops what is typed meanwhile. No procps, on purpose: a hook
# run without a terminal (Grok's) must reach its Pane without `ps`, as in any container.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git python3 zstd \
        ripgrep fd-find \
    && ln -s /usr/bin/fdfind /usr/local/bin/fd \
    && rm -rf /var/lib/apt/lists/*

# Each CLI's latest release; run.sh --refresh changes CLIS to install them again.
ARG CLIS=0
RUN npm install -g --no-audit --no-fund \
        @earendil-works/pi-coding-agent @github/copilot opencode-ai
RUN arch=$(uname -m | sed 's/aarch64/arm64/; s/x86_64/x64/') \
    && curl -fsSL -o /usr/local/bin/omp \
        "https://github.com/can1357/oh-my-pi/releases/latest/download/omp-linux-${arch}" \
    && chmod +x /usr/local/bin/omp
# The installer keeps the binary under $HOME and links it; a readable $HOME lets the
# unprivileged user run it.
RUN curl -fsSL https://x.ai/cli/install.sh | HOME=/opt/grok GROK_BIN_DIR=/usr/local/bin bash \
    && chmod -R a+rX /opt/grok
# Kimi Code; the older kimi-cli replaces itself with it on first start.
RUN curl -fsSL https://code.kimi.com/kimi-code/install.sh | KIMI_INSTALL_DIR=/usr/local bash
RUN npm install -g --no-audit --no-fund @qwen-code/qwen-code
RUN curl -fsSL "https://github.com/aaif-goose/goose/releases/latest/download/goose-$(uname -m)-unknown-linux-gnu.tar.gz" \
        | tar -xz -C /usr/local/bin
RUN npm install -g --no-audit --no-fund @kilocode/cli
RUN npm install -g --no-audit --no-fund @tencent-ai/codebuddy-code

# A release tag, `nightly` or `latest`, fetched again on every build: run.sh changes FETCHED.
ARG CONDR_VERSION=nightly
ARG FETCHED=0
# The headless build, checked against the release's SHA256SUMS as script/install-condr.sh does.
RUN set -e; arch=$(uname -m | sed 's/aarch64/arm64/'); \
    case "$CONDR_VERSION" in latest) api=releases/latest ;; *) api="releases/tags/$CONDR_VERSION" ;; esac; \
    urls=$(curl -fsSL "https://api.github.com/repos/condrdev/condr/$api" \
        | sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p'); \
    archive=$(echo "$urls" | grep "/condr-headless-.*-linux-$arch\.tar\.gz$"); \
    cd "$(mktemp -d)" && curl -fsSLO "$archive" && curl -fsSLO "$(echo "$urls" | grep '/SHA256SUMS$')"; \
    grep " $(basename "$archive")\$" SHA256SUMS | sha256sum -c; \
    tar -xzf "$(basename "$archive")" -C /opt && rm -rf "$PWD"; \
    ln -s /opt/condr-headless/condr /usr/local/bin/condr

RUN useradd --create-home --shell /bin/bash agent && mkdir /out && chown agent:agent /out
USER agent
ENV LANG=C.UTF-8
WORKDIR /home/agent
