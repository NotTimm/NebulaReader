cask "nebula-reader" do
  arch arm: "arm64"

  version "0.2.2"
  # Run packaging/homebrew/update-cask.sh <version> to fill in the real SHA-256
  # before publishing this cask to your Homebrew tap.
  sha256 "REPLACE_WITH_SHA256_FROM_RELEASE_DMG"

  url "https://github.com/NotTimm/NebulaReader/releases/download/v#{version}/NebulaReader-#{version}-macos-#{arch}.dmg"
  name "Nebula Reader"
  desc "Comic, manga and ebook reader for local libraries, Jellyfin and OPDS"
  homepage "https://github.com/NotTimm/NebulaReader"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64
  depends_on macos: ">= :big_sur"

  app "Nebula Reader.app"

  zap trash: [
    "~/Library/Application Support/NebulaReader",
    "~/Library/Caches/NebulaReader",
  ]

  caveats <<~EOS
    Nebula Reader is not yet notarized with Apple. If macOS blocks the first
    launch, either install with:
      brew install --cask --no-quarantine NotTimm/tap/nebula-reader
    or run:
      xattr -dr com.apple.quarantine "/Applications/Nebula Reader.app"
  EOS
end
