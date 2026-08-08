# ---- Proto compilation ----
FROM golang:1.24-bookworm AS proto

RUN apt-get update && apt-get install -y wget unzip && \
    wget -qO /tmp/protoc.zip https://github.com/protocolbuffers/protobuf/releases/download/v27.3/protoc-27.3-linux-x86_64.zip && \
    unzip /tmp/protoc.zip -d /usr/local && \
    rm /tmp/protoc.zip

WORKDIR /build
COPY backend/Cargo.toml backend/build.rs* backend/
COPY backend/proto/ backend/proto/
# Run the build.rs which compiles proto files via tonic-build
# (protoc is available for the Rust build script to invoke)

# ---- Frontend build ----
FROM node:24-alpine AS frontend

WORKDIR /frontend
COPY frontend/package.json frontend/package-lock.json* ./
RUN npm ci
COPY frontend/ .
RUN npm run build

# ---- Backend build ----
FROM rustlang/rust:1.97-bookworm AS backend

WORKDIR /build

# Copy proto-compiled sources or just rebuild everything with protoc available
COPY --from=proto /usr/local/bin/protoc /usr/local/bin/protoc
COPY --from=proto /usr/local/include /usr/local/include

# Copy Cargo manifests first for better caching
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/migrations/ migrations/

# Create a dummy src to resolve dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Now copy real source and build
COPY backend/src/ src/
COPY --from=proto /build/backend/target/ target/

RUN cargo build --release

# ---- Final minimal image ----
FROM scratch

# Add CA certificates for HTTPS
COPY --from=backend /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

WORKDIR /app

# Copy the compiled binary
COPY --from=backend /build/target/release/talos-control-system ./tcs

# Copy frontend static assets
COPY --from=frontend /frontend/build ./frontend-dist

# Copy database migrations
COPY backend/migrations ./migrations

# Cert volume for TLS
VOLUME /var/lib/tcs/certs

EXPOSE 80 443 8080 8082 9090

ENTRYPOINT ["/app/tcs"]
