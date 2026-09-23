# Release Configurations

This README file documents the format of the `release_configurations.json` file located in this directory.  The file defines InfiniShell-Desktop's various release channels, and provides values for the various variables that are necessary to run the `create_new_releases.yml` GitHub workflow.

At some point, we may want to replace this document with a JSON schema file (which could be used to validate the correctness of the configuration as part of PR presubmit).

## Fields

* **channel**: The channel's unique identifier
* **type**: The release cadence.  At present, the valid values are "nightly" or "weekly".
* **is_prerelease**: If true, the GitHub release for this channel will be marked as prerelease.
* **is_autopush**: If true, this channel uses the "latest" keyword in `channel_versions.json` to automatically deploy new release candidates.  Non-autopush channels require a manual change in order to deploy them.
* **release_base_name**: The base name of GitHub releases created for this channel.
* **release_body_text**: The body text for GitHub releases created for this channel.
* **changelog_slack_channel**: The Slack channel where new changelogs will be posted whenever a new release candidates is cut.
* **gcs_cache_control_value**: The value of the cache-control response header for release DMGs.
  - **IMPORTANT!!**: the value of the cache-control header _must_ be all lowercase; uppercase values will not be respected by Cloud CDN.

## InfiniShell OSS macOS architecture policy

InfiniShell OSS desktop releases support Apple Silicon (`arm64`) only. The release
workflow publishes `InfiniShell-arm64.dmg` and does not build or publish an Intel
desktop DMG. The macOS `x86_64` target remains in the remote-server matrix solely
for the SSH extension; it is built natively on an Intel runner. An optional
preflight job retains Apple Silicon cross-compilation plus Intel native acceptance
as a fallback when a long-running Intel build is unavailable.
