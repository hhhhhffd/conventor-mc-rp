<div align="center">

<h1>packconv</h1>
<hr>

**Minecraft resource pack converter in one codebase: HTTP server, in-browser WebAssembly, pack repair**

![Rust](https://img.shields.io/badge/RUST-2021-DEA584?style=flat-square&logo=rust&logoColor=white)
![WebAssembly](https://img.shields.io/badge/WEBASSEMBLY-wasm--bindgen%200.2.126-654FF0?style=flat-square&logo=webassembly&logoColor=white)
![Axum](https://img.shields.io/badge/AXUM-0.8-000000?style=flat-square)
[![Pages](https://img.shields.io/github/actions/workflow/status/hhhhhffd/conventor-mc-rp/pages.yml?style=flat-square&label=PAGES)](https://github.com/hhhhhffd/conventor-mc-rp/actions/workflows/pages.yml)
[![License](https://img.shields.io/github/license/hhhhhffd/conventor-mc-rp?style=flat-square)](LICENSE)

[Website](https://hhhhhffd.github.io/conventor-mc-rp/) · [Stack](#technical-stack) · [Quick start](#quick-start)

</div>

## What it is

packconv converts Minecraft Java Edition resource packs between versions 1.7.2 and 26.3, in both directions. It also has a repair mode for badly saved files (PNG, JSON, animations) that old versions tolerated and new ones may reject.

It is pure Rust: ZIP and RAR (1.3, 4, 5) archives are unpacked in memory, with no external programs and no C dependencies. The same core runs as an axum HTTP server and as a WebAssembly module in the browser, where the pack is never uploaded.

## Technical Stack

| Layer | Technology |
|---|---|
| Core | Rust 2021, crate `packconv-core` (`zip`, `image`, `png`, `rars`, `serde_json`) |
| Server | `packconv-server`: axum 0.8, tokio |
| Browser | `packconv-wasm`: wasm-bindgen 0.2.126, static HTML in `web/`, converter runs in a Web Worker |
| CI/CD | GitHub Actions builds the wasm and deploys GitHub Pages on every push to `main` |

## Quick start

```bash
cargo build --release -p packconv-server    # binary: target/release/packconv
target/release/packconv                     # listens on 0.0.0.0:8080
curl --data-binary @pack.rar "http://localhost:8080/convert?to=26.3" -o out.zip
```

Browser build (needs the `wasm-bindgen` CLI 0.2.126):

```bash
scripts/build-wasm.sh web && cp -r pkg web/pkg   # then serve web/ with any static server
```

Tests:

```bash
cargo test -p packconv-core
scripts/e2e.sh <directory with packs>            # end-to-end run on real packs
```

<details>
<summary>HTTP API and configuration</summary>

| Method | Path | Body | Response |
|---|---|---|---|
| `GET` | `/health` | none | `ok` |
| `GET` | `/versions` | none | JSON array of supported versions |
| `POST` | `/convert?to=<version>[&from=<version>]` | ZIP or RAR as is | ZIP; summary in the `x-packconv-report` header |
| `POST` | `/repair` | ZIP or RAR as is | ZIP; summary in the `x-packconv-report` header |

If `from` is omitted, the source version is detected from `pack.mcmeta`. Errors: `413` body over the limit, `422` bad archive or version, `429` queue full, `504` job timed out.

| Variable | Default | Purpose |
|---|---|---|
| `PACKCONV_ADDR` | `0.0.0.0:8080` | listen address |
| `PACKCONV_MAX_JOBS` | `1` | concurrent conversions |
| `PACKCONV_MAX_QUEUE` | `4 x jobs` | queued requests before `429` |
| `PACKCONV_MAX_UPLOAD` | 128 MiB | request body size |
| `PACKCONV_MAX_UNPACKED` | 512 MiB | total unpacked size |
| `PACKCONV_MAX_ENTRIES` | 100 000 | files per archive |
| `PACKCONV_TIMEOUT_SECS` | 300 | job timeout |

> [!NOTE]
> Defaults target a small single-core VPS. Cancellation is checked between conversion steps, not inside one.

Conversion limits: shaders (`shaders/`, `post_effect/`) are tied to the engine version and are removed with a warning when the version changes. Conversions that cannot keep data are listed in the `warnings` field of the report. Only vanilla assets are processed (`pack.mcmeta`, `pack.png`, `assets/minecraft/`, `assets/realms/`); everything else passes through unchanged.

</details>

## License

Licensed under [AGPL-3.0](LICENSE), copyright 2026 hhhhhffd. Mapping tables from MIT-licensed projects keep their own licenses, see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
