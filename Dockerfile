###############################################################################
# Bundle CSS, JS, and static assets
###############################################################################
FROM node:18 as bundle

WORKDIR /usr/local/src/metagram

COPY package.json package-lock.json ./
RUN npm ci

COPY . .

RUN npm run build

###############################################################################
# Build License page template
###############################################################################
FROM rust:1.63 as licenses

ENV CARGO_ABOUT_VERSION='0.5.1'
ENV CARGO_ABOUT_PACKAGE="cargo-about-${CARGO_ABOUT_VERSION}-x86_64-unknown-linux-musl"
ENV CARGO_ABOUT_SHA256='b96621b6204e0027ab9cabc76df224e2b770f565aca8c70b62827e8629123d6c'
RUN curl --location "https://github.com/EmbarkStudios/cargo-about/releases/download/${CARGO_ABOUT_VERSION}/${CARGO_ABOUT_PACKAGE}.tar.gz" --output /tmp/cargo-about.tar.gz \
    && (cd /tmp && echo "${CARGO_ABOUT_SHA256}  cargo-about.tar.gz" | sha256sum --check) \
    && (cd /usr/local/bin && tar -zxvf /tmp/cargo-about.tar.gz --strip-components 1 "${CARGO_ABOUT_PACKAGE}/cargo-about")

WORKDIR /usr/local/src/metagram

# Copy in just enough to make `cargo metadata` work.
COPY Cargo.toml Cargo.lock ./
COPY src/bin src/bin

COPY about.toml about.toml
COPY templates/home/licenses.hbs templates/home/licenses.hbs

RUN cargo about generate \
    --output-file licenses.html \
    templates/home/licenses.hbs

###############################################################################
# Build the server
###############################################################################
FROM rust:1.63 as build

WORKDIR /usr/local/src/metagram

# Copy in just enough to make `cargo fetch` work.
COPY Cargo.toml Cargo.lock ./
COPY src/bin src/bin

RUN cargo fetch

COPY . .
COPY --from=licenses \
    /usr/local/src/metagram/licenses.html \
    /usr/local/src/metagram/licenses.html

ENV SQLX_OFFLINE=true

ARG METAGRAM_COMMIT_HASH
ENV METAGRAM_COMMIT_HASH="${METAGRAM_COMMIT_HASH:-development}"
RUN cargo build --release --bin server

###############################################################################
# Make the runnable image
###############################################################################
FROM rust:1.63

RUN cargo install squill --version 0.3.0

WORKDIR /usr/local/src/metagram

COPY migrations/ migrations/

COPY --from=bundle \
    /usr/local/src/metagram/dist \
    /usr/local/src/metagram/dist

COPY --from=build \
    /usr/local/src/metagram/target/release/server \
    /usr/local/bin/metagram-server

CMD [ "/usr/local/bin/metagram-server" ]
