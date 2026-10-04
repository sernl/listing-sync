#!/usr/bin/env bash
# Which platforms a tagged release builds, signs and publishes, decided once
# from which secrets exist, before any runner minute is spent.
#
# `secrets` is not a context an `if:` may read, but a job-level `env:` may, so
# the desktop-release workflow's verify job turns each secret group into a
# `true`/`false` here and every later job reads the answer as an output. A
# platform whose secrets are absent is skipped, never built half-signed, and
# the run summary says which secret was missing; setting the secrets is the
# only change needed for the next tag to carry it. Runnable by hand with the
# same variables, which is how the table below is checked.
#
# Inputs (each "true" or anything else):
#   HAS_AZURE_SIGNING   AZURE_CLIENT_ID, AZURE_CLIENT_SECRET, AZURE_TENANT_ID
#   HAS_PFX_SIGNING     WINDOWS_CERTIFICATE, WINDOWS_CERTIFICATE_PASSWORD
#   HAS_APPLE_CERT      APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD
#   HAS_APPLE_ID        APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID
#   HAS_APPLE_API_KEY   APPLE_API_ISSUER, APPLE_API_KEY, APPLE_API_KEY_P8
#   HAS_APP_STORE       APPLE_APP_STORE_CERTIFICATE,
#                       APPLE_APP_STORE_CERTIFICATE_PASSWORD,
#                       APPLE_APP_STORE_PROVISIONING_PROFILE, APPLE_TEAM_ID
#   HAS_ANDROID         ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD, ANDROID_KEY_BASE64
#   HAS_PLAY            GOOGLE_PLAY_SERVICE_ACCOUNT_JSON
#   HAS_MSSTORE         MSSTORE_TENANT_ID, MSSTORE_CLIENT_ID,
#                       MSSTORE_CLIENT_SECRET, MSSTORE_SELLER_ID,
#                       MSSTORE_PRODUCT_ID
#   MSSTORE_PUBLISHER   the MSSTORE_PUBLISHER repository variable's value
# Outputs, as key=value lines on stdout (append to $GITHUB_OUTPUT):
#   windows_signing  artifact | pfx | none
#   macos            true | false    Developer ID build, signed and notarized
#   macos_notary     apple-id | api-key | none
#   mac_app_store    true | false
#   android          true | false
#   play             true | false
#   store_installer  true | false    build the Microsoft Store's offline installer
#   msstore          true | false    submit it to Partner Center
# The Markdown plan goes to $GITHUB_STEP_SUMMARY when set, else to stderr.
set -euo pipefail

on() { [ "${!1:-}" = "true" ]; }

summary=${GITHUB_STEP_SUMMARY:-/dev/stderr}
rows=()
row() { rows+=("| $1 | $2 | $3 |"); }

# Windows always builds: it is the release the installed base updates from.
# Azure Artifact Signing is preferred where both are configured, because it is
# Microsoft's recommended service for non-Store distribution and needs no
# exportable key; a PFX is the fallback Tauri's own guide documents.
if on HAS_AZURE_SIGNING; then
    windows_signing=artifact
    row "Windows NSIS + MSI" "builds" "signed with Azure Artifact Signing"
elif on HAS_PFX_SIGNING; then
    windows_signing=pfx
    row "Windows NSIS + MSI" "builds" "signed with the WINDOWS_CERTIFICATE PFX"
else
    windows_signing=none
    row "Windows NSIS + MSI" "builds **unsigned**" "neither AZURE_CLIENT_ID/AZURE_CLIENT_SECRET/AZURE_TENANT_ID nor WINDOWS_CERTIFICATE/WINDOWS_CERTIFICATE_PASSWORD is set; SmartScreen shows no publisher"
fi

row "Linux AppImage + deb" "builds" "updater-signed with TAURI_SIGNING_PRIVATE_KEY"

# An unsigned or un-notarized macOS build is refused by Gatekeeper on every Mac
# that downloads it, so there is no unsigned macOS fallback: it is skipped.
if on HAS_APPLE_ID; then
    macos_notary=apple-id
elif on HAS_APPLE_API_KEY; then
    macos_notary=api-key
else
    macos_notary=none
fi
if on HAS_APPLE_CERT && [ "$macos_notary" != none ]; then
    macos=true
    row "macOS universal app + dmg" "builds" "Developer ID signed, notarized with the $macos_notary credentials"
else
    macos=false
    if ! on HAS_APPLE_CERT; then
        row "macOS universal app + dmg" "**skipped**" "APPLE_CERTIFICATE and APPLE_CERTIFICATE_PASSWORD are not both set"
    else
        row "macOS universal app + dmg" "**skipped**" "no notarization credentials: set APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID, or APPLE_API_ISSUER, APPLE_API_KEY and APPLE_API_KEY_P8"
    fi
fi

# The App Store upload authenticates with the App Store Connect API key alone.
if on HAS_APP_STORE && on HAS_APPLE_API_KEY; then
    mac_app_store=true
    row "Mac App Store pkg" "builds and uploads" "to App Store Connect; it appears in TestFlight once Apple has processed it"
else
    mac_app_store=false
    if ! on HAS_APP_STORE; then
        row "Mac App Store pkg" "**skipped**" "APPLE_APP_STORE_CERTIFICATE, APPLE_APP_STORE_CERTIFICATE_PASSWORD, APPLE_APP_STORE_PROVISIONING_PROFILE and APPLE_TEAM_ID are not all set"
    else
        row "Mac App Store pkg" "**skipped**" "the upload needs APPLE_API_ISSUER, APPLE_API_KEY and APPLE_API_KEY_P8"
    fi
fi

# An unsigned APK cannot be installed, so building one would cost twenty
# minutes of every tag and produce nothing shippable.
if on HAS_ANDROID; then
    android=true
    row "Android APK + AAB" "builds" "signed with the upload key"
else
    android=false
    row "Android APK + AAB" "**skipped**" "ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD and ANDROID_KEY_BASE64 are not all set"
fi
if [ "$android" = true ] && on HAS_PLAY; then
    play=true
    row "Google Play" "uploads" "the AAB to the \`${PLAY_TRACK:-internal}\` track"
else
    play=false
    if [ "$android" = true ]; then
        row "Google Play" "**skipped**" "GOOGLE_PLAY_SERVICE_ACCOUNT_JSON is not set"
    else
        row "Google Play" "**skipped**" "there is no signed Android build to upload"
    fi
fi

# The Store's installer is built whenever it could be accepted — signed, and
# with a publisher name — whether or not the submission secrets exist yet:
# Partner Center's first submission is made by hand and needs the installer's
# URL on the download mirror before any automation can take over. The Store
# refuses an unsigned installer (Microsoft Store policy 10.2.9).
if [ "$windows_signing" = none ]; then
    store_installer=false
    row "Microsoft Store installer" "**skipped**" "the Store accepts only a signed installer, and Windows signing is not configured"
elif [ -z "${MSSTORE_PUBLISHER:-}" ]; then
    store_installer=false
    row "Microsoft Store installer" "**skipped**" "the MSSTORE_PUBLISHER repository variable (the Partner Center publisher name) is not set"
else
    store_installer=true
    row "Microsoft Store installer" "builds" "signed, WebView2 offline, attached to the release for the download mirror"
fi
if on HAS_MSSTORE && [ "$store_installer" = true ]; then
    msstore=true
    row "Microsoft Store" "submits" "the offline installer, once the download mirror serves it"
else
    msstore=false
    if ! on HAS_MSSTORE; then
        row "Microsoft Store" "**skipped**" "MSSTORE_TENANT_ID, MSSTORE_CLIENT_ID, MSSTORE_CLIENT_SECRET, MSSTORE_SELLER_ID and MSSTORE_PRODUCT_ID are not all set"
    else
        row "Microsoft Store" "**skipped**" "there is no Store installer to submit (see the row above)"
    fi
fi

{
    echo "### Release plan"
    echo
    echo "| Platform | Outcome | Why |"
    echo "| --- | --- | --- |"
    printf '%s\n' "${rows[@]}"
    echo
    echo "Secrets and where each comes from: docs/notes/runbooks/release.md."
} >> "$summary"

printf 'windows_signing=%s\n' "$windows_signing"
printf 'macos=%s\n' "$macos"
printf 'macos_notary=%s\n' "$macos_notary"
printf 'mac_app_store=%s\n' "$mac_app_store"
printf 'android=%s\n' "$android"
printf 'play=%s\n' "$play"
printf 'store_installer=%s\n' "$store_installer"
printf 'msstore=%s\n' "$msstore"
