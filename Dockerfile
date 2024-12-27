# NOTE: Ensure builder's Rust version matches CI's in .circleci/config.yml
ARG debian_version=bookworm-slim
ARG rust_image=docker.io/library/rust
ARG rust_version=1.78-slim-bookworm
FROM $rust_image:$rust_version AS buildbase

WORKDIR /app

ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

ARG chef_version=0.1.67

RUN cargo install cargo-chef --locked --version $chef_version

# Either libmysqlclient-dev or libmariadb-dev.
# libmysqlclient-dev is only available for AMD64 from repo.mysql.com.
ARG MYSQLCLIENTPKG=libmariadb-dev

# Fetch and load the MySQL public key.
RUN \
    if [ "$MYSQLCLIENTPKG" = libmysqlclient-dev ] ; then \
       apt-get -q update && \
       DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates wget && \
       wget -q -O/etc/apt/trusted.gpg.d/mysql.asc https://repo.mysql.com/RPM-GPG-KEY-mysql-2023 && \
       echo "deb https://repo.mysql.com/apt/debian/ bookworm mysql-8.0" >> /etc/apt/sources.list && \
       rm -rf /var/lib/apt/lists/* ; \
    fi

# build-essential and cmake are required to build grpcio-sys for Spanner builds
# libssl-dev is required by the openssl-sys crate, used for libmysqlclient-dev.
RUN \
    apt-get -q update && \
    DEBIAN_FRONTEND=noninteractive apt-get -q install -y --no-install-recommends build-essential cmake dpkg-dev $MYSQLCLIENTPKG libssl-dev pkg-config python3-dev python3-pip && \
    rm -rf /var/lib/apt/lists/*


FROM buildbase AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json


FROM buildbase AS builder

ARG rust_image=

# If using a nightly Rust, allow recompiling the standard library.
RUN if [ "$rust_image" = docker.io/rustlang/rust ] ; then rustup component add rust-src ; fi

ARG DATABASE_BACKEND=spanner
ARG CARGO_ARGS=

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --no-default-features --features=syncstorage-db/$DATABASE_BACKEND --features=py_verifier $CARGO_ARGS --recipe-path recipe.json

COPY . .

RUN \
    cargo --version && \
    rustc --version && \
    mkdir -p bin && \
    cargo build --workspace --no-default-features --features=syncstorage-db/$DATABASE_BACKEND --features=py_verifier $CARGO_ARGS --locked --bin syncserver && \
    cp -p target/*/syncserver bin/ && \
    if [ "$DATABASE_BACKEND" = "spanner" ] ; then \
       cargo build --workspace --locked --bin purge_ttl && \
       cp -p target/*/purge_ttl bin/ ; \
    fi

# Creates a file called shlibdeps containing names of packages of shared libraries.
#
# dpkg-shlibdeps outputs
#
#   shlibs:Depends=libc6 (>= 2.36), libgcc-s1 (>= 4.2)
#
# and we reduce that to just an apt-get command line.
RUN \
    mkdir -p debian && \
    touch debian/control && \
    dpkg-shlibdeps -O bin/* | sed -E -e 's;shlibs:Depends=|\([^)]*\)|,\s*; ;g' >shlibdeps && \
    echo shlibdeps: $(cat shlibdeps) && \
    rm -r debian


FROM buildbase AS python_builder

COPY requirements.txt .
COPY tools/integration_tests tools/integration_tests
COPY tools/tokenserver tools/tokenserver

RUN \
    mkdir -p wheels && \
    pip3 wheel --wheel-dir wheels -r requirements.txt -r /app/tools/integration_tests/requirements.txt -r /app/tools/tokenserver/requirements.txt


FROM docker.io/library/debian:$debian_version
WORKDIR /app
COPY --from=builder /app/requirements.txt /app
# Due to a build error that occurs with the Python cryptography package, we
# have to set this env var to prevent the cryptography package from building
# with Rust. See this link for more information:
# https://pythonshowcase.com/question/problem-installing-cryptography-on-raspberry-pi
ENV CRYPTOGRAPHY_DONT_BUILD_RUST=1

RUN \
    groupadd --gid 10001 app && \
    useradd --uid 10001 --gid 10001 --home /app --no-create-home app

COPY --from=builder /app/shlibdeps /app/

# Fetch and load the MySQL public key.
RUN \
    if grep -q libmysqlclient shlibdeps ; then \
       apt-get -q update && \
       DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates wget && \
       wget -q -O/etc/apt/trusted.gpg.d/mysql.asc https://repo.mysql.com/RPM-GPG-KEY-mysql-2023 && \
       echo "deb https://repo.mysql.com/apt/debian/ bookworm mysql-8.0" >> /etc/apt/sources.list && \
       rm -rf /var/lib/apt/lists/* ; \
    fi

# curl is used by scripts/prepare-spanner.sh and health checks
# jq is used by scripts/prepare-spanner.sh.
RUN \
    apt-get -q update && \
    DEBIAN_FRONTEND=noninteractive apt-get -q install -y --no-install-recommends $APT_INSTALL_ARGS $(cat shlibdeps) python3-pip curl jq && \
    rm -rf /var/lib/apt/lists/*

COPY --from=python_builder /app/wheels /app/wheels

RUN pip3 install --no-index --no-deps --break-system-packages /app/wheels/*.whl && \
    rm -r /app/wheels

COPY --from=builder /app/bin /app/bin
COPY --from=builder /app/syncserver/version.json /app/
COPY --from=builder /app/tools/spanner /app/tools/spanner
COPY --from=builder /app/tools/integration_tests /app/tools/integration_tests
COPY --from=builder /app/tools/tokenserver /app/tools/tokenserver
COPY --from=builder /app/scripts/prepare-spanner.sh /app/scripts/prepare-spanner.sh
COPY --from=builder /app/scripts/start_mock_fxa_server.sh /app/scripts/start_mock_fxa_server.sh
COPY --from=builder /app/syncstorage-spanner/src/schema.ddl /app/schema.ddl

RUN chmod +x /app/scripts/prepare-spanner.sh

USER app:app

ENTRYPOINT ["/app/bin/syncserver"]
