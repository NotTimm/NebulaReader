# Homebrew Cask (macOS)

`nebula-reader.rb` is a Homebrew Cask definition for installing the Apple Silicon
macOS build via a personal Homebrew tap (`NotTimm/homebrew-tap`).

## One-time setup

1. Create a public GitHub repository named `NotTimm/homebrew-tap`.
2. Fill in the real SHA-256 for the latest published release:
   ```bash
   bash packaging/homebrew/update-cask.sh 0.2.2
   ```
3. Copy `packaging/homebrew/nebula-reader.rb` into `Casks/nebula-reader.rb` in
   `NotTimm/homebrew-tap` and push.

Users on Apple Silicon Macs can then install and update with:

```bash
brew tap NotTimm/tap
brew install --cask --no-quarantine NotTimm/tap/nebula-reader
```

## Updating on each release

After a new GitHub Release publishes `NebulaReader-<version>-macos-arm64.dmg`:

```bash
bash packaging/homebrew/update-cask.sh <version>
```

Then copy the updated `nebula-reader.rb` to `Casks/nebula-reader.rb` in
`NotTimm/homebrew-tap` (or automate that step from `.github/workflows/release.yml`
with a PAT scoped to the tap repo).

On a Mac, you can validate the cask before pushing:

```bash
brew audit --cask --strict NotTimm/tap/nebula-reader
```
