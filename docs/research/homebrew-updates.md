# Homebrew install and updates for OpenMusic

Research date: 2026-11-23. Homebrew was at [7.0.16](https://github.com/Homebrew/brew/releases/tag/7.0.16), released that day. I read the Homebrew docs from their Markdown source in [`Homebrew/brew/docs`](https://github.com/Homebrew/brew/tree/main/docs). Those files are what docs.brew.sh serves, and the links below point to docs.brew.sh. Source code links are pinned to Homebrew/brew commit [`738cf5a`](https://github.com/Homebrew/brew/tree/738cf5a40801e6953477fb425f61778af318dbd4) and Sparkle commit [`e13374c`](https://github.com/sparkle-project/Sparkle/tree/e13374c6820d9139ec2fcb7fd7dab76ba33c4b21).

**[observed]** marks something I checked on this machine. **Unconfirmed** marks a claim with no primary source.

## Summary and recommendation

**Adopt now**

1. Publish a cask named `openmusic` in our own tap, `<owner>/homebrew-tap`, which already holds the owner's other casks, not in `homebrew/cask`. `homebrew/cask` now rejects apps that fail Gatekeeper, and its failing casks carry a disable date of 2026-09-01. OpenMusic is not notarized, and we assume its use of Spotify's private APIs rules it out anyway.
2. Point the cask at the GitHub Release zips (arm64 and x86_64). Use `livecheck` with `strategy :github_latest`. Leave out `auto_updates`, because the app has no updater.
3. Have the release workflow write the new `version` and both `sha256` values into the tap and push. Authenticate with a fine-grained token, a deploy key or a GitHub App token, each limited to the tap repo. The workflow's own `GITHUB_TOKEN` cannot push to another repository.
4. Tell users to run `brew install --cask <owner>/tap/openmusic`. That command also trusts this one cask, which Homebrew 6.0 and later require for third-party taps. Users then update with `brew upgrade`.
5. Expect a Gatekeeper block on first launch. Homebrew always quarantines cask downloads, and `--no-quarantine` no longer exists. Users must click **Open Anyway** in System Settings once. Put that step in a `caveats` stanza and in the README.
6. Sign release builds with the same certificate every time, not ad hoc. Homebrew copies the user's approval to the new version only if the new app meets the old app's designated requirement. An ad-hoc requirement is a list of code hashes, so it changes with every build, and users would be blocked again after every upgrade.
7. Do not add Sparkle yet. If we want an in-app notice, have the app check GitHub's "latest release" endpoint and tell the user to run `brew upgrade --cask openmusic`.

**Add once Developer ID signing and notarization exist**

1. Sign with Developer ID, hardened runtime and a secure timestamp. Notarize the zip, staple the app, then zip it again. The repo's `release.yml` already does this when its secrets exist.
2. Store these GitHub secrets: the Developer ID `.p12` (base64) and its password, plus the App Store Connect API key (`.p8`, base64), key ID and issuer ID. Keep the tap token too.
3. Then consider Sparkle 2, and set `auto_updates true` in the cask when the app ships it. Sparkle needs an EdDSA key pair. Keep the private key in a secret and pass it to Sparkle's tools on standard input.
4. Even signed and notarized, `homebrew/cask` acceptance would still depend on notability and on the maintainers' "project risk" judgement. Stay in our own tap.

## 1. Own tap or `homebrew/cask`

### Rules for `homebrew/cask`

- The shared rules require public interest beyond the author. For a GitHub project, that means at least 30 forks, 30 watchers or 75 stars. A self-submission by the repository owner needs 90 forks, 90 watchers or 225 stars. A repository less than 30 days old is normally not eligible. ([Package Acceptance Policy: Notability](https://docs.brew.sh/Package-Acceptance-Policy#notability))
- A new package needs a working `livecheck`. ([Package Acceptance Policy: Public presence and maintenance](https://docs.brew.sh/Package-Acceptance-Policy#public-presence-and-maintenance))
- "Homebrew may reject or remove a package when carrying it creates a material legal, infrastructure, safety or project-continuity risk." ([Package Acceptance Policy: Project risk](https://docs.brew.sh/Package-Acceptance-Policy#project-risk)) This is the closest written rule to our Spotify concern. No page I read names private or unofficial APIs. That OpenMusic would be refused on this ground is our assumption, not a stated rule.
- A cask that supports macOS must work on the latest major macOS. Its apps "must pass Homebrew's Gatekeeper checks and must not require System Integrity Protection or Gatekeeper to be disabled or bypassed." ([Acceptable Casks: Platform compatibility and macOS security protections](https://docs.brew.sh/Acceptable-Casks#platform-compatibility-and-macos-security-protections))
- The download must come from the developer or a source the developer endorses. ([Acceptable Casks: Verifiable upstream distribution](https://docs.brew.sh/Acceptable-Casks#verifiable-upstream-distribution)) Casks in `homebrew/cask` also need a `zap` stanza. ([Cask Cookbook: Required stanzas](https://docs.brew.sh/Cask-Cookbook#required-stanzas))
- Software that fails these rules "can generally be maintained in a third-party tap", and that tap carries no Homebrew endorsement. ([Package Acceptance Policy: Scope and third-party taps](https://docs.brew.sh/Package-Acceptance-Policy#scope-and-third-party-taps))
- Precedent: `homebrew/cask` did carry `psst`, another third-party Spotify client. It is now `disable! date: "2026-09-01", because: :fails_gatekeeper_check`. ([Casks/p/psst.rb](https://github.com/Homebrew/homebrew-cask/blob/d5ed86e4ac8c891cac60a679cd12064c9c47c397/Casks/p/psst.rb)) So the cask that was removed failed on Gatekeeper, not on its use of Spotify.

### The 2025–2026 Gatekeeper policy

| Date | Change | Source |
|---|---|---|
| 2025-07-22 | The codesign audit runs on every `homebrew/cask` cask, not only new ones. | [PR #20286](https://github.com/Homebrew/brew/pull/20286) |
| 2025-08-05 | 4.6.0: "All casks submitted in Homebrew/homebrew-cask must be signed." | [4.6.0 notes](https://brew.sh/2025/08/05/homebrew-4.6.0/) |
| 2025-11-12 | 5.0.0: "Casks without codesigning are deprecated. We will disable all Homebrew/homebrew-cask casks that fail Gatekeeper checks in September 2026." `--no-quarantine` and `--quarantine` deprecated. | [5.0.0 notes](https://brew.sh/2025/11/12/homebrew-5.0.0/), [PR #20973](https://github.com/Homebrew/brew/pull/20973) |
| 2026-03-10 | 5.1.0: `--[no-]quarantine` changed from deprecated to disabled. | [PR #21629](https://github.com/Homebrew/brew/pull/21629) (merged 2026-03-02) |
| 2026-06-11 | 6.0.0: the flag was removed. The release notes say Gatekeeper-failing casks "remain on track to be disabled in September 2026". | [PR #22549](https://github.com/Homebrew/brew/pull/22549), [6.0.0 notes](https://brew.sh/2026/06/11/homebrew-6.0.0/) |
| 2026-09-01 | Such casks carry `disable! date: "2026-09-01", because: :fails_gatekeeper_check`. | e.g. [psst.rb](https://github.com/Homebrew/homebrew-cask/blob/d5ed86e4ac8c891cac60a679cd12064c9c47c397/Casks/p/psst.rb) |
| 2026-09-13 | 7.0.0: `brew audit --signing` and `--no-signing` disabled. The audit decides when to check signing. | [7.0.0 migration guide](https://brew.sh/7.0.0-migration-guide/) |

What happens to a failing cask: it is deprecated, then disabled, then removed once it has been disabled for over a year. Popular casks get at least six months of deprecation first. ([Deprecating, Disabling and Removing: casks](https://docs.brew.sh/Deprecating-Disabling-and-Removing#when-to-deprecate-casks)) Removal "does not mean Homebrew has determined that the software is malware". The cask "can become eligible again when upstream provides artefacts that pass the checks." ([Security and Supply Chain: Casks have a different trust model](https://docs.brew.sh/Homebrew-Security-and-Supply-Chain#casks-have-a-different-trust-model), [FAQ](https://docs.brew.sh/FAQ#why-was-a-cask-disabled-or-removed-after-a-macos-security-check))

Why: Homebrew cannot rebuild cask binaries, so it treats Developer ID signing, notarization and Gatekeeper as an independent check. "Homebrew applies macOS quarantine attributes to Cask downloads so Gatekeeper performs these checks instead of Homebrew bypassing them." ([Security and Supply Chain](https://docs.brew.sh/Homebrew-Security-and-Supply-Chain#casks-have-a-different-trust-model)) The issue that proposed removing `--no-quarantine` cites Apple's Sequoia change and Apple Silicon's need for a valid signature. ([issue #20755](https://github.com/Homebrew/brew/issues/20755))

### Does it apply to third-party taps?

The acceptance rule and the September 2026 disable cover `homebrew/cask` only. ([Acceptable Casks](https://docs.brew.sh/Acceptable-Casks), [5.0.0 notes](https://brew.sh/2025/11/12/homebrew-5.0.0/)) The audit code agrees. `audit_signing` returns early when the tap is not official and signing was not requested, and only `--new` requests it. ([cask/audit.rb L640–L651](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/audit.rb#L640-L651), [L66–L79](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/audit.rb#L66-L79)) A plain `brew audit` on our tap skips the signing check.

Our tap still feels three effects:

1. **Quarantine is always on.** Every cask download is quarantined ([cask/download.rb L78](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/download.rb#L78), [L253–L254](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/download.rb#L253-L254)), and no flag turns it off. Gatekeeper blocks an app that is not notarized on first launch. Since macOS Sequoia, Control-click no longer overrides Gatekeeper; users must go to System Settings > Privacy & Security. ([Apple, "Updates to runtime protection in macOS Sequoia"](https://developer.apple.com/news/?id=saqachfa)) There they click **Open Anyway** within about an hour of the blocked launch and enter their password. ([Apple Support: Open a Mac app from an unknown developer](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unknown-developer-mh40616/mac))
2. **Tap trust.** Since 6.0.0, non-official taps must be trusted before Homebrew loads their code. Installing by full name (`brew install --cask user/repo/cask`) trusts only that cask. ([Tap Trust](https://docs.brew.sh/Tap-Trust#installing-from-a-tap), [6.0.0 notes](https://brew.sh/2026/06/11/homebrew-6.0.0/))
3. **Approval carries over only when the signer matches.** On upgrade, Homebrew copies the user's Gatekeeper approval to the new app only if the new app meets the old app's designated requirement. ([cask/upgrade.rb L321–L345](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/upgrade.rb#L321-L345), [extend/os/mac/cask/quarantine.rb L39–L54](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/extend/os/mac/cask/quarantine.rb#L39-L54), [PR #23060](https://github.com/Homebrew/brew/pull/23060), [PR #23546](https://github.com/Homebrew/brew/pull/23546)) Otherwise it warns: "signer changed so macOS may prompt at next launch." ([upgrade.rb L486–L487](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/upgrade.rb#L486-L487))
   - **[observed]** The local build signed with Apple Development has the requirement `identifier "io.openmusic.player" and anchor apple generic and certificate leaf[subject.CN] = "Apple Development: … (TEAMID)" and …`. That stays the same from build to build while the certificate name stays the same. `spctl --assess` rejects this build.
   - **[observed]** An ad-hoc signature (`codesign -s -`) produces a requirement of the form `cdhash H"…" or cdhash H"…"`. It changes with every build, so each upgrade would block the user again.
   - **Unconfirmed:** whether a renewed Apple Development certificate keeps the same leaf common name. I found no Apple page on this.

### Status of `--no-quarantine`

Gone. Deprecated in 5.0.0, disabled in 5.1.0, removed in 6.0.0. The leftover code was deleted on 2026-07-30. (Sources in the table above; [PR #23363](https://github.com/Homebrew/brew/pull/23363).) No `quarantine` option appears in the current [Manpage](https://docs.brew.sh/Manpage).

## 2. The cask

### Stanzas

| Stanza | What to write | Source |
|---|---|---|
| `arch` | `arch arm: "arm64", intel: "x86_64"`, to fill the arch part of the zip name. | [Cookbook: Handling different system configurations](https://docs.brew.sh/Cask-Cookbook#handling-different-system-configurations) |
| `version` | The tag without `v`, e.g. `"0.2.0"`. `url` interpolates it. | [Cookbook: Stanza `version`](https://docs.brew.sh/Cask-Cookbook#stanza-version) |
| `sha256` | `sha256 arm: "…", intel: "…"`, one checksum per zip. | [Cookbook: Handling different system configurations](https://docs.brew.sh/Cask-Cookbook#handling-different-system-configurations) |
| `url` | The GitHub release asset URL. HTTPS is preferred. Do not add `verified:`, which 7.0.0 deprecated ("Default URL verification"). | [Cookbook: Stanza `url`](https://docs.brew.sh/Cask-Cookbook#stanza-url), [7.0.0 migration guide](https://brew.sh/7.0.0-migration-guide/) |
| `name`, `desc`, `homepage` | Required. `desc` is one neutral line with no marketing words. | [Cookbook: Required stanzas](https://docs.brew.sh/Cask-Cookbook#required-stanzas), [Description Guidelines](https://docs.brew.sh/Description-Guidelines) |
| `livecheck` | `url :url` plus `strategy :github_latest` (see below). | [Brew Livecheck](https://docs.brew.sh/Brew-Livecheck#general-guidelines) |
| `auto_updates` | Leave it out today. It means the app downloads and installs its own updates, not that it only opens a web page. | [Cookbook: Optional stanzas](https://docs.brew.sh/Cask-Cookbook#optional-stanzas) |
| `depends_on macos:` | A symbol, e.g. `depends_on macos: :ventura` for `LSMinimumSystemVersion` 13.0, or `:tahoe` if the minimum becomes 26. Use bare `depends_on :macos` if the minimum is Big Sur or older. The string form `">= :ventura"` is deprecated. | [Cookbook: `depends_on` macos](https://docs.brew.sh/Cask-Cookbook#depends_on-macos), [macos_requirement.rb L33–L63](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/requirements/macos_requirement.rb#L33-L63), [macos_version.rb L22–L26](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/macos_version.rb#L22-L26) |
| `app` | `app "OpenMusic.app"` | [Cookbook: Stanza `app`](https://docs.brew.sh/Cask-Cookbook#stanza-app) |
| `uninstall quit:` | `"io.openmusic.player"`. It quits the app during `uninstall`, `upgrade` and `reinstall`. Since 6.0.0, Homebrew reopens apps it closed during an upgrade. | [Cookbook: `uninstall` quit](https://docs.brew.sh/Cask-Cookbook#uninstall-quit), [6.0.0 notes](https://brew.sh/2026/06/11/homebrew-6.0.0/) |
| `zap` | `trash:` for files under `~/Library`. `script:` runs `/usr/bin/security` to delete the Keychain item. | [Cookbook: Stanza `zap`](https://docs.brew.sh/Cask-Cookbook#stanza-zap), [Cookbook: `uninstall` script](https://docs.brew.sh/Cask-Cookbook#uninstall-script) |
| `caveats` | A short note about the first-launch Gatekeeper step. | [Cookbook: Stanza `caveats`](https://docs.brew.sh/Cask-Cookbook#stanza-caveats) |

Notes:

- **Livecheck.** Without a block, livecheck reduces any github.com URL to the repo's Git URL and reads tags. ([livecheck/strategy/git.rb L88–L92](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/livecheck/strategy/git.rb#L88-L92)) `GithubLatest` has priority 0, so it runs only when named. ([github_latest.rb L13–L40](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/livecheck/strategy/github_latest.rb#L13-L40)) The docs allow `GithubLatest` when "there's a notable time gap between a tag and release". ([Brew Livecheck: General guidelines](https://docs.brew.sh/Brew-Livecheck#general-guidelines)) Our tag exists before the release job uploads the zips, so the tag strategy would report a version that has no download yet. GitHub's "latest" release is "the most recent non-prerelease, non-draft release". ([GitHub REST: Get the latest release](https://docs.github.com/en/rest/releases/releases#get-the-latest-release))
- **Keychain.** The Cookbook offers no Keychain directive for generic passwords. The only Keychain step deletes certificates (`delete_keychain_certificates`). ([Cookbook: `*flight_steps`](https://docs.brew.sh/Cask-Cookbook#stanza-flight_steps)) `homebrew/cask` has a precedent for calling `security` from `zap script:`: `openvpn-connect` runs `security delete-keychain …` with `must_succeed: false`. ([Casks/o/openvpn-connect.rb](https://github.com/Homebrew/homebrew-cask/blob/d5ed86e4ac8c891cac60a679cd12064c9c47c397/Casks/o/openvpn-connect.rb)) The 7.0 sandbox applies to `*_steps` blocks ([PR #23461](https://github.com/Homebrew/brew/pull/23461)), not to `uninstall`/`zap` `script:`. The app stores its token under service `openmusic.spotify`, account `refresh-token` ([src/worker/keychain.rs](../../src/worker/keychain.rs)). **Unconfirmed:** I have not run this `zap` against a real install.
- **Zap paths.** The app writes `~/Library/Caches/OpenMusic` ([src/worker/cache.rs](../../src/worker/cache.rs)). **[observed]** This machine also has `~/Library/Preferences/io.openmusic.player.plist` and `~/Library/HTTPStorages/io.openmusic.player`. To be sure, run `brew generate-zap openmusic` after installing and using the app. ([Cookbook: `zap` creation](https://docs.brew.sh/Cask-Cookbook#zap-creation))
- **Name clash.** Cask tokens must be unique across taps. ([How to Create and Maintain a Tap: Naming](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap#naming)) `homebrew/cask` has no `openmusic` cask today (`Casks/o/openmusic.rb` returns 404).
- **Repo draft.** `packaging/homebrew/openmusic.rb` uses `depends_on macos: ">= :ventura"`. Homebrew prints a deprecation warning for that form and suggests `depends_on macos: :ventura`. ([macos_requirement.rb L46–L53](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/requirements/macos_requirement.rb#L46-L53))
- **Intel.** Homebrew moved Intel macOS to Tier 3 in 7.0.0, and it stops running on Intel in September 2027. ([7.0.0 notes](https://brew.sh/2026/09/13/homebrew-7.0.0/)) A third-party tap may still ship an Intel zip.

### Example: `Casks/openmusic.rb` in `<owner>/homebrew-tap`

`<owner>` and the app repository name `openmusic` are placeholders.

```ruby
cask "openmusic" do
  arch arm: "arm64", intel: "x86_64"

  version "0.2.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000",
         intel: "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/<owner>/openmusic/releases/download/v#{version}/OpenMusic-#{version}-#{arch}.zip"
  name "OpenMusic"
  desc "Spotify Connect player"
  homepage "https://github.com/<owner>/openmusic"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: :ventura

  app "OpenMusic.app"

  uninstall quit: "io.openmusic.player"

  zap script: {
        executable:   "/usr/bin/security",
        args:         ["delete-generic-password", "-s", "openmusic.spotify", "-a", "refresh-token"],
        must_succeed: false,
      },
      trash:  [
        "~/Library/Caches/OpenMusic",
        "~/Library/HTTPStorages/io.openmusic.player",
        "~/Library/Preferences/io.openmusic.player.plist",
        "~/Library/Saved Application State/io.openmusic.player.savedState",
      ]

  caveats <<~EOS
    OpenMusic is not notarized by Apple, so macOS blocks its first launch.
    Open it once, then go to System Settings > Privacy & Security and click Open Anyway.
  EOS
end
```

Users install with:

```sh
brew install --cask <owner>/tap/openmusic
```

Run `brew style --fix <owner>/tap` and `brew audit --cask --online <owner>/tap/openmusic` on the tap before publishing. `brew style` enforces stanza order and layout. ([Cookbook: Stanza order](https://docs.brew.sh/Cask-Cookbook#stanza-order))

## 3. How `outdated` and `upgrade` treat casks

Homebrew 7.0.0 changed the default for `auto_updates true` casks. The table describes 7.0.x.

| Cask | `brew outdated` / `brew upgrade` (default) | `--greedy` | Source |
|---|---|---|---|
| Versioned, no `auto_updates` (our case today) | Listed and upgraded when the tap version differs from the installed version. | Same. | [cask.rb L501–L521](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/cask.rb#L501-L521) |
| Versioned, `auto_updates true` | Listed and upgraded only when the installed app's bundle version looks older than the tap version. Skipped when the app is the same or newer, or when Homebrew cannot compare. `HOMEBREW_NO_UPGRADE_AUTO_UPDATES_CASKS=1` turns this off. | Listed and upgraded whenever the tap version differs from Homebrew's install record. `--greedy-auto-updates` does this for these casks alone. | [FAQ](https://docs.brew.sh/FAQ#how-does-brew-upgrade-handle-apps-that-update-themselves), [cask.rb L513–L518](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/cask.rb#L513-L518), [cask.rb L884+](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/cask.rb#L884-L940), [Manpage](https://docs.brew.sh/Manpage) (`outdated`, `upgrade`) |
| `version :latest` (needs `sha256 :no_check`) | Never listed; skipped. | Included, also with `--greedy-latest`. Homebrew downloads the artifact and reinstalls only if its SHA-256 differs from the one recorded at install. | [FAQ](https://docs.brew.sh/FAQ#how-does-brew-upgrade-handle-apps-that-update-themselves), [Cookbook: Special value `:latest`](https://docs.brew.sh/Cask-Cookbook#special-value-latest), [cask.rb L505–L509](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/cask.rb#L505-L509) |

More detail:

- History of the `auto_updates` default: up to 5.x, the default skipped these casks unless `--greedy`. In 6.0.0 the bundle-version check arrived behind `HOMEBREW_UPGRADE_AUTO_UPDATES_CASKS` ([PR #21985](https://github.com/Homebrew/brew/pull/21985), [6.0.0 notes](https://brew.sh/2026/06/11/homebrew-6.0.0/)). In 7.0.0 it became the default, the opt-in variable was disabled, and `HOMEBREW_NO_UPGRADE_AUTO_UPDATES_CASKS` opts out. ([7.0.0 migration guide](https://brew.sh/7.0.0-migration-guide/), [env_config.rb L663–L667, L766–L775](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/env_config.rb#L663-L775))
- `HOMEBREW_UPGRADE_GREEDY=1` applies `--greedy` to every cask upgrade. `HOMEBREW_UPGRADE_GREEDY_CASKS` applies it to the casks it lists. `brew outdated` honours the latter since 5.1.0. ([Manpage](https://docs.brew.sh/Manpage), [5.1.0 notes](https://brew.sh/2026/03/10/homebrew-5.1.0/))
- Naming a cask (`brew upgrade --cask openmusic`) also includes casks the default would skip. ([FAQ](https://docs.brew.sh/FAQ#how-does-brew-upgrade-handle-apps-that-update-themselves))
- `version :latest` casks are never autobumped ([Cookbook: Special value `:latest`](https://docs.brew.sh/Cask-Cookbook#special-value-latest)). Users would need `--greedy` to get updates. We publish versioned releases, so it does not suit us.
- For OpenMusic, `CFBundleShortVersionString` is the release version and `CFBundleVersion` is the CI run number ([scripts/bundle-macos.sh](../../scripts/bundle-macos.sh)). Because the cask version has no commas, the bundle check compares the short version with the cask version. ([cask.rb L884–L940](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/cask.rb#L884-L940))

## 4. Automating the bump

### `brew bump-cask-pr`

- **What it does.** It edits `version`, `url` and `sha256` and opens a pull request. Without `--sha256`, it downloads the new file to compute the checksum. For a cask with `arch`, it does this for each arch. ([Manpage](https://docs.brew.sh/Manpage) `bump-cask-pr`, [bump-cask-pr.rb L303–L390](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/dev-cmd/bump-cask-pr.rb#L303-L390)) It also runs `brew audit` and `brew style --fix` unless told not to.
- **Requirements.** The cask must sit in a tap that is a Git repository with a remote. ([bump-cask-pr.rb L97–L100](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/dev-cmd/bump-cask-pr.rb#L97-L100), [bump.rb L56–L60](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/bump.rb#L56-L60)) The runner needs Homebrew installed, with the tap tapped and trusted.
- **Third-party taps.** It works. The "autobump list" refusal applies only to official taps. ([tap.rb L1406–L1408](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/tap.rb#L1406-L1408))
- **Forks.** By default it forks the tap through the GitHub API, pushes a branch to the fork and opens a PR from it. `--no-fork` pushes the branch to the tap's own `origin`. `--fork-org` picks an organisation for the fork. ([bump.rb L66–L78, L124–L134](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/bump.rb#L66-L134)) `--write-only` edits the file without Git. `--write-only --commit` also commits but neither pushes nor opens a PR. ([Manpage](https://docs.brew.sh/Manpage))
- **Token.** It reads `HOMEBREW_GITHUB_API_TOKEN`. When set, Homebrew puts the token into the HTTPS push URL. ([bump.rb L144–L147](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/bump.rb#L144-L147), [Manpage: `HOMEBREW_GITHUB_API_TOKEN`](https://docs.brew.sh/Manpage)) For our setup the token needs write access to the tap's contents, plus pull requests unless we use `--write-only`.
- **Fit.** It is heavier than we need. It installs Homebrew in CI, downloads both zips again and runs the audit. Its gain is that it reuses Homebrew's own file editing and checks.

### Committing straight to the tap

The repo's draft `release.yml` already does this. It checks out the tap with a token, renders the cask from a template and pushes. Options for the credential:

- **`GITHUB_TOKEN` will not work.** "The token's permissions are limited to the repository that contains your workflow." ([GitHub Docs: GITHUB_TOKEN](https://docs.github.com/en/actions/concepts/security/github_token))
- **Fine-grained personal access token.** Limit it to the tap repository with Contents read and write. GitHub recommends fine-grained tokens over classic ones and recommends setting an expiry. Classic tokens reach every repository the user can. ([GitHub Docs: Managing your personal access tokens](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens))
- **Deploy key with write access.** An SSH key for one repository. It does not expire and is usually stored without a passphrase. GitHub recommends a GitHub App for finer control. ([GitHub Docs: Managing deploy keys](https://docs.github.com/en/authentication/connecting-to-github-with-ssh/managing-deploy-keys))
- **GitHub App installation token.** GitHub recommends a GitHub App "to access resources on behalf of an organization, or for long-lived integrations". ([GitHub Docs: Managing your personal access tokens](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)) [`actions/create-github-app-token`](https://github.com/actions/create-github-app-token) mints the token in a workflow; its latest release is `v3.4.0` (2026-11-18).

A direct commit has no review step. Pushes made with `GITHUB_TOKEN` do not start new workflow runs. ([GitHub Docs: GITHUB_TOKEN](https://docs.github.com/en/actions/concepts/security/github_token)) With a PAT or app token, a `tests.yml` in the tap would run on the push.

### Maintained GitHub Actions

| Action | Status on 2026-11-23 | Fit |
|---|---|---|
| [`Homebrew/actions/bump-packages`](https://github.com/Homebrew/actions/tree/main/bump-packages) | Maintained. Latest release [`2026.11.23.1`](https://github.com/Homebrew/actions/releases/tag/2026.11.23.1). 7.0.0 removed `@master`; pin a CalVer release or full SHA. ([7.0.0 notes](https://brew.sh/2026/09/13/homebrew-7.0.0/)) | Runs `brew bump --open-pr --casks …`, which uses livecheck to find the new version. Its `token` input says "GitHub token (not the default one)". `fork` defaults to true outside the Homebrew org. ([action.yml](https://github.com/Homebrew/actions/blob/main/bump-packages/action.yml)) Suits a schedule better than a release trigger. |
| [`mislav/bump-homebrew-formula-action`](https://github.com/mislav/bump-homebrew-formula-action) | Maintained. Latest release `v4.2` (2026-07-12). | Edits files through the GitHub API without Homebrew. Its README lists "Limited support for bumping Homebrew casks". It sets one `sha256`, and our cask has one per arch. Not a good fit. |
| [`macauley/action-homebrew-bump-cask`](https://github.com/macauley/action-homebrew-bump-cask) | Last release `v1.0.0` (2021-12-30), last push 2021. Treat as unmaintained. | Do not use. |

### `brew tap-new` workflows and bottles

`brew tap-new` writes `.github/dependabot.yml` and three workflows. ([dev-cmd/tap-new.rb L83–L96](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/dev-cmd/tap-new.rb#L83-L96))

- `tests.yml` ("brew test-bot") runs on pushes and PRs. Its tap-syntax step runs `brew style`, `brew readall` and `brew audit` over the tap's formulae and casks ([test_bot/tap_syntax.rb L23–L33](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/test_bot/tap_syntax.rb#L23-L33)). Its other steps build and upload formula bottles. ([template](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/.github/workflows/tap-new-tests.yml))
- `publish.yml` ("brew pr-pull") is started by hand to publish bottles from a reviewed PR. Since 7.0.0 it also writes build attestations. ([template](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/.github/workflows/tap-new-publish.yml), [7.0.0 notes](https://brew.sh/2026/09/13/homebrew-7.0.0/))
- `autobump.yml` ("brew bump"), new in 7.0.0, runs daily at a random time. It calls `brew bump --no-fork --open-pr --tap=…` with `GITHUB_TOKEN` and opens PRs for new versions that livecheck finds. ([template](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/.github/workflows/tap-new-autobump.yml), [7.0.0 notes](https://brew.sh/2026/09/13/homebrew-7.0.0/))

Bottles are for formulae: "Bottles are produced by installing a formula with `brew install --build-bottle`…" ([Bottles](https://docs.brew.sh/Bottles)) A cask installs the upstream artifact. ([Manpage: Terminology](https://docs.brew.sh/Manpage)) A cask-only tap needs no bottles, so we can delete `publish.yml`. Keep the tap-syntax part of `tests.yml`. `autobump.yml` can serve as a fallback that catches a release the release workflow failed to push.

## 5. Sparkle

### How Sparkle 2 works

- **Appcast.** The app's `Info.plist` names an RSS feed in `SUFeedURL`. Each item lists a version and a download. `CFBundleVersion` must increase from release to release, because Sparkle compares it. ([Sparkle: Basic Setup, step 5](https://sparkle-project.org/documentation/#5-publish-your-appcast)) `generate_appcast` builds the feed from a folder of archives and signs it. ([same page](https://sparkle-project.org/documentation/#5-publish-your-appcast)) Sparkle reads zip archives. ([Basic Setup, step 4](https://sparkle-project.org/documentation/#4-distributing-your-app))
- **EdDSA keys.** `generate_keys` creates an ed25519 key pair and stores the private key in the login Keychain. The public key goes in `SUPublicEDKey`. ([Basic Setup, step 3](https://sparkle-project.org/documentation/#eddsa-ed25519-signatures)) In CI, pass the private key on standard input (`--ed-key-file -`) rather than as an argument. ([Security and reliability: 2.2.2](https://sparkle-project.org/documentation/security-and-reliability/))
- **Validation.** Sparkle accepts an update if either the EdDSA signature is valid or the new app's code signature matches the old app's. "At least one method of signature verification must be valid. The update will be rejected." ([SUUpdateValidator.m L386–L396](https://github.com/sparkle-project/Sparkle/blob/e13374c6820d9139ec2fcb7fd7dab76ba33c4b21/Sparkle/SUUpdateValidator.m#L386-L396)) EdDSA alone therefore works for an app without a Developer ID. Sparkle still recommends Developer ID signing and notarization "if possible". ([Basic Setup, step 3](https://sparkle-project.org/documentation/#3-segue-for-security-concerns))
- **Key rotation** needs both Developer ID signing and EdDSA. You can change one of the two in a release, not both. ([Rotating signing keys](https://sparkle-project.org/documentation/#rotating-signing-keys))
- **Versions.** The current release is [2.10.1](https://github.com/sparkle-project/Sparkle/releases/tag/2.10.1) (2026-10-25). The 2.x line needs macOS 12.0 or later. ([README](https://github.com/sparkle-project/Sparkle/blob/e13374c6820d9139ec2fcb7fd7dab76ba33c4b21/README.markdown))

### With `auto_updates true`

- A cask for an app that updates itself should declare `auto_updates true`. ([Cookbook: Optional stanzas](https://docs.brew.sh/Cask-Cookbook#optional-stanzas), [FAQ](https://docs.brew.sh/FAQ#how-does-brew-upgrade-handle-apps-that-update-themselves))
- From 7.0.0, `brew upgrade` replaces such an app only when the installed bundle is older than the tap version. Homebrew does not downgrade an app that Sparkle has already updated. (Section 3.)
- An app's own updater leaves no quarantine attribute on the files it installs. Homebrew 7 treats such an app as approved on its next upgrade, after checking the signer. ([PR #23556](https://github.com/Homebrew/brew/pull/23556), [upgrade.rb L327–L331](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/upgrade.rb#L327-L331))
- The cask's livecheck can read the appcast with `strategy :sparkle`. ([Brew Livecheck: `Sparkle` strategy block](https://docs.brew.sh/Brew-Livecheck#sparkle-strategy-block)) `brew audit --online` then also compares `minimumSystemVersion` from the feed with `depends_on macos:`. ([Cookbook: Choosing the minimum macOS release](https://docs.brew.sh/Cask-Cookbook#choosing-the-minimum-macos-release))

### Sparkle in a non-Xcode Rust app

Sparkle documents setups that do not use Xcode. Its Qt example says it "can also be used as reference when integrating Sparkle into other non-Apple provided toolkits". The steps it lists ([Programmatic setup: Create an Updater in Qt](https://sparkle-project.org/documentation/programmatic-setup/#create-an-updater-in-qt)):

- link with `-framework Sparkle` and set the framework search path (`-F`);
- add an rpath so the binary finds `Contents/Frameworks` (the manual-install steps give `-Wl,-rpath,@loader_path/../Frameworks` for non-Xcode projects, [Basic Setup, step 1](https://sparkle-project.org/documentation/#1-add-the-sparkle-framework-to-your-project));
- copy `Sparkle.framework` into `Contents/Frameworks/` and keep its symlinks;
- set `CFBundleIdentifier`, `CFBundleVersion`, `CFBundleShortVersionString`, `SUFeedURL` and `SUPublicEDKey`;
- create one `SPUStandardUpdaterController` with `initWithStartingUpdater:YES …`, then call `checkForUpdates:` from a menu item.

In Cargo, a build script can emit `cargo::rustc-link-search=framework=PATH`, `cargo::rustc-link-lib=framework=Sparkle` and `cargo::rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks`. ([Cargo Book: Build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rustc-link-search)) OpenMusic already depends on `objc2`, `objc2-foundation` and `objc2-app-kit` ([Cargo.toml](../../Cargo.toml)), so it could send those messages from Rust. **Unconfirmed:** I found no Sparkle doc on Rust, and I have not built this.

Signing is the hard part outside Xcode:

- Sparkle ships its XPC services and helpers with ad-hoc signatures and the hardened runtime. Without Xcode's archive-and-export step, we must re-sign `Installer.xpc`, `Downloader.xpc`, `Autoupdate`, `Updater.app` and then the framework, each with `-o runtime` and never `--deep`. ([Sandboxing: Code Signing](https://sparkle-project.org/documentation/sandboxing/#code-signing)) A non-sandboxed app may remove the XPC services. ([Sandboxing: Removing XPC Services](https://sparkle-project.org/documentation/sandboxing/#removing-xpc-services))
- With library validation (part of the hardened runtime), an ad-hoc signed app may fail to load Sparkle. Developer ID signing avoids this. ([Basic Setup](https://sparkle-project.org/documentation/#1-add-the-sparkle-framework-to-your-project))
- `scripts/bundle-macos.sh` would need to copy the framework and sign inside-out. The release workflow would need the EdDSA key, `sign_update` or `generate_appcast`, and somewhere to host the appcast over HTTPS.

### A lighter option

The app can ask GitHub for the newest release: `GET /repos/{owner}/{repo}/releases/latest` returns the latest non-draft, non-prerelease release. ([GitHub REST: Get the latest release](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)) Without authentication, GitHub allows 60 requests an hour per IP address. ([GitHub REST: Rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)) One check per launch, or one a day, stays well under that. When the tag is newer than `CFBundleShortVersionString`, the app shows "Update available: run `brew upgrade --cask openmusic`" or links to the release page.

| | Sparkle 2 | GitHub check + `brew upgrade` |
|---|---|---|
| Installs the update | Yes, in the app | No; the user runs `brew upgrade` |
| Cask stanza | `auto_updates true` | none |
| Work | Framework embedding, re-signing helpers, EdDSA key, appcast hosting, Objective-C calls | One HTTPS request and a version comparison |
| Signing needs | EdDSA is enough; Developer ID recommended | none |
| Users without Homebrew | Covered | Need a "download" link instead |
| Risk if a key leaks | A leaked EdDSA key can sign a malicious update | None beyond GitHub |

Recommendation: ship the GitHub check now and add Sparkle after Developer ID signing. Do not set `auto_updates true` for the light option, because the app does not install anything itself. ([Cookbook: Optional stanzas](https://docs.brew.sh/Cask-Cookbook#optional-stanzas))

## 6. Recommendation for OpenMusic

### Now

1. Use the existing `<owner>/homebrew-tap`. If it should run checks, the workflows `brew tap-new` generates are a starting point: delete `publish.yml`, keep the tap-syntax checks in `tests.yml` and the daily `autobump.yml`, and pin `Homebrew/actions` to a release SHA.
2. Add `Casks/openmusic.rb` as in section 2. Fix `packaging/homebrew/openmusic.rb` to use `depends_on macos: :ventura` (or the final minimum) instead of the deprecated string form.
3. In `release.yml`, push the rendered cask with a write deploy key on the tap repo, stored as `TAP_DEPLOY_KEY`, as the reviewpad release already does. A fine-grained token or a GitHub App token limited to the tap repo also works.
4. Sign the release builds with one stable identity, not ad hoc. Until a Developer ID exists, that means importing the Apple Development certificate into CI. It keeps the designated requirement the same between releases, so Homebrew carries the user's **Open Anyway** approval to each new version. The cost: a development private key sits in CI secrets, and Gatekeeper still rejects the app on first launch. If we decide against that, tell users to expect the Gatekeeper prompt after every upgrade.
5. Document the install command, the trust step it implies, and the **Open Anyway** step in the README and in `caveats`.
6. Add the GitHub "latest release" check in the app. Leave out `auto_updates`.

### After Developer ID signing and notarization

1. Sign with `--options runtime --timestamp` and a Developer ID Application certificate. Apple's notary service requires Developer ID ("Don't use a Mac Distribution, ad hoc, Apple Developer, or local development certificate"), the hardened runtime and a secure timestamp. ([Apple: Notarizing macOS software before distribution](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)) The repo's `bundle-macos.sh` already adds both flags for Developer ID identities.
2. Zip the app with `ditto -c -k --keepParent`, submit the zip with `xcrun notarytool submit --wait`, staple the `.app` with `xcrun stapler staple`, then zip it again for the release. A ticket cannot be stapled to a zip. ([Apple: Customizing the notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)) `notarytool` accepts an App Store Connect API key through `--key`, `--key-id` and `--issuer`. (`xcrun notarytool submit --help`)
3. GitHub secrets (names as in the repo's `release.yml`):

   | Secret | Holds |
   |---|---|
   | `MACOS_CERTIFICATE` | Developer ID Application certificate and private key, as a base64 `.p12` |
   | `MACOS_CERTIFICATE_PASSWORD` | The `.p12` password |
   | `APPLE_API_KEY` | App Store Connect API private key (`.p8`), base64 |
   | `APPLE_API_KEY_ID` | Its key ID |
   | `APPLE_API_ISSUER` | Its issuer ID (UUID) |
   | `TAP_DEPLOY_KEY` | Write deploy key for the tap repo (unchanged) |
   | `SPARKLE_ED_PRIVATE_KEY` | Only if Sparkle is adopted |

   GitHub's guide uses the same pattern with different names: a base64 `.p12`, its password and a random keychain password. It imports into a temporary keychain with `security create-keychain`, `import` and `set-key-partition-list`. ([GitHub Docs: Installing an Apple certificate on macOS runners](https://docs.github.com/en/actions/how-tos/deploy/deploy-to-third-party-platforms/sign-xcode-applications)) Our workflow generates the keychain password at run time, so it needs no secret for it.
4. Check each build with `spctl --assess --type execute` before upload. Homebrew's audit runs `spctl --assess` and `codesign --verify -R=notarized --check-notarization`. ([cask/audit.rb L1321–L1340](https://github.com/Homebrew/brew/blob/738cf5a40801e6953477fb425f61778af318dbd4/Library/Homebrew/cask/audit.rb#L1321-L1340))
5. Drop the `caveats` note.
6. If we then adopt Sparkle: embed and re-sign the framework, add `SUFeedURL` and `SUPublicEDKey`, publish a signed appcast, and set `auto_updates true` in the cask. Livecheck may switch to `strategy :sparkle`.
7. `homebrew/cask` stays out of reach while the notability bar and the "project risk" clause apply. Keep the tap.

## Unconfirmed or not tested

- That `homebrew/cask` would refuse OpenMusic because it uses Spotify's private APIs. No Homebrew page says so; the nearest rule is "project risk". The `psst` cask was accepted, then disabled for Gatekeeper.
- Whether the `zap script:` that runs `security delete-generic-password` works in Homebrew 7. It follows the `openvpn-connect` precedent but I have not run it.
- Whether a renewed Apple Development certificate keeps the same common name, and so the same designated requirement.
- The full flow of Homebrew carrying an **Open Anyway** approval to a new Apple Development-signed version. I read the code and PRs but did not run an upgrade.
- Calling Sparkle from Rust through `objc2`, and linking it with a Cargo build script. Not built.
