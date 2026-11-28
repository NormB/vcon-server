# The image ClusterFuzzLite builds the fuzz targets in. The base image is
# pinned by digest; its own nightly Rust is older than this workspace
# needs, so the nightly named in fuzz/rust-toolchain.toml is installed.
FROM gcr.io/oss-fuzz-base/base-builder-rust:v1@sha256:53035c6f510f5f949828ab2cb70f551c3c37e43d94c26579228c9f1fa5fc6a1e
COPY fuzz/rust-toolchain.toml /tmp/rust-toolchain.toml
RUN channel="$(sed -n 's/^channel = "\(.*\)"$/\1/p' /tmp/rust-toolchain.toml)" \
 && test -n "$channel" \
 && rustup toolchain install "$channel" --profile minimal --component rust-src \
 && rm /tmp/rust-toolchain.toml
# The tree as ClusterFuzzLite was given it, not a fresh clone, so a pull
# request is fuzzed as it stands.
COPY . $SRC/vcon-server
COPY .clusterfuzzlite/build.sh $SRC/build.sh
WORKDIR $SRC/vcon-server
