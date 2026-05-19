# ─── Stage 1: build ──────────────────────────────────────────────────────────
# Use the full bookworm image (not slim) so we have gcc, pkg-config, and the
# rest of build-essential.  This stage never ships to users — only the binary
# is extracted in stage 2.
FROM rust:1 AS builder
WORKDIR /build

# ── Dependency layer (cached until Cargo.toml or Cargo.lock changes) ──────────
# Copy only the manifest files and synthesise the smallest possible stub source
# that satisfies both the [lib] and [[bin]] targets declared in Cargo.toml.
# Without matching stubs `cargo build` cannot resolve the crate graph and the
# dependency-only build fails.
#
# After the stub build we remove the stub sources so that Cargo's fingerprint
# system detects a source change when the real source is copied in the next
# layer and forces a recompile of our crates (but NOT the cached deps).
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && printf 'pub fn _dummy() {}\n' > src/lib.rs \
    && printf 'fn main() {}\n'      > src/main.rs \
    && cargo build --release        \
    && rm src/lib.rs src/main.rs

# ── Source layer (re-runs only when src/ changes) ─────────────────────────────
# Touching every .rs file updates their mtime so Cargo knows to recompile our
# crates even if file contents happen to match the stub at the byte level
# (which won't happen in practice, but the touch is a cheap guarantee).
COPY src ./src
RUN find src -name '*.rs' | xargs touch \
    && cargo build --release

# Strip debug symbols: ~22 MB → ~6 MB.  objcopy/strip are part of binutils
# which is present in the rust:1 image via build-essential.
RUN strip target/release/diff-crawler

# ─── Stage 2: runtime ────────────────────────────────────────────────────────
# debian:bookworm-slim contains glibc 2.36 and libpthread — the only runtime
# deps our binary has.  ca-certificates is included so any future HTTPS calls
# (e.g. from a plugin or registry lookup) work out of the box.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/diff-crawler /usr/local/bin/diff-crawler

# Run as nobody by default so the container doesn't start as root.
# The host-side wrapper script passes --user $(id -u):$(id -g) to ensure
# output files are owned by the calling user, not root.
USER nobody

ENTRYPOINT ["/usr/local/bin/diff-crawler"]
