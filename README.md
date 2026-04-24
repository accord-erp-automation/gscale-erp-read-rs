# gscale-erp-read-rs

## Abstract
`gscale-erp-read-rs` is the Rust implementation of the read-only ERP catalog service used by the GScale system. It exposes a narrow, stable HTTP interface for item and warehouse discovery without granting write access to ERP business documents.

This repository exists as a drop-in Rust replacement for the Go service in `gscale-erp-read`, while keeping the same API contract, response shapes, and ERPNext/MariaDB read behavior.

## Role in the Three-Repository Architecture

The central design decision of the GScale system is the separation of ERP writes from ERP reads.

This repository exists so that:

- item search remains fast and controlled,
- warehouse-related lookup logic can be specialized,
- the mobile client does not need direct awareness of ERP schema details,
- the main runtime can write ERP documents without also becoming the canonical read-model implementation.

In short, this service is the catalog intelligence layer for the broader GScale system.

## Architectural Relationship

```text
gscale-mobile-app
        |
        v
   gscale-platform/mobileapi
        |
        v
   gscale-erp-read-rs
        |
        v
     ERPNext DB
```

The mobile application normally does not call this service directly. Instead, `gscale-platform` calls it and re-exposes catalog operations through mobile-facing endpoints.

## Responsibilities

This repository is intentionally limited to read-only concerns:

- item search,
- item detail lookup,
- item-to-warehouse shortlist lookup,
- warehouse detail lookup,
- warehouse-aware item filtering for default-warehouse workflows.

It deliberately does **not**:

- create ERP drafts,
- submit ERP documents,
- coordinate print requests,
- interact with Zebra printers,
- maintain batch transaction state.

Those responsibilities belong to `gscale-platform`.

## API Surface

### Health

- `GET /healthz`
- `GET /v1/handshake`

### Catalog Endpoints

- `GET /v1/items?query=...&limit=...&warehouse=...`
- `GET /v1/items/{item_code}`
- `GET /v1/items/{item_code}/warehouses?query=...&limit=...`
- `GET /v1/warehouses?query=...&limit=...`
- `GET /v1/warehouses/{warehouse}`

### Important Semantics

`warehouse` on `/v1/items` is not cosmetic. It acts as a real filter when the caller wants the item picker constrained to a default warehouse. That behavior is important for the mobile workflow implemented in `gscale-platform`.

Search behavior is also intentionally richer than a simple `LIKE` query. The Rust implementation keeps the same policy as the Go service:

- normalized token search,
- alias expansion,
- Uzbek transliteration handling,
- fuzzy ranking for short and noisy queries,
- stable ordering by relevance and item code.

## Why This Service Exists Instead of Using ERP REST Directly

Using ERP resource endpoints directly from the runtime layer would be possible, but it would push catalog policy into the wrong place. This repository provides:

- a narrower contract,
- lower coupling to ERP internals,
- easier search policy tuning,
- easier warehouse-aware filtering,
- cleaner testing boundaries.

It therefore serves as the domain-specific read adapter for the broader GScale system.

## Data Source Strategy

The service loads ERP connection metadata from the ERP bench and site configuration:

- `ERP_BENCH_ROOT`
- `ERP_SITE_NAME`
- `ERP_SITE_CONFIG`

When deployed beside the ERP installation, the service can read trusted site configuration and connect directly to MariaDB using the site database credentials.

## Runtime Configuration

The Rust service reads the same environment variables as the Go version:

- `ERP_READ_ADDR`
- `ERP_BENCH_ROOT`
- `ERP_SITE_NAME`
- `ERP_SITE_CONFIG`
- `ERP_DB_HOST`
- `ERP_DB_PORT`
- `ERP_DB_USER`

If `ERP_SITE_NAME` or `ERP_SITE_CONFIG` are not provided, the service reads ERP bench site files:

- `sites/common_site_config.json`
- `sites/{site}/site_config.json`

## Run

From the bench root, or by pointing at it explicitly:

```bash
cargo run
```

Typical development invocation:

```bash
ERP_BENCH_ROOT=/path/to/erp/bench \
ERP_SITE_NAME=erp.localhost \
ERP_READ_ADDR=127.0.0.1:8090 \
cargo run
```

If you want to run against a custom site config file directly:

```bash
ERP_SITE_CONFIG=/path/to/site_config.json \
ERP_DB_USER=erpdb \
cargo run
```

## Testing Philosophy

This repository is validated in two modes:

1. isolated Rust tests:

```bash
cargo test
```

2. integrated tests through `gscale-platform`:

- start `gscale-erp-read-rs`,
- start `gscale-platform/mobileapi`,
- verify `/v1/mobile/items`,
- verify default-warehouse filtering,
- verify item-to-warehouse shortlist results.

The second mode matters because this repository is only one part of the system contract.

## Implementation Notes

The Rust service is implemented with:

- `axum` for HTTP,
- `sqlx` for MariaDB access,
- `tokio` for async runtime and graceful shutdown,
- `serde` and `serde_json` for JSON payloads.

The current Rust code mirrors the Go service behavior for:

- health and handshake endpoints,
- item search and item detail lookup,
- warehouse shortlist generation,
- warehouse list search,
- warehouse detail lookup,
- config loading from ERP bench/site files,
- response envelope shape.

## Recommended Companion Reading

To understand how this repository participates in the full system, read these next:

1. [`gscale-platform`](https://github.com/accord-erp-automation/gscale-platform)
2. [`gscale-mobile-app`](https://github.com/WIKKIwk/gscale-mobile-app)

Those repositories are the operational and UI counterparts of this service.
