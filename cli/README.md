# lekton-sync

CLI tool to sync markdown documents and prompt definitions to a [Lekton](https://github.com/dghilardi/lekton) instance.

It scans a directory for `.md` files and prompt YAML files, calls the Lekton sync APIs to compute the delta, and uploads only the content that has changed.

## Installation

```sh
cargo install lekton-sync
```

### Docker

Build the default minimal container image from the repository root:

```sh
docker build -f cli/Dockerfile -t lekton-sync .
```

Build a Jenkins-friendly variant with a `debian:bookworm-slim` runtime:

```sh
docker build -f cli/Dockerfile --target ci -t lekton-sync-ci .
```

Run it by mounting the documentation workspace and passing the usual environment variables:

```sh
docker run --rm \
  -e LEKTON_URL=https://lekton.example.com \
  -e LEKTON_TOKEN=your-service-token \
  -v "$PWD:/workspace" \
  lekton-sync /workspace
```

This image contains only the `lekton-sync` binary in a distroless runtime, so it fits well in documentation CI pipelines without installing Rust toolchains on the runner.

When you need a shell-capable image for CI systems that keep sidecars alive with commands such as `sleep infinity`, use the `ci` target instead. It ships the same `lekton-sync` binary on top of `debian:bookworm-slim`.

Tagged releases can also publish a ready-to-use Docker image via GitHub Actions:

```sh
docker run --rm \
  -e LEKTON_URL=https://lekton.example.com \
  -e LEKTON_TOKEN=your-service-token \
  -v "$PWD:/workspace" \
  docker.io/<your-dockerhub-namespace>/lekton-sync:0.13.0 /workspace
```

The publish workflow also builds a companion `lekton-sync-ci` image for Jenkins/Kubernetes-style runners.

## Usage

```sh
lekton-sync [OPTIONS] [ROOT]
```

| Argument | Default | Description |
|---|---|---|
| `ROOT` | `.` | Root directory to scan for markdown files and prompt definitions |
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

## Prompt format

Prompt definitions are loaded from `prompts/*.yaml` by default. This directory can be changed via `.lekton.yml`.

```yaml
name: Code Review
description: Review a patch before merge
owner: platform-team
access_level: developer
status: active
publish_to_mcp: true
default_primary: true
context_cost: medium
tags: [engineering, review]
variables:
  - name: diff
    description: Unified diff to inspect
    required: true
prompt_body: |
  Review the following diff:
  {{diff}}
```

Supported prompt fields:

| Field | Required | Description |
|---|---|---|
| `name` | No | Human-readable prompt name. Defaults to the file slug segment. |
| `description` | Yes | Short description shown in Lekton. |
| `owner` | Yes* | Owning team or service. Falls back to `default_service_owner` if configured. |
| `access_level` | No | Falls back to `default_access_level`, then `public`. |
| `status` | No | Prompt lifecycle status. Defaults to `active`. |
| `tags` | No | Optional tags array. |
| `variables` | No | Optional variable descriptors (`name`, `description`, `required`). |
| `publish_to_mcp` | No | Publish this prompt to MCP discovery/context. Defaults to `false`. |
| `default_primary` | No | Include in the default MCP context unless hidden by the user. Defaults to `false`. |
| `context_cost` | No | Prompt context weight hint. Defaults to `medium`. |
| `slug` | No | Overrides the slug derived from the prompt file path. |
| `lekton-import` | No | Set to `false` to skip the prompt. |
| `prompt_body` | Yes | Raw prompt body uploaded to Lekton. |

The prompt slug is derived from the path relative to the prompt directory and prefixed with `prompts/` by default (for example `prompts/code-review.yaml` → `prompts/code-review`).

## Configuration file

Place a `.lekton.yml` file in the root directory (or pass `--config`) to set project-level defaults:

```yaml
url: https://lekton.example.com
default_access_level: internal
default_service_owner: platform-team
slug_prefix: protocols/my-service
prompts_dir: prompts
prompt_slug_prefix: prompts
archive_missing: false
```

| Field | Description |
|---|---|
| `url` | Base URL of the Lekton server (overridden by `LEKTON_URL`) |
| `default_access_level` | Fallback access level when not set in front matter |
| `default_service_owner` | Fallback service owner when not set in front matter |
| `slug_prefix` | Prefix prepended to every document slug |
| `prompts_dir` | Directory containing prompt YAML files, relative to `ROOT` |
| `prompt_slug_prefix` | Prefix prepended to every prompt slug |
| `archive_missing` | Archive documents not found locally (overridden by `--archive-missing`) |

## Example

```sh
export LEKTON_TOKEN=my-service-token
export LEKTON_URL=https://lekton.example.com

# Preview what would change
lekton-sync --dry-run ./docs

# Sync and archive documents/prompts no longer present locally
lekton-sync --archive-missing ./docs
```

### GitHub Actions example

```yaml
- name: Sync docs to Lekton
  run: |
    docker run --rm \
      -e LEKTON_URL="${{ secrets.LEKTON_URL }}" \
      -e LEKTON_TOKEN="${{ secrets.LEKTON_TOKEN }}" \
      -v "${{ github.workspace }}:/workspace" \
      docker.io/${{ secrets.DOCKERHUB_USERNAME }}/lekton-sync:${{ github.ref_name }} \
      --archive-missing /workspace/docs
```

## License

AGPL-3.0
