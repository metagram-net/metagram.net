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

WORKDIR /usr/local/src/metagram

RUN cargo install cargo-about
COPY . .

RUN cargo about generate \
        --output-file templates/home/licenses.html \
        templates/home/licenses.hbs

###############################################################################
# Build the server
###############################################################################
FROM rust:1.63 as build

WORKDIR /usr/local/src/metagram

RUN cargo install cargo-make

COPY . .
COPY --from=bundle \
        /usr/local/src/metagram/dist \
        /usr/local/src/metagram/dist
COPY --from=licenses \
        /usr/local/src/metagram/templates/home/about.html \
        /usr/local/src/metagram/templates/home/about.html

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
