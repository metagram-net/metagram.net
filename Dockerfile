###############################################################################
# Bundle CSS, JS, and static assets
###############################################################################
FROM node:18 as bundle

WORKDIR /usr/local/src/metagram

COPY package.json package-lock.json ./
RUN npm install

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

COPY . .

RUN cargo about generate \
    --output-file templates/home/licenses.html \
    templates/home/licenses.hbs

###############################################################################
# Build the server
###############################################################################
FROM rust:1.63 as build

WORKDIR /usr/local/src/metagram

COPY . .
COPY --from=bundle \
    /usr/local/src/metagram/dist \
    /usr/local/src/metagram/dist
COPY --from=licenses \
    /usr/local/src/metagram/templates/home/licenses.html \
    /usr/local/src/metagram/templates/home/licenses.html

ENV SQLX_OFFLINE=true
RUN cargo build --release --bin server

###############################################################################
# Make the runnable image
###############################################################################
FROM rust:1.63
COPY --from=build \
    /usr/local/src/metagram/target/release/server \
    /usr/local/bin/metagram-server
CMD [ "/usr/local/bin/metagram-server" ]
