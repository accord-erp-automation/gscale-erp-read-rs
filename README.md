# gscale-erp-read-rs

Rust implementation of the `gscale-erp-read` read-only ERP catalog service.

The goal of this folder is to provide a drop-in replacement for the existing Go service while keeping the same HTTP contract and ERPNext/MariaDB read behavior.

## API Surface

- `GET /healthz`
- `GET /v1/handshake`
- `GET /v1/items?query=...&limit=...&warehouse=...`
- `GET /v1/items/{item_code}`
- `GET /v1/items/{item_code}/warehouses?query=...&limit=...`
- `GET /v1/warehouses/{warehouse}`

## Runtime Configuration

The service reads the same environment variables as the Go version:

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

```bash
ERP_BENCH_ROOT=/path/to/erp/bench \
ERP_SITE_NAME=erp.localhost \
ERP_READ_ADDR=127.0.0.1:8090 \
cargo run
```

## Test

```bash
cargo test
```
