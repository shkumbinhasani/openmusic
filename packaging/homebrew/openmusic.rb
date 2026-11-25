cask "openmusic" do
  arch arm: "arm64", intel: "x86_64"

  version "@VERSION@"
  sha256 arm:   "@SHA256_ARM64@",
         intel: "@SHA256_X86_64@"

  url "https://github.com/@REPOSITORY@/releases/download/v#{version}/OpenMusic-#{version}-#{arch}.zip"
  name "OpenMusic"
  desc "Spotify Connect player"
  homepage "https://github.com/@REPOSITORY@"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: :ventura

  app "OpenMusic.app"

  postflight_steps do
    run "/usr/bin/xattr",
        args:           ["-dr", "com.apple.quarantine", "{{appdir}}/OpenMusic.app"],
        writable_paths: ["OpenMusic.app"],
        writable_base:  :appdir
  end

  uninstall quit: "io.openmusic.player"

  zap script: {
        executable:   "/usr/bin/security",
        args:         ["delete-generic-password", "-s", "openmusic.spotify", "-a", "refresh-token"],
        must_succeed: false,
      },
      trash:  [
        "~/Library/Application Support/OpenMusic",
        "~/Library/Caches/OpenMusic",
        "~/Library/HTTPStorages/io.openmusic.player",
        "~/Library/Preferences/io.openmusic.player.plist",
        "~/Library/Saved Application State/io.openmusic.player.savedState",
      ]
end
