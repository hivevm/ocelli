# syntax=docker/dockerfile:1

FROM mcr.microsoft.com/devcontainers/base:debian

# -- System packages ----------------------------------------------------------
# Includes the Tauri 2 Linux build dependencies (webkit2gtk 4.1).
# Building bundles (deb/AppImage) works in-container; running the GUI via
# `bun tauri dev` needs host display forwarding. ShellCheck lets the pre-push hook run the shell
# lint CI runs; without it the lint skips itself locally.
RUN apt-get update && apt-get install -y --no-install-recommends \
    libwebkit2gtk-4.1-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libssl-dev \
    libxdo-dev \
    clang \
    lld \
    xdg-utils \
    pkg-config \
    file \
    shellcheck \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# -- The Windows build's lint through cargo-xwin --------------------------------
# cargo-xwin calls clang-cl and llvm-lib, which Debian's clang and lld carry without these names:
# clang takes the cl driver by the name it is called with, and lld-link makes a library with /lib.
RUN ln -s /usr/bin/clang /usr/local/bin/clang-cl \
    && printf '#!/bin/sh\nexec lld-link /lib "$@"\n' > /usr/local/bin/llvm-lib \
    && chmod 755 /usr/local/bin/llvm-lib

USER vscode
ENV PATH="/home/vscode/.local/bin:${PATH}"
