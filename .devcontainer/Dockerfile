# syntax=docker/dockerfile:1

# ============================================================
# Stage 1: Prepare base container
# ============================================================
FROM mcr.microsoft.com/devcontainers/base:debian AS base

# -- System packages ----------------------------------------------------------
# Includes the Tauri 2 Linux build dependencies (webkit2gtk 4.1).
# Building bundles (deb/AppImage) works in-container; running the GUI via
# `bun tauri dev` needs host display forwarding.
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
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
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# -- The Windows build's lint through cargo-xwin --------------------------------
# cargo-xwin calls clang-cl and llvm-lib, which Debian's clang and lld carry without these names:
# clang takes the cl driver by the name it is called with, and lld-link makes a library with /lib.
RUN ln -s /usr/bin/clang /usr/local/bin/clang-cl \
    && printf '#!/bin/sh\nexec lld-link /lib "$@"\n' > /usr/local/bin/llvm-lib \
    && chmod 755 /usr/local/bin/llvm-lib


# ============================================================
# Stage 2: Prepare development container
# ============================================================
FROM base

ARG USERNAME=vscode

USER ${USERNAME}

# Ensure all paths are available
ENV PATH="/home/${USERNAME}/.local/bin:${PATH}"
