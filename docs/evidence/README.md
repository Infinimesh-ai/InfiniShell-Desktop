# Public documentation and private evidence

[简体中文](README.zh-CN.md) · [Documentation index](../README.md)

Public Git stores reusable guides, product/source identities, public CI links, necessary digests, scoped acceptance results, and failure boundaries. Machine usernames, real storage layouts, personal configuration/database details, private backup mappings, and temporary debugging copies are not shared operating guides.

## private-evidence

`private-evidence:ref-<identifier>` and “private evidence” links bearing that identifier refer to originals retained by the maintainer. The untracked `.local/private-evidence-map.json` maps original locators and field origins, including IDs from records since removed from Git. IDs are derived from locators; **they are not evidence-file SHA-256 digests**. Existing `sha256` fields remain the original recorded content digests.

Before the October 25, 2026 cleanup, relevant files were copied byte for byte and SHA-256 verified; the location is recorded in `.local/docs-cleanup-backup.json`. Retired G09, Draft, and one-off platform acceptance logs are no longer tracked. Originals and intermediate cleanup copies remain local; the public historical ledger is no longer maintained. Current summaries retain untested scope and `cleanup_ready=false`. Removing private material from the current tracked tree does not erase existing Git history; this maintenance does not rewrite history.

A mapping does not prove that an external original remains accessible or was revalidated during this cleanup. Ask the maintainer to review identity, access, and digests when needed. Do not upload raw logs, databases, sessions, credentials, or screenshots merely to make a link public.

## History and fixtures

- Interpret dated reviews and acceptance records against their original source, build, platform, and time. Do not transfer old candidate results to new packages.
- Fixed-version protocol fixtures, signing public keys, and regression data remain tracked with their bytes and digests unchanged. They are test inputs or protocol evidence, not local configuration templates.
- Load machine rules from `.local/README.md`; [shared test gates](../local-test-storage.md) remain effective.
- Retain local originals and backups before untracking temporary debugging files. Ignore rules do not authorize deletion.
