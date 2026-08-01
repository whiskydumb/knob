set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

nightly := "nightly"
locked := ""

# list available recipes
default:
    @just --list

# format all code
fmt:
    rustup run {{nightly}} cargo fmt --all

# verify formatting without writing
fmt-check:
    rustup run {{nightly}} cargo fmt --all --check

# type-check the workspace
check:
    cargo check --workspace --all-targets {{locked}}

# lint the workspace, warnings are errors
clippy:
    cargo clippy --workspace --all-targets {{locked}} -- -D warnings

# formatting and lints
lint: fmt-check clippy

# the full gate
ci: lint check

# debug build with full symbols
dbg:
    cargo build --profile dbg {{locked}}

# optimized build
release:
    cargo build --release {{locked}}

# shippable build, fat LTO
dist:
    cargo build --profile dist {{locked}}

# build and open the documentation
doc:
    cargo doc --workspace --no-deps --open

# install the toolchain these recipes need
setup:
    rustup toolchain install {{nightly}} --profile minimal --component rustfmt

# remove build artifacts
clean:
    cargo clean
