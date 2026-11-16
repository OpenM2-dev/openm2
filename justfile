# openm2 task runner. Recipes are one-liners that work in pwsh (Windows) and sh (Linux),
# except `openm2`, which has a variant per system so arguments keep their quoting;
# anything bigger lives in xtask/ (Rust). CI runs `just check`.

set windows-shell := ["pwsh.exe", "-NoLogo", "-NoProfile", "-Command"]

# List the recipes
default:
    @just --list --unsorted

# Everything CI checks: fmt, clippy, tests, drift, check-deps, no-assets, snapshots, deny
check:
    cargo xtask ci

# Faster check: fmt, clippy and tests for the crates you changed, plus drift
quick:
    cargo xtask ci --quick

# Report missing tools and how to install them
preflight:
    cargo xtask preflight

# Format all code
fmt:
    cargo fmt --all

# Clippy with warnings as errors
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Run tests; extra args go to cargo test (e.g. `just test -p m2-protocol`)
test *args:
    cargo test --workspace --locked {{args}}

# Regenerate code and docs from crates/m2-protocol/packets/*.toml
gen:
    cargo xtask gen

# Fail if generated files are out of date
drift:
    cargo xtask drift

# Enforce the crate dependency direction and `publish = false`
check-deps:
    cargo xtask check-deps

# Fail if game assets, archive keys or large binaries are in the tree
no-assets:
    cargo xtask no-assets

# Advisories, licenses, bans and sources
deny:
    cargo deny check

# The third-party notices the Windows release ships with, into target/THIRD-PARTY-NOTICES.txt (--out FILE, --package NAME, --target TRIPLE)
notices *args:
    cargo xtask notices {{args}}

# Run the openm2 CLI (e.g. `just openm2 messages`). Arguments keep their quoting: game paths have spaces (`d:/ymir work/…`).
[windows]
[positional-arguments]
[script("pwsh.exe", "-NoLogo", "-NoProfile", "-File")]
[extension(".ps1")]
openm2 *args:
    cargo run --quiet --package m2-cli -- @args
    exit $LASTEXITCODE

# Run the openm2 CLI (e.g. `just openm2 messages`). Arguments keep their quoting: game paths have spaces (`d:/ymir work/…`).
[unix]
[positional-arguments]
openm2 *args:
    cargo run --quiet --package m2-cli -- "$@"

# Run openm2's own server (docs/server.md), e.g. `just server serve --memory --providers local/providers.toml` (players sign in only with those providers)
server *args:
    cargo run --quiet --package m2-server --bin openm2-server -- {{args}}

# Run openm2-launcher (docs/specs/update.md), e.g. `just launcher --dir local/install --update-only`
launcher *args:
    cargo run --quiet --package m2-launcher --bin openm2-launcher -- {{args}}

# Decode every file of your own client and check it: archives and proto tables, then every GR2 model, DDS texture, text script and map. Release build: a client is gigabytes.
client-check client keys="local/keys.toml":
    cargo run --release --quiet --package m2-cli -- vfs check --client "{{client}}" --keys "{{keys}}"
    cargo run --release --quiet --package m2-cli -- gr2 check --client "{{client}}" --keys "{{keys}}"
    cargo run --release --quiet --package m2-cli -- dds check --client "{{client}}" --keys "{{keys}}"
    cargo run --release --quiet --package m2-cli -- tga check --client "{{client}}" --keys "{{keys}}"
    cargo run --release --quiet --package m2-cli -- jpeg check --client "{{client}}" --keys "{{keys}}"
    cargo run --release --quiet --package m2-cli -- script check --client "{{client}}" --keys "{{keys}}"
    cargo run --release --quiet --package m2-cli -- map check --client "{{client}}" --keys "{{keys}}"
    cargo run --release --quiet --package m2-cli -- sound check --client "{{client}}" --keys "{{keys}}"
