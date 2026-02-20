# Releasing virgil-cli

## Steps

1. Update the version in `Cargo.toml`:
   ```toml
   [package]
   version = "X.Y.Z"
   ```

2. Commit the version bump:
   ```bash
   git add Cargo.toml
   git commit -m "Bump version to vX.Y.Z"
   ```

3. Create and push a tag:
   ```bash
   git tag release/vX.Y.Z
   git push origin master
   git push origin release/vX.Y.Z
   ```

## What happens

The `release.yml` workflow triggers on tags matching `release/v*` and:

- Builds binaries for 5 targets:
  | Target | Runner |
  |--------|--------|
  | x86_64-unknown-linux-gnu | ubuntu-latest |
  | aarch64-unknown-linux-gnu | ubuntu-24.04-arm |
  | x86_64-apple-darwin | macos-13 |
  | aarch64-apple-darwin | macos-latest |
  | x86_64-pc-windows-msvc | windows-latest |

- Packages as `.tar.gz` (Unix) or `.zip` (Windows)
- Generates `SHA256SUMS.txt`
- Creates a GitHub Release with auto-generated release notes

## Installing a release

Users can install with [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```bash
cargo binstall virgil-cli
```

Or download binaries directly from the [Releases](https://github.com/Delanyo32/virgil-cli/releases) page.
