# Distributing the desktop client

How a Teachouse release is built, signed, published and updated on every platform — Windows, macOS, Linux and Android, and the Microsoft Store, the Mac App Store and Google Play — and what the founder must do by hand before each store's first release.
The secrets inventory and the step-by-step setup are in `docs/notes/runbooks/release.md`; this note is the design and its sources.

- date: 2026-09-03
- status: every tag from `v0.1.3` to `v0.3.4` is published to both the Cloud's beta channel and GitHub; from `v0.2.0` the Android job succeeds in the same run, so the APK and its sums file ride the release rather than a repair dispatch; `v0.4.0` (phases 0-2), `v0.5.0` (phase 3) `v0.6.0` (phase 4) and `v0.7.0` (phase 5) are the releases that follow each phase of the 2026-09-12 plan of record, and a release is owed at every phase's end; the sums files are written with bare names from `v0.5.0` on; Windows builds are still unsigned because no Azure Artifact Signing account exists; from `v0.21.0` the workflow also builds macOS (Developer ID and Mac App Store), signs Linux updates, builds the Android AAB for Google Play and the Microsoft Store's installer, each gated on its own secrets, none of which exist yet
- decisions it implements: D2 (Windows desktop first, Tauri v2, distributed through CrabNebula Cloud), D29 (build infrastructure: release builds run on a GitHub Windows runner where the MSI and the signing step are native)
- companion: `docs/notes/design/desktop-client.md`, which is the client itself

Every claim below was read from the source named beside it on 2026-09-03.
Where two sources disagree, both are quoted and the disagreement is stated rather than resolved silently.

## What the updater needs, and where each piece lives

Tauri v2's updater refuses to install an update that is not signed, and the documentation is explicit that this cannot be turned off: "Tauri's updater needs a signature to verify that the update is from a trusted source. This cannot be disabled."
The signature is minisign, not a code-signing certificate, and it is entirely separate from Windows code signing; the two solve different problems and neither substitutes for the other.
The keypair is generated once with `cargo tauri signer generate -w ~/.tauri/teachouse.key`, the public half goes into `plugins.updater.pubkey` in `tauri.conf.json` — "This has to be the public key generated from the Tauri CLI... It **cannot** be a file path!" — and the private half reaches the build only through `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, which the documentation notes are deliberately not read from `.env` files.
Source: <https://v2.tauri.app/plugin/updater/>, fetched 2026-09-03.

Setting `bundle.createUpdaterArtifacts` to `true` makes the Windows build emit `Teachouse_<version>_x64-setup.exe` and `Teachouse_<version>_x64_en-US.msi` each with a sibling `.sig` file, and those installers are themselves the update payloads; the `"v1Compatible"` value, which wraps them in archives, exists only for applications migrating from Tauri v1 and does not apply here.
The same page records that the option "will be removed in v3", so it is a migration artefact rather than a permanent knob.

The manifest the updater fetches carries the signature inline: the `signature` field is "The content of the generated `.sig` file... A path or URL does not work!", `version` is SemVer with or without a leading `v`, `pub_date` must be RFC 3339 when present, and the platform keys are `OS-ARCH`, optionally suffixed with the bundle type.
tauri-plugin-updater 2.11.0 looks up `{os}-{arch}-{bundle}` first and `{os}-{arch}` second (`plugins/updater/src/updater.rs`, `get_urls`), where the bundle is `nsis`, `msi`, `app`, `appimage`, `deb` or `rpm`; the generated `latest.json` therefore carries `windows-x86_64` and `windows-x86_64-nsis` (the NSIS installer), `windows-x86_64-msi`, `darwin-aarch64`, `darwin-x86_64` and their `-app` forms (the one universal archive), `linux-x86_64` and `linux-x86_64-appimage`, and `linux-x86_64-deb`, which is what tauri-action writes for the same artefacts.
Tauri validates the whole manifest before it looks at the version, so a malformed entry for a platform we do not ship would break updates for the one we do.

The endpoint already configured in `apps/desktop/src-tauri/tauri.conf.json` is CrabNebula's, and its shape is confirmed by CrabNebula's own documentation: `https://cdn.crabnebula.app/update/ORG_NAME/APP_NAME/{{target}}-{{arch}}/{{current_version}}`.
Source: <https://docs.crabnebula.dev/cloud/auto-updates/tauri/>, fetched 2026-09-03.
That endpoint is served dynamically from the release the Cloud holds, so with CrabNebula as the update host no `latest.json` has to exist for updates to work.
The pipeline generates one anyway, and the reason is recorded under "The lock-in escape hatch" below.

That documented shape is incomplete for a channelled release, and `v0.1.1` proved it.
A release published with `--channel beta` is not visible at the channel-less endpoint: `GET /update/teachouse/teachouse/windows-x86_64/0.1.0` returns 404, while the same URL with `?channel=beta` returns 200 carrying version 0.1.1, its asset URL and its signature.
The query string is CrabNebula's documented form — "To fetch the latest asset of a particular release channel you can append the `?channel=<channel-name>` query string to the URL" — and that sentence is repeated beneath each of the three CDN endpoints the page lists, the `/update/` one included.
Source: <https://docs.crabnebula.dev/cloud/cli/fetch-latest-release/>, fetched 2026-09-03.
An empty parameter is not a fallback to production: `?channel=` returns 404 exactly as the bare URL does, measured against the live CDN on 2026-09-03.

The channel therefore has to follow the build rather than sit only in a checked-in file, because only the release job knows which channel it published to.
The workflow's overlay step composes `plugins.updater.endpoints` from the `CN_CHANNEL` repository variable, appending `?channel=<name>` when it is set and dropping the parameter when it is not, so a binary always polls the channel it came from.
Tauri merges `--config` with RFC 7396 JSON Merge Patch, under which an array in the patch replaces the one it patches rather than extending it, so the overlay's single-element `endpoints` array supersedes the base file's instead of leaving a dead first entry ahead of it.
Read from `crates/tauri-cli/src/helpers/config.rs` and `json_patch::merge` at `tauri-cli-v2.11.4`, the CLI version this workflow pins.
The base file keeps `?channel=beta` so that a local build points at a channel that actually holds releases.

## Where the Windows build runs, and why not here

D29 already settled this — "release builds run on a GitHub Windows runner where the MSI and the signing step are native" — and the primary sources say the same thing twice over, so the pipeline follows the decision rather than reopening it.

Tauri's bundling documentation states that ".msi installers can **only be created on Windows** as WiX can only run on Windows systems", which removes the MSI outright from any Linux route.
It describes the NSIS cross-compile as something that "is not as straight forward as compiling on Windows directly and is not tested as much" and that "should only be used as a last resort if local VMs or CI solutions like GitHub Actions don't work for you".
Source: <https://v2.tauri.app/distribute/windows-installer/>, fetched 2026-09-03.

The signing half is harder still.
Tauri's Windows signing page states that when "cross compiling Windows installers from Linux and macOS machines, you **must** use a custom sign command", because the default `signtool` path "only works on Windows machines".
Source: <https://v2.tauri.app/distribute/sign/windows/>, fetched 2026-09-03.
So the Linux route costs the MSI, costs the native `signtool`, and buys nothing a `windows-latest` runner does not already give.

`just desktop-build-windows` stays exactly as it is, and stays valuable: it is the fast local loop that proves the crate cross-compiles and the bundle assembles, and it needs no runner minutes and no secrets.
It is not the release path, and the recipe's own comment already said so before this note existed.

The console is built on Linux rather than on the Windows runner, and that split is not cosmetic.
`frontendDist` is `web/build`, which is SvelteKit's static output, and that build consumes `web/src/lib/core/generated`, which `just web-wasm` produces with `wasm-bindgen`.
`crates/tam-core-wasm` pins `wasm-bindgen = "=0.2.121"` exactly because the CLI refuses a version that differs from the crate's, and the version this repository trusts is the one nixpkgs supplies to the dev shell.
Rebuilding that on a Windows runner would mean pinning the CLI a second time, in a second place, in a second package manager.
The pipeline instead builds the console once on Linux, deriving the CLI version from the crate's own pin so the two cannot drift, and hands `web/build` to the Windows job as an artefact.

## CrabNebula Cloud

The release lifecycle is four CLI verbs — draft, upload, publish, and optionally purge — and every one of them takes `{org-slug}/{app-slug}`.
`cn release draft <org/app> --framework tauri` reads the version out of the Tauri configuration instead of being told it, which is what keeps the Cloud's idea of the version and the repository's the same object.
`cn release upload <org/app> --framework tauri` discovers the Tauri bundles and their `.sig` files, and `cn release publish <org/app> --framework tauri` makes the release live.
Sources: <https://docs.crabnebula.dev/cloud/cli/create-draft/> and <https://docs.crabnebula.dev/cloud/ci/tauri-v2-workflow/>, both fetched 2026-09-03.

Channels are a flag rather than a separate application: `--channel beta` on the draft, and the documentation is emphatic that the flag "must also be provided to the `release show`, `release purge`, `release upload` and `release publish` commands".
Production is the absence of a channel — it "does **NOT** have an actual name, so no value provided as channel name matches it" — and channels are unlisted rather than private: "Release channels are NOT visible on your application's public page, but they are still publicly accessible."
The pipeline threads one `CN_CHANNEL` variable through all four verbs so that a half-channelled release is not expressible.

Authentication is an API key in `CN_API_KEY`, or `--api-key` on any command.
Source: <https://docs.crabnebula.dev/cloud/cli/install/>, fetched 2026-09-03.

The free tier is genuinely free: "You do **not** need to enter payment details to create an account, upload releases, or use DevTools", and there are "no monthly fees, no subscription tiers, and no credit card required to start using the platform".
Two limits are worth knowing before depending on it.
Storage is reclaimed — "A release that sees no downloads for 90 days will be removed to save space" — though "your _latest_ release on every channel is always protected".
Bandwidth has no published number: "We don't set a fixed number for downloads, but we watch for traffic that is well outside ordinary usage", and the stated response is notification and discussion before any throttling.
Source: <https://docs.crabnebula.dev/cloud/org-management/billing/>, fetched 2026-09-03.

The recommendation is to use it: it is the endpoint the client already ships, it is free at our volume, and it is an official Tauri partner integration.
The 90-day reclamation is the one clause to keep in view, and it bites archives rather than the current release.

## The lock-in escape hatch

CrabNebula serves the update response dynamically, so the pipeline does not need a `latest.json` and the Cloud never reads the one it produces.
It produces one regardless, publishes it as a build artefact beside the checksums, and the reason is that the manifest is the only piece of the arrangement that is expensive to reconstruct after the fact.
The `signature` field must carry the literal contents of the `.sig` file produced by that exact build; once the runner is gone and the artefacts have aged out, that content is not recoverable and every installed client is stranded on an endpoint we would no longer control.
A manifest emitted at build time turns a hosting migration into a file copy: point `plugins.updater.endpoints` at any static host, upload the installers and the manifest, and the installed base follows.
Its `url` fields are built from the `UPDATER_BASE_URL` repository variable, and when that variable is unset the manifest carries bare filenames and says so in its own `notes` field, which is honest about it being a fallback rather than a live manifest.

## Windows code signing, as of 2026

The recommendation is Azure Artifact Signing, formerly Trusted Signing, formerly Azure Code Signing.
Microsoft's own guidance now names it: "Artifact Signing (formerly Trusted Signing) is Microsoft's recommended code signing service for non-Store distribution", at a cost that "Starts at $9.99/month", with "No hardware token required — integrates directly with CI/CD pipelines (GitHub Actions, Azure DevOps)".
Source: <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation>, `ms.date` 2026-05-04, last updated 2026-08-17, fetched 2026-09-03.

Buy it for the publisher name and the absence of a hardware token, not for SmartScreen, because SmartScreen is no longer for sale.
The same Microsoft page states plainly: "EV certificates no longer bypass SmartScreen. Years ago, signing files with an Extended Validation (EV) code signing certificate would result in positive SmartScreen reputation by default, but this behavior no longer exists... Paying a premium for EV solely to avoid SmartScreen warnings is no longer justified."
Its table gives OV and EV the identical first-download outcome: "Warning — app flagged as unrecognized until reputation accumulates; verified publisher name is displayed."
What signing does buy is the publisher name in that warning and the ability for reputation to accumulate across releases at all: "Signing files using a trusted certificate can allow certificate reputation to build... Unsigned files must build reputation anew with every update."
Expect the warning for a while: "it can take several weeks and hundreds of clean installs from a wide audience", and "There is no need (or mechanism) to manually submit a file for SmartScreen reputation review for consumer endpoints."

Tauri's own signing page still says an EV-signed app "will receive an immediate reputation with Microsoft SmartScreen and won't show any warnings to users".
That is contradicted by the Microsoft page above, which is both the authority on SmartScreen and the more recently revised of the two.
Take Microsoft's, and read `docs/notes/design/desktop-client.md`'s "roughly $120 a year through Azure Trusted Signing" as the same recommendation at the same price ($9.99 a month) under the service's current name.

Eligibility is the part to check before budgeting for it, and it is the part where the loudest search results are wrong.
The current prerequisites say: "Public Trust certificates are available to organizations in the United States, Canada, the European Union, the United Kingdom, Australia, New Zealand, Japan, South Korea, Singapore, Switzerland, Norway, and Israel. Individual developers must be located in the United States or Canada."
No minimum organisation age appears anywhere in that page.
Source: <https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart>, `ms.date` 2026-05-21, last updated 2026-08-11, fetched 2026-09-03.
A widely-repeated "three or more years of verifiable tax history" rule does circulate, and on the Microsoft Q&A thread where it is most often cited it appears in an AI-generated answer that a Microsoft moderator then corrected on 2026-08-17: "Artifact Signing has country/region onboarding pre-reqs, no minimum org age restrictions."
Source: <https://learn.microsoft.com/en-us/answers/questions/5977141/azure-artifact-signing-trusted-signing-is-a-us-llc>, fetched 2026-09-03.
Treat the quickstart page as the answer and the three-year rule as stale.

The individual-developer path validates identity through a third party, AU10TIX, with a government-issued photo ID and a recent proof of address, and the certificate subject is populated from the Azure billing account rather than from the form, so the billing account's legal name and address must already be right before the request is created.
Organisation validation takes "from 1 to 20 business days (possibly longer if we need to request more documentation from you)", which is the lead time to plan around; it is the single longest-pole item in this whole document.

Two alternatives, for completeness.
An OV certificate from a commercial CA costs more, requires a hardware token or an HSM, and by Microsoft's table produces the identical SmartScreen outcome, so it is worse on every axis that matters here.
Shipping unsigned is free and is what the tree does today: the warning says "Windows protected your PC", no publisher name is shown, reputation restarts at zero with every release, and "Enterprise policy can prevent continuation entirely".
For a product sold to teachers who are told to download an executable, unsigned is a conversion problem rather than a technical one.

Signing is wired as an opt-in overlay rather than into `tauri.conf.json`, so that the tree builds without an Azure account and starts signing the day one exists.
Tauri's signing page suggests a `bundle.windows.signCommand` of the form `"artifact-signing-cli -e ... -a MyAccount -c MyProfile -d MyApp %1"`, but Microsoft's own integration page documents no such standalone CLI, and the supported Windows path there is SignTool driven by a signing library.
The pipeline follows Microsoft rather than Tauri, because Microsoft is the authority on its own service and its page is the more recently revised.

That path has three parts, all quoted from <https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations>, `ms.date` 2026-05-14, last updated 2026-08-03, fetched 2026-09-03.
The runner installs the client tools, which bundle SignTool's prerequisites, the .NET 8 runtime and the signing library, from the MSI Microsoft publishes at `https://download.microsoft.com/download/70ad2c3b-761f-4aa9-a9de-e7405aa2b4c1/ArtifactSigningClientTools.msi`.
It writes a `metadata.json` naming the account — `{"Endpoint": ..., "CodeSigningAccountName": ..., "CertificateProfileName": ...}` — with the documented `ExcludeCredentials` list, which forces `DefaultAzureCredential` past every interactive and managed-identity method down to `EnvironmentCredential`, the one that reads `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET`.
It then composes the `signCommand` in Microsoft's exact shape: `signtool.exe sign /v /debug /fd SHA256 /tr "http://timestamp.acs.microsoft.com" /td SHA256 /dlib "...\\Azure.CodeSigning.Dlib.dll" /dmdf "...\\metadata.json" %1`.

Two details in that command are not decoration.
The `Endpoint` must name the region the account and the certificate profile were created in, because "A region/endpoint mismatch commonly causes a 403 Forbidden error and an internal `SignerSign()` failure during signing"; the region table is on the same page.
The timestamp is mandatory rather than advisory, because "Artifact Signing certificates have a three-day validity, so time stamping is critical for continued successful validation of a signature beyond that three-day validity period" — an untimestamped signature stops validating three days after it is made.

The two absolute paths in that command are discovered on the runner rather than hardcoded, because Microsoft documents the installer but not where it lands.
The step fails loudly and by name if either the signing library or SignTool is not found after the install, which is the failure most likely to greet the founder's first signed release.

Tauri's guide documents a second Windows path, an exported certificate: import the `.pfx` into the user store and set `bundle.windows.certificateThumbprint`, `digestAlgorithm` and `timestampUrl`, after which Tauri runs SignTool itself (<https://v2.tauri.app/distribute/sign/windows/>, fetched 2026-10-05).
The workflow carries it as the fallback — `WINDOWS_CERTIFICATE` and `WINDOWS_CERTIFICATE_PASSWORD`, used only when the `AZURE_*` secrets are absent — because Tauri's page makes no recommendation between the two and Microsoft's does.
It is a fallback with a short reach: since June 2023 the CA/Browser Forum requires new OV and EV code-signing keys to be generated in hardware, so a certificate bought today normally cannot be exported to a `.pfx` at all, which Tauri's page does not mention.
Whichever path signs, the job then checks every `.exe` and `.msi` with `Get-AuthenticodeSignature`, so a sign command that ran and signed nothing fails the run rather than shipping as signed.

## Every platform, as of 0.21.0

Read from <https://v2.tauri.app/distribute/> and each page under it, the updater plugin page, the GitHub pipeline page and the tauri-action README, all on 2026-10-05.

**macOS** is one universal build (`--target universal-apple-darwin --bundles app,dmg`), signed with a Developer ID Application certificate under the hardened runtime (`bundle.macOS.hardenedRuntime`, Tauri's default), and notarized by Tauri itself from whichever credentials are in the environment: `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`, or `APPLE_API_ISSUER`, `APPLE_API_KEY` and a key file at `APPLE_API_KEY_PATH` (<https://v2.tauri.app/distribute/sign/macos/>).
The certificate goes into a throwaway keychain, as Tauri's CI example does, and the identity is read back from it rather than configured, so `bundle.macOS.signingIdentity` stays null in the tree.
Tauri notarizes and staples the `.app`; the workflow then signs, notarizes (`xcrun notarytool submit --wait`) and staples the `.dmg` itself, because that is the file a seller downloads, and checks both with `spctl`.
`minimumSystemVersion` is set to 10.15 explicitly rather than left at Tauri's 10.13 default, and the Developer ID build carries no entitlements.
The updater payload is the `.app.tar.gz`; the `.dmg` is never one (<https://v2.tauri.app/plugin/updater/>).
CrabNebula has no universal platform — its DMG slugs are `dmg-aarch64` and `dmg-x86_64` and its update platforms `darwin-aarch64` and `darwin-x86_64` (<https://docs.crabnebula.dev/cloud/cli/upload-assets/>) — so the one file is uploaded under each, as copies named for the architecture; a minisign signature covers bytes, not names, so both archives share one `.sig`.
An unsigned macOS build is never published: Gatekeeper refuses it on every Mac that downloads it, so without the certificate and notarization credentials the job is skipped.

**The Mac App Store** build follows <https://v2.tauri.app/distribute/app-store/>: the provisioning profile embedded through `bundle.macOS.files`, the sandbox and team entitlements (`apps/desktop/src-tauri/Entitlements.appstore.plist`, with the team-scoped keys written in by the job from `APPLE_TEAM_ID`), `ITSAppUsesNonExemptEncryption` false in `Info.plist`, an Apple Distribution signature, `xcrun productbuild --sign … --component` and `xcrun altool --upload-app --type macos`.
It is built with the crate's `app-store` feature, which leaves `tauri-plugin-updater` unregistered: App Review guideline 2.4.5(vii) forbids a Mac App Store app from updating itself, and the start-up check already refuses to ask an unregistered plugin.
Sandboxed behaviour — marketplace sign-in, picking files, the library's peer connections — has not been exercised, and the runbook asks for that on TestFlight before the first review.

**Linux** updates were not published before 0.21.0; the AppImage and the `.deb` were downloads only.
They are now both update payloads, and that makes the plain `linux-x86_64` key ambiguous: the plugin installs a payload with the installer matching how the running copy was installed (`Update::install_inner` dispatches on the bundle type to `install_deb`, `install_rpm` or the AppImage path), so a `.deb` install handed the AppImage would fail in `dpkg`.
The Linux build's endpoint therefore templates `{{target}}-{{arch}}-{{bundle_type}}`, which the plugin substitutes (2.11.0, `updater.rs` lines 471-486), and each bundle is uploaded to the Cloud under its suffixed platform, `linux-x86_64-appimage` and `linux-x86_64-deb`, a form the Cloud documents.
Linux builds before 0.21.0 poll the bare `linux-x86_64` key, which nothing is uploaded under, so they are offered nothing rather than offered the wrong installer.
Windows and macOS keep `{{target}}-{{arch}}`: each has one updatable bundle, and the installed Windows base polls exactly that.

**Android** builds the AAB beside the APK in one Gradle run (`--apk --aab`), both signed with the upload key.
The AAB goes to Google Play through `r0adkll/upload-google-play`, track `internal` unless `PLAY_TRACK` says otherwise; Tauri automates no part of a Play release and Play requires the first upload to be made by hand (<https://v2.tauri.app/distribute/google-play/>).
Tauri's versionCode, `major*1000000 + minor*1000 + patch`, only ever rises, which is what Play requires of each upload.

**The Microsoft Store** cannot take an MSIX from Tauri: the bundler's targets are `deb`, `rpm`, `appimage`, `nsis`, `msi`, `app` and `dmg`, and the MSIX issue (tauri-apps/tauri#4818) is open with "no explicit plans" as of January 2026.
Tauri's guide instead submits an EXE: an NSIS installer with WebView2 bundled offline (`tauri.microsoftstore.conf.json`, `webviewInstallMode` `offlineInstaller`), signed, installed silently with `/S`, under a publisher name that differs from the product name (<https://v2.tauri.app/distribute/microsoft-store/>).
Partner Center installs an EXE app from a URL the publisher hosts, at certification and on every Store install, so the URL must be public and must not change under a live submission.
The GitHub release is private, so the URL is the download mirror's, which keeps the current Store installer and the one before it.
The `microsoft-store` job waits for the mirror to serve the exact bytes the Windows job built, then replaces the draft submission's package with the Microsoft Store Developer CLI (`msstore submission update`, `msstore submission publish`); Microsoft archived its `store-submission` action in that CLI's favour, and the CLI covers EXE apps.

**tauri-action** is not used, for three reasons read from its README and `src/upload-version-json.ts`: it publishes to GitHub releases and nothing else, where this pipeline publishes to the Cloud as well; from v1 its `latest.json` URLs point at the GitHub API asset URL, which a private repository answers only with a token the installed app does not have; and the workflow already names each artefact explicitly, which is what the Cloud upload needed after `v0.1.0`.
Its `latest.json` key scheme is what `.github/scripts/updater-manifest.sh` reproduces, universal macOS archive included, so moving to it later changes no key.

The workflow is split for the same reason the platforms are: `desktop-release.yml` holds the gates and the joins (verify, draft, console, the GitHub release, the Store submission, publish), and each platform is a reusable workflow it calls with `secrets: inherit` — `release-windows.yml`, `release-macos.yml`, `release-mac-app-store.yml`, `release-linux.yml`, `release-android.yml`.
The setup every desktop build shares (the console artefact, the toolchain, the pinned CLI) is the composite action `.github/actions/tauri-setup`, and the release overlay (updater artefacts on, the channel's endpoint, any signing patch) is `.github/actions/release-overlay`.
Which platforms a tag builds is decided once, by `.github/scripts/release-plan.sh` in the verify job, from which secrets exist; it writes the plan to the run summary, and `github-release` writes what each platform job came to.

## The workflow, and the rules it is built to

`.github/workflows/desktop-release.yml` triggers only on a `v*.*.*` tag push, and every job additionally refuses to run on a fork.
There is no `workflow_dispatch`, no branch trigger and no schedule, so there is no path to a release build that is not a tag on this repository.

Every third-party action is pinned to a full commit SHA with the tag in a trailing comment, following GitHub's guidance that "Pinning an action to a full-length commit SHA is currently the only way to use an action as an immutable release", and that this "helps mitigate the risk of a bad actor adding a backdoor to the action's repository".
Permissions are empty at the top of the file and granted per job, following the same page's advice to "set the default permission for the `GITHUB_TOKEN` to read access only for repository contents", raised "as required, for individual jobs".
No secret and no piece of tag-derived text is interpolated into a shell script body; every one reaches `run:` through `env:`, which is the pattern the same page prescribes for keeping context values out of script generation.
Source: <https://docs.github.com/en/actions/reference/security/secure-use>, fetched 2026-09-03.

One supply-chain gap is worth naming rather than hiding.
`crabnebula-dev/cloud-release` is pinned by SHA, but the action's own body downloads the `cn` binary from `https://cdn.crabnebula.app/download/crabnebula/cn-cli/latest/...` — the literal path segment is `latest`, so the CLI itself is not pinned by anything.
Read from <https://github.com/crabnebula-dev/cloud-release/blob/main/action.yml>, fetched 2026-09-03.
Pinning the action therefore pins the wrapper and not the tool, and no versioned download URL is documented.
This is accepted rather than solved: the alternative is vendoring a binary we would then have to update by hand, and the blast radius is the upload step of a release, not the artefact's contents, which are signed before that step runs.

Provenance is attested with `actions/attest-build-provenance`, and it is guarded because this repository is private.
GitHub's terms are that "artifact attestations are only available for public repositories" on the Free, Pro and Team plans, and "To use artifact attestations in private or internal repositories, you must be on a GitHub Enterprise Cloud plan".
Read from <https://github.com/actions/attest-build-provenance>, fetched 2026-09-03.
The step therefore carries `if: ${{ !github.event.repository.private }}`, so it is inert today and becomes live the moment the repository is made public, with no edit.
SHA-256 checksums are unconditional and are the provenance that does work today, together with the minisign `.sig` files, which are a stronger claim than a checksum because they bind the artefact to a key rather than to a listing.

The Android APK rides along, built in parallel with the Windows bundles and published only when it can be installed.
The Cloud has no Android platform identifier — neither the `--public-platform` list nor the `--update-platform` list has one (<https://docs.crabnebula.dev/cloud/cli/upload-assets/>, fetched 2026-09-03) — so the APK is uploaded with no platform flag at all, which is the documented form for an asset that is platform-independent and is what the Cloud calls a generic asset.
Such an asset is still served from the CDN by file name, at `https://cdn.crabnebula.app/download/<org-slug>/<app-slug>/latest/<asset-file-name>` (<https://docs.crabnebula.dev/cloud/cli/fetch-latest-release/>, fetched 2026-09-03).
It gets no download button and drives no updater, which costs nothing here because `tauri-plugin-updater` supports Android at level `none` anyway; `docs/notes/design/android-client.md` holds that reading.
An unsigned APK cannot be installed, so the job runs only when all three `ANDROID_KEY_*` secrets exist.
The `verify` job answers that question — `secrets` is not a context an `if:` may read, but a job-level `env:` is, so the boolean is computed there once and travels as an output — and when the answer is no the run summary records that the APK was skipped for want of a signing key.
Building one anyway would cost twenty minutes of every tag and produce nothing installable, so the skip is the whole saving.
The APK is additive to a release rather than a gate on it, which `v0.1.2` is the reason for stating explicitly.
`publish` waits for every job that uploads to the Cloud draft, whatever their results, because a published release is sealed, and it requires only the Windows job's success.
`github-release` downloads each platform's artefacts when that platform's job succeeded, and records in the run summary what each job came to.
Both conditions use a status function, because a `needs` job that skips or fails would otherwise skip the job waiting on it, and then name the one result they actually require.
Setting the three secrets is the only change needed to make the next tag carry an APK.
The NDK version reaches both Android workflows through `.github/scripts/android-pins.sh`, which reads it from `flake.nix`, so the pin has one home rather than three.

Version consistency is checked before anything is built, by `.github/scripts/release-check.sh`, which the workflow and `just release-check` both call so that there is one definition.
It compares the tag against `apps/desktop/src-tauri/tauri.conf.json` and the `[package]` version in `apps/desktop/src-tauri/Cargo.toml`, and it fails on a placeholder updater public key or an unedited `ORG`/`APP` endpoint, so that a release cannot be cut against a build whose updater could never verify anything.
It also refuses a tag whose `docs/releases/<version>.md` is missing or blank, because notes are written for a seller rather than derived from commits, so nothing here can generate them and their absence has to be an error.
Run it with no argument to check the tree, or `just release-check v0.2.0` to check a tag you are about to create.
It is deliberately not part of `just check` or `just pre-push`: it was red by design for as long as the updater key and endpoint were placeholders, and a gate that is red for a reason everyone has agreed to stops being read.
Both are set now, so it passes.

## Release notes, and the two places a release lands

`docs/releases/<version>.md` is the notes for that version, written for a seller and not as a changelog of commits.
One file is the source for both destinations, so the Cloud and GitHub cannot describe the same release differently.

`cn release draft` is the only verb that accepts them: it takes `--notes` and `--notes-file`, and neither `upload` nor `publish` has any such flag.
Read from `cn release draft --help` and `cn release publish --help`, cn 0.13.4, on 2026-09-03.
That is why `v0.1.1`'s manifest carries `"notes":""` and always will — the draft that made it was created without them, and no later verb can add them.

The same boundary governs assets, and it was found the same way, by hitting it.
`cn release upload` refuses a release that has already been published: "Failed to create asset for uploading: The release was already published", measured on run 33747151917 attaching an APK to the published `0.1.3`.
So a CrabNebula release is sealed at publish in both its notes and its assets, and everything it will ever carry has to be in place while it is still a draft.
Two consequences follow.
The APK reaches the Cloud only through the release workflow's own `android` job, which runs between `draft` and `publish`, so from the next tag it is in the release rather than beside it.
And `android-build.yml`'s `attach_to` path can repair the GitHub release alone; it still attempts the Cloud, because a release left in draft would accept the asset, and it records that specific refusal as a skip while still failing on any other error.

The workflow also creates a GitHub release for the tag, carrying the same notes file, both installers, the updater `.sig` files, `SHA256SUMS.txt` and `latest.json`.
`latest.json` is there because it is the hosting escape hatch described above and the workflow artefact it otherwise lives in expires after thirty days, whereas a release asset does not.
The job is the only one in the file with `contents: write`, and it is the only one that needs it.
A release cut to a channel is marked `--prerelease`, so a beta does not present itself as the repository's latest release.

## What the founder must do

Nothing below can be inferred.
As of 2026-09-03 the updater keypair, the CrabNebula organisation and application, and the API key all exist, and the first tag has been cut; run 33730896941 is the evidence.
The Azure signing account does not exist, so every Artifact Signing item below is still outstanding and releases carry no Authenticode signature until it is done.

Generate the updater keypair, on the founder's own machine, and never let the private half into the repository.

```
nix develop --command cargo tauri signer generate -w ~/.tauri/teachouse.key
```

Answer the password prompt with a real password and record it in the password manager alongside the key file.
Back the key file up somewhere that survives losing the machine: this key is irreplaceable, and losing it means the installed base can never be updated again.
Then put the public half — the contents of `~/.tauri/teachouse.key.pub`, one long base64 line, not a path — into `plugins.updater.pubkey` in `apps/desktop/src-tauri/tauri.conf.json`, replacing `PLACEHOLDER_FOUNDER_SUPPLIES_THIS_RUN_JUST_RELEASE_CHECK`.
The field is required by the configuration schema and cannot be left absent, which is why a marked placeholder sits there; `just release-check` fails while it is still in place.

Create the CrabNebula organisation and application, at <https://web.crabnebula.cloud/>, signing in with GitHub.
Note the two slugs it gives you; the pair is written `org/app`.
Then replace `ORG` and `APP` in `plugins.updater.endpoints` in the same file, so the endpoint reads `https://cdn.crabnebula.app/update/<org>/<app>/{{target}}-{{arch}}/{{current_version}}?channel=beta`.
Keep the `?channel=` suffix while releases go to a channel; the release job overwrites this endpoint with the channel it actually published to, so the value here governs local builds only.
`just release-check` fails while `/ORG/APP/` is still there.

Mint the CrabNebula API key at <https://docs.crabnebula.dev/cloud/org-management/create-api-key/>, choosing a key with write permission.

Set up Windows code signing, which is the long-lead item and should be started before anything else on this list.
Register the resource provider, create the account, and complete identity validation in the Azure portal — identity validation cannot be done from the CLI:

```
az login
az provider register --namespace "Microsoft.CodeSigning"
az extension add --name artifact-signing
az group create --name teachouse-signing --location westus2
az artifact-signing create -n teachouse -l westus2 -g teachouse-signing --sku Basic
```

Then, in the portal, assign yourself the Artifact Signing Identity Verifier role, create an identity validation under the Artifact Signing account — Individual if you are signing as yourself, Organization if Teachouse is a registered entity — and create a Public Trust certificate profile against it once validation completes.
Before you start, make the Azure billing account's legal name and address exactly what should appear on the certificate, because for an individual identity the certificate subject is taken from the billing account and not from the form.
Budget 1 to 20 business days for an organisation validation.

Then create a service principal for the workflow and grant it the Artifact Signing Certificate Profile Signer role on the certificate profile:

```
az ad sp create-for-rbac --name teachouse-release --years 1
```

Its `appId`, `password` and `tenant` become `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` and `AZURE_TENANT_ID` below, and the account name, profile name and the region's endpoint URI become the three `AZURE_SIGNING_*` repository variables.
Nothing else needs installing: the workflow installs the Artifact Signing client tools on the runner itself.

Create the GitHub secrets and variables by the exact names `docs/notes/runbooks/release.md` lists, which is the one inventory: it covers the updater key, the Cloud, Windows signing (Artifact Signing or a PFX), macOS, the Mac App Store, Android, Google Play and the Microsoft Store, where each value comes from, and what a missing one does.

Then cut a release.
Write `docs/releases/<version>.md` first, for a seller rather than for a reviewer, then bump the version in both `apps/desktop/src-tauri/tauri.conf.json` and `apps/desktop/src-tauri/Cargo.toml` to the same value, run `just release-check v<version>` locally until it passes, commit, and push the tag.
`just release-check` fails while the notes are missing or blank, so the order is enforced rather than remembered.
Watch it with `gh run watch`, and read the log through the repository's capture convention rather than the terminal.
When it finishes, download the installer from the CrabNebula release page and verify three things by hand, because the workflow cannot verify any of them for you.
Check the signature: right-click the `.exe`, Properties, Digital Signatures, and confirm the publisher name is what the certificate profile says.
Check the checksum against the `SHA256SUMS.txt` artefact.
Then check the update path, which is the one that silently breaks: install the previous version on a clean Windows machine, launch it, and confirm it offers and applies the new one.
Do this on every release where the updater configuration changed, and at least once against a real previous version before announcing the product, because an updater that cannot verify is indistinguishable from one that has no update to offer.

### The first tag, and why it produced no release

`v0.1.0` was cut on 2026-09-03 and did not release.
In run 33730896941 the verify, draft and console jobs passed, the Windows runner built both bundles and signed each with the updater key, and the collect step assembled them with their checksums; the upload to CrabNebula Cloud then failed in 120 ms with `Could not find Tauri bundle path`.

The cause was `--framework tauri` on `cn release upload`.
That flag makes the CLI "[a]utomatically determine the release version from the framework's configuration file and the files to upload from the framework's output bundles", and it looks for those bundles beneath the Tauri configuration it read — here `apps/desktop/src-tauri/target/release/bundle`.
This crate is a member of the root Cargo workspace, so cargo wrote them to the workspace root instead, at `target/release/bundle`, which is where the collect step had already found them.
Read from `cn release upload --help` (cn 0.13.4) and <https://docs.crabnebula.dev/cloud/cli/upload-assets/>, both consulted 2026-09-03.
The action cannot correct this: its only inputs are `command`, `api-key`, `path` — the directory the `cn` binary downloads into, not a bundle path — and `working-directory`, and no working directory holds both the configuration and the bundles.
Read from <https://github.com/crabnebula-dev/cloud-release/blob/1a8803698ba41de6b23e42abc5dcc3721308233c/action.yml>, the revision this workflow pins.

The fix drops `--framework` from the upload and names each file: `--file`, the version positional the verify job already computed, and the platform identifiers the CLI documents.
The NSIS installer uploads as `--public-platform nsis-x86_64 --update-platform windows-x86_64` with its `.sig`, and the MSI as `--public-platform wix-x86_64` alone, because only one artefact can be the update for a platform and the NSIS installer is the one Tauri's Windows `installMode` drives without an elevation prompt.
Naming the files also retires the hazard the CLI's own help warns of, that `--framework` "uploads all discovered artifacts to the release, so make sure there is no older bundles mixed in the framework's output directories" — a live risk with a cached target directory.
`draft` and `publish` keep `--framework tauri`, because neither reads a bundle and it is what stops the Cloud's version and the tree's from disagreeing.

`v0.1.2` and `v0.1.3` both failed in the Android job, for the same missing binary found at two different depths, and the pair is worth recording because the first fix was not wrong so much as incomplete.

`v0.1.2` failed at the top: the job installed the CLI from npm, which provides a `tauri` binary, and then invoked `cargo tauri`, which is a separate cargo package that was never installed.
`v0.1.3` failed one layer down, after the top-level invocation had been corrected: `cargo tauri` reappeared inside Gradle, in the task `:app:rustBuildArm64Release`.
That task is Tauri's own generated plugin, and it hardcodes the executable — `val executable = """cargo"""` and `listOf("tauri", "android", "android-studio-script")`, at lines 19 and 51 of `gen/android/buildSrc/src/main/java/io/teachouse/desktop/kotlin/BuildTask.kt`.
There is no property or environment variable to point it elsewhere, so which CLI the workflow itself invokes was never the whole question: `cargo-tauri` has to be on the runner either way.

Both Android jobs therefore install the real cargo binary and invoke `cargo tauri`, matching the nix shell that produced the measured APKs, and the npm install is gone from them.
The install uses `cargo binstall` against Tauri's own published binary rather than compiling the crate: `tauri-cli` 2.11.4 carries `[package.metadata.binstall]` with `pkg-url = "{ repo }/releases/download/tauri-cli-v{ version }/cargo-tauri-{ target }.{ archive-format }"`, and the `tauri-cli-v2.11.4` release publishes `cargo-tauri-x86_64-unknown-linux-gnu.tgz`, which that template resolves to and which answers with 8.3 MB.
Read from `crates/tauri-cli/Cargo.toml` at `tauri-cli-v2.11.4` and the release's own asset list on 2026-09-03.
`cargo tauri --version` runs immediately after, so a broken install costs seconds rather than the twenty minutes to the Gradle task that would have found it.
The Windows job still installs from npm and invokes `tauri`, which is not an inconsistency to tidy away: it has green runs behind it, and it never enters Gradle.

The `v0.1.2` failure also blocked `publish`, which is what prompted decoupling the APK from the desktop release above; by `v0.1.3` that decoupling held, and the desktop release published while the APK did not.
Dispatch run 33747151917 then built and signed the APK with the binstall install, which is what proves that fix, and attached `Teachouse_0.1.3_arm64.apk` to the GitHub release.

`v0.1.0` is a spent tag rather than a release.
It exists on the repository, no GitHub release was ever created for it, and a pushed tag cannot be moved without rewriting what others have already fetched, so the next release is `v0.1.1` and it will be the first published one.
That run did leave a `0.1.0` draft on CrabNebula Cloud, unpublished by design; `cn release purge` removes it, and purging it before tagging keeps an orphan out of the account.

## What could not be verified here

This section recorded the state before any tag existed, and run 33730896941 superseded most of it on 2026-09-03.
The CrabNebula organisation, application and API key now exist, and all four verbs have run: `v0.1.1` drafted, uploaded and published, and the CDN serves its manifest.
The updater keypair exists, `createUpdaterArtifacts` was true in that build, and a `.sig` was produced beside each installer.
No Azure resource was created, so `AZURE_CLIENT_ID` was unset, every Artifact Signing step was skipped, and the bundles carry no Authenticode signature.
What follows is the local evidence that existed before that run.

The workflow is clean under `actionlint` 1.7.12, which found and cost two real defects: `secrets` is not a context a step-level `if:` may read, and one glob parsed `ls` output.
`release-check.sh` is clean under `shellcheck`, and was exercised on four paths against a fixture tree — matching tag, wrong tag, mismatched manifest version, and the real tree, which failed while both placeholders were still in it.
The console job's command sequence was run end to end inside the dev shell and produced `web/build`, and the collect step's find, checksum, glob and manifest logic was run against a fixture bundle tree, producing a `latest.json` carrying the three fields Tauri documents as required.

That covered everything the pipeline does on Linux and nothing it does on Windows, and its own prediction — that the first failure would be a path or an environment detail on the Windows runner rather than a logic error — is exactly what the first tag found.
Runs 33730896941 and 33734883020 have since exercised the Windows build, the NSIS and MSI bundling, the updater signing and all four CrabNebula verbs, on a `--channel beta` release, which is what kept the first failure off the production channel.
Still unexercised: every Azure signing step, and the installed-base update path — which is the one the missing channel would have broken, and which no amount of green CI would have caught.
