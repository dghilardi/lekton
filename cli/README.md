# lekton-sync

CLI tool to sync markdown documents to a [Lekton](https://github.com/dghilardi/lekton) instance.

It scans a directory for `.md` files, reads their YAML front matter, calls the Lekton sync API to compute the delta, and uploads only the documents that have changed.

## Installation

```sh
cargo install lekton-sync
```

## Usage

```sh
lekton-sync [OPTIONS] [ROOT]
```

| Argument | Default | Description |
|---|---|---|
| `ROOT` | `.` | Root directory to scan for markdown files |
| `--archive-missing` | — | Archive documents present in Lekton but not found locally |
| `--dry-run` | — | Show what would be done without making any changes |
| `--config <PATH>` | `<ROOT>/.lekton.yml` | Path to config file |
| `-v, --verbose` | — | Verbose output |

### Environment variables

| Variable | Required | Description |
|---|---|---|
| `LEKTON_TOKEN` | Yes | Service token for authentication |
| `LEKTON_URL` | Yes* | Base URL of the Lekton server |

*Can also be set via `url` in `.lekton.yml`.

## Document format

Each markdown file must have a YAML front matter block. Files without front matter are skipped.

```markdown
---
title: My Document
slug: optional/custom-slug        # defaults to file path relative to root
access_level: public               # defaults to "public"
service_owner: my-team             # optional
tags: [guide, onboarding]          # optional
order: 10                          # optional, for ordering within a section
is_hidden: false                   # optional
---

Document body...
```

The slug is derived from the file path relative to the root directory (e.g. `docs/guides/intro.md` → `docs/guides/intro`), unless overridden by the `slug` field in the front matter.

## Configuration file

Place a `.lekton.yml` file in the root directory (or pass `--config`) to set project-level defaults:

```yaml
url: https://lekton.example.com
default_access_level: internal
default_service_owner: platform-team
slug_prefix: protocols/my-service
archive_missing: false
```

| Field | Description |
|---|---|
| `url` | Base URL of the Lekton server (overridden by `LEKTON_URL`) |
| `default_access_level` | Fallback access level when not set in front matter |
| `default_service_owner` | Fallback service owner when not set in front matter |
| `slug_prefix` | Prefix prepended to every document slug |
| `archive_missing` | Archive documents not found locally (overridden by `--archive-missing`) |

## Example

```sh
export LEKTON_TOKEN=my-service-token
export LEKTON_URL=https://lekton.example.com

# Preview what would change
lekton-sync --dry-run ./docs

# Sync and archive documents no longer present locally
lekton-sync --archive-missing ./docs
```

## License

AGPL-3.0
