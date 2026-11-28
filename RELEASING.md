# Releasing

`crates.io` does not pull from GitHub, and `docs.rs` builds every version
published to `crates.io`. A release therefore has exactly two steps:

1. push a git tag; the `release` workflow tests, publishes to crates.io, and
   creates a GitHub release;
2. docs.rs picks up the new crates.io version automatically and rebuilds the
   API docs (the `[package.metadata.docs.rs]` section controls the build).

A crate version is immutable. A docs-only change (README, `docs/`,
module docs) needs a new patch version before crates.io and docs.rs show it;
the GitHub copy updates as soon as the commit is merged.

## One-time setup

The workflow needs a crates.io API token with `publish-new` and
`publish-update` scope, stored as the repository secret
`CARGO_REGISTRY_TOKEN`:

- Web: **Settings → Secrets and variables → Actions → New repository secret**.
- CLI from a machine that already ran `cargo login`:

  ```sh
  sed -n 's/^token = "\(.*\)"/\1/p' ~/.cargo/credentials.toml \
    | gh secret set CARGO_REGISTRY_TOKEN
  ```

Alternatively, configure a
[trusted publisher](https://crates.io/settings/trusted-publishing) on the
crate (GitHub repository `lovelock/yar-rs`, workflow `release.yml`) and
replace the token with the `rust-lang/crates-io-auth-action` step; this
avoids storing a long-lived token.

## Automated release

```sh
# 1. bump the version in Cargo.toml (and Cargo.lock)
# 2. commit and merge to master
git tag v0.1.2
git push origin v0.1.2
```

The `release` workflow then:

- verifies that the tag (`vX.Y.Z`) matches `Cargo.toml`;
- runs `cargo test --all-features`;
- publishes with `cargo publish --registry crates-io` (skipping if that
  version is already on crates.io);
- creates a GitHub release with generated notes.

After crates.io accepts the version, docs.rs builds it automatically — no
webhook or extra workflow is needed. The Documentation link on crates.io is
`https://docs.rs/yar-rs`.

## Manual fallback

If the workflow cannot run, publish from a clean checkout on the tag:

```sh
cargo test --all-features
cargo publish --registry crates-io
```

docs.rs still builds automatically from the published version.
