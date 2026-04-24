use std::{collections::HashMap, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;

use crate::store::{CatalogStore, StoreError};

#[derive(Clone)]
pub struct AppState {
    store: Arc<dyn CatalogStore>,
}

impl AppState {
    pub fn new(store: Arc<dyn CatalogStore>) -> Self {
        Self { store }
    }
}

pub fn router(store: Arc<dyn CatalogStore>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/handshake", get(handshake))
        .route("/v1/items", get(search_items))
        .route("/v1/items/{item_code}", get(get_item))
        .route(
            "/v1/items/{item_code}/warehouses",
            get(search_item_warehouses),
        )
        .route("/v1/warehouses", get(search_warehouses))
        .route("/v1/warehouses/{warehouse}", get(get_warehouse))
        .with_state(AppState::new(store))
}

async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn handshake() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "service": "gscale_erp_read",
    }))
}

async fn search_items(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<DataResponse<Vec<crate::store::Item>>>, ApiError> {
    let query = params.get("query").map(String::as_str).unwrap_or_default();
    let limit = parse_limit(params.get("limit"));
    let warehouse = params
        .get("warehouse")
        .map(String::as_str)
        .unwrap_or_default();

    let data = state
        .store
        .search_items(query, limit, warehouse)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DataResponse { data }))
}

async fn get_item(
    State(state): State<AppState>,
    Path(item_code): Path<String>,
) -> Result<Json<DataResponse<crate::store::ItemDetail>>, ApiError> {
    let item_code = item_code.trim();
    if item_code.is_empty() {
        return Err(ApiError::bad_request("item_code is required"));
    }

    let data = state
        .store
        .get_item(item_code)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DataResponse { data }))
}

async fn search_item_warehouses(
    State(state): State<AppState>,
    Path(item_code): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<DataResponse<Vec<crate::store::WarehouseStock>>>, ApiError> {
    let item_code = item_code.trim();
    if item_code.is_empty() {
        return Err(ApiError::bad_request("item_code is required"));
    }

    let query = params.get("query").map(String::as_str).unwrap_or_default();
    let limit = parse_limit(params.get("limit"));
    let data = state
        .store
        .search_item_warehouses(item_code, query, limit)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DataResponse { data }))
}

async fn get_warehouse(
    State(state): State<AppState>,
    Path(warehouse): Path<String>,
) -> Result<Json<DataResponse<crate::store::Warehouse>>, ApiError> {
    let warehouse = warehouse.trim();
    if warehouse.is_empty() {
        return Err(ApiError::bad_request("warehouse is required"));
    }

    let data = state
        .store
        .get_warehouse(warehouse)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DataResponse { data }))
}

async fn search_warehouses(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<DataResponse<Vec<crate::store::Warehouse>>>, ApiError> {
    let query = params.get("query").map(String::as_str).unwrap_or_default();
    let limit = parse_limit(params.get("limit"));
    let data = state
        .store
        .search_warehouses(query, limit)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(DataResponse { data }))
}

fn parse_limit(raw: Option<&String>) -> i64 {
    raw.and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_default()
}

#[derive(Serialize)]
struct DataResponse<T> {
    data: T,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        let status = match value {
            StoreError::EmptyItemCode | StoreError::EmptyWarehouse => StatusCode::BAD_REQUEST,
            StoreError::ItemNotFound(_) | StoreError::WarehouseNotFound(_) => StatusCode::NOT_FOUND,
            StoreError::SearchItems(_)
            | StoreError::GetItem(_)
            | StoreError::SearchWarehouses(_)
            | StoreError::GetWarehouse(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        Self {
            status,
            message: value.to_string().trim().to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use crate::store::{Item, ItemDetail, Warehouse, WarehouseStock};

    #[derive(Clone, Default)]
    struct FakeStore {
        items: Vec<Item>,
        stocks: Vec<WarehouseStock>,
        item: Option<ItemDetail>,
        warehouse: Option<Warehouse>,
        warehouses: Vec<Warehouse>,
    }

    #[async_trait]
    impl CatalogStore for FakeStore {
        async fn search_items(
            &self,
            query: &str,
            limit: i64,
            warehouse: &str,
        ) -> Result<Vec<Item>, StoreError> {
            assert_eq!(query, "itm");
            assert_eq!(limit, 10);
            assert_eq!(warehouse, "Stores - A");
            Ok(self.items.clone())
        }

        async fn search_item_warehouses(
            &self,
            _item_code: &str,
            _query: &str,
            _limit: i64,
        ) -> Result<Vec<WarehouseStock>, StoreError> {
            Ok(self.stocks.clone())
        }

        async fn get_item(&self, _item_code: &str) -> Result<ItemDetail, StoreError> {
            Ok(self.item.clone().expect("fake item"))
        }

        async fn search_warehouses(
            &self,
            _query: &str,
            _limit: i64,
        ) -> Result<Vec<Warehouse>, StoreError> {
            Ok(self.warehouses.clone())
        }

        async fn get_warehouse(&self, _warehouse: &str) -> Result<Warehouse, StoreError> {
            Ok(self.warehouse.clone().expect("fake warehouse"))
        }
    }

    #[tokio::test]
    async fn items_endpoint() {
        let app = router(Arc::new(FakeStore {
            items: vec![Item {
                name: "ITM-001".to_string(),
                item_code: "ITM-001".to_string(),
                item_name: "Item 1".to_string(),
            }],
            ..FakeStore::default()
        }));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/items?query=itm&limit=10&warehouse=Stores%20-%20A")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["data"][0]["item_code"], "ITM-001");
    }

    #[tokio::test]
    async fn handshake_endpoint() {
        let app = router(Arc::new(FakeStore::default()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/handshake")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["service"], "gscale_erp_read");
    }

    #[tokio::test]
    async fn warehouses_endpoint() {
        let app = router(Arc::new(FakeStore {
            stocks: vec![WarehouseStock {
                warehouse: "Stores - A".to_string(),
                actual_qty: 12.5,
            }],
            ..FakeStore::default()
        }));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/items/ITM-001/warehouses")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["data"][0]["warehouse"], "Stores - A");
    }

    #[tokio::test]
    async fn item_detail_endpoint() {
        let app = router(Arc::new(FakeStore {
            item: Some(ItemDetail {
                name: "ITM-001".to_string(),
                item_code: "ITM-001".to_string(),
                item_name: "Item 1".to_string(),
                stock_uom: "Kg".to_string(),
            }),
            ..FakeStore::default()
        }));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/items/ITM-001")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn warehouse_detail_endpoint() {
        let app = router(Arc::new(FakeStore {
            warehouse: Some(Warehouse {
                name: "Stores - A".to_string(),
                company: "A Company".to_string(),
            }),
            ..FakeStore::default()
        }));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/warehouses/Stores%20-%20A")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn warehouse_list_endpoint() {
        let app = router(Arc::new(FakeStore {
            warehouses: vec![Warehouse {
                name: "Stores - A".to_string(),
                company: "A Company".to_string(),
            }],
            ..FakeStore::default()
        }));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/warehouses?query=stores&limit=10")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["data"][0]["name"], "Stores - A");
        assert_eq!(payload["data"][0]["company"], "A Company");
    }
}
