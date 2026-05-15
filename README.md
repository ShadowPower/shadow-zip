# Shadow Zip

Shadow Zip is a CLI-first archive manager with a Rust core scaffolded from the product docs in
`archive-product-docs`.

The code is organized around capability-driven archive sessions instead of UI
checks against file extensions:

- `crates/domain`: stable data models, capability levels, errors, options,
  preflight checks, password/conflict models, and path safety helpers.
- `crates/archive-core`: backend traits and backend selection service.
- `crates/archive-zip`, `archive-7z`, `archive-tar`, `archive-rar`,
  `archive-libarchive`: format adapters. Current implementations are ideal
  skeletons and task-plan producers.
- `crates/task-engine`: priority queue and task state aggregation.
- `crates/preview`: staged preview pipeline for metadata, thumbnails,
  fit-window images, text, and external-open temp files.
- `crates/cache`: index, thumbnail, and temp cache models.
- `crates/i18n`: English and Simplified Chinese translations via stable
  message keys.
- `crates/platform`: platform integration boundary and external helper config.
- `crates/app-core`: shared non-UI use cases for archive operations, used by
  the CLI today and a future Flutter + Rust desktop adapter.
- `crates/cli`: command-line interface for headless automation and core
  workflow testing.

## CLI

The CLI binary is `shadow-zip`. It is intentionally a command-line interface,
not a TUI. It uses `app-core` directly, so CLI tests exercise the archive
workflows that a future desktop UI should also call.

Common commands:

```sh
shadow-zip info archive.zip
shadow-zip list archive.zip
shadow-zip tree archive.zip
shadow-zip preflight extract archive.zip --to out
shadow-zip extract archive.zip --to out --rename-existing
shadow-zip create output.zip input-dir --format zip
shadow-zip test archive.zip
shadow-zip cat archive.zip docs/readme.txt
shadow-zip preview archive.zip images/pixel.png --mode metadata --json
shadow-zip backends --json
shadow-zip helpers --json
shadow-zip diagnose archive.zip --json
shadow-zip cache status --json
shadow-zip recent list --json
shadow-zip config get --json
```

Dependencies are intended to come from crates.io. The project-level Cargo config
routes crates.io through the USTC sparse mirror.

## Implementation Policy

Shadow Zip uses third-party crates for format parsing, compression, image
decoding, filesystem traversal, resizing, and persistence wherever practical:

- ZIP: `zip`
- tar: `tar`
- gzip/xz/zstd streams: `flate2`, `xz2`, `zstd`
- image metadata/decode/resize: `image`, `fast_image_resize`
- directory traversal: `walkdir`
- path normalization: `path-clean`
- filesystem errors: `fs-err`
- cache/config persistence: `serde`, `serde_json`

Project-owned code is kept to product-specific coordination:

- capability and error models
- backend selection and fallback
- preflight and extraction safety policy
- task scheduling, cancellation, and progress aggregation
- UI state and localization
- bounded stream adapters between archive entries and safe writers

Large archive data must flow through bounded streams. Code should not read an
entire archive, tar stream, or large entry into memory unless a third-party
decoder requires a bounded in-memory buffer and a product limit explicitly
allows it, as in image preview.

## Testing Shape

Pure logic is kept behind small services and traits so it can be tested without
desktop UI or platform integration:

- `domain`: path safety and archive-bomb policy
- `archive-core`: stream pump, safe writer, preflight
- `task-engine`: priority, cancellation, lifecycle
- `i18n`: locale detection and translation

Backend integration tests should use `tempfile` fixtures and real third-party
writers/readers instead of mocked archive bytes.
