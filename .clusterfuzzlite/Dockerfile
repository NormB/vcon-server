# The image ClusterFuzzLite builds the fuzz targets in. The base image is
# pinned by digest; its own nightly Rust is older than this workspace
# needs, so the nightly named in fuzz/rust-toolchain.toml is installed.
FROM gcr.io/oss-fuzz-base/base-builder-rust:v1@sha256:8d2315aeda612bc4176bf20057a72db2a7f61cdf4a8699829c147ab5af02d234
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
