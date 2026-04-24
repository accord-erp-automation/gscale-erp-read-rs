use async_trait::async_trait;
use serde::Serialize;
use sqlx::{FromRow, MySqlPool};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
pub struct Item {
    pub name: String,
    pub item_code: String,
    pub item_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
pub struct ItemDetail {
    pub name: String,
    pub item_code: String,
    pub item_name: String,
    pub stock_uom: String,
}

#[derive(Debug, Clone, PartialEq, FromRow, Serialize)]
pub struct WarehouseStock {
    pub warehouse: String,
    pub actual_qty: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
pub struct Warehouse {
    pub name: String,
    pub company: String,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("search items query: {0}")]
    SearchItems(sqlx::Error),
    #[error("get item query: {0}")]
    GetItem(sqlx::Error),
    #[error("search warehouses query: {0}")]
    SearchWarehouses(sqlx::Error),
    #[error("get warehouse query: {0}")]
    GetWarehouse(sqlx::Error),
    #[error("item code is empty")]
    EmptyItemCode,
    #[error("item topilmadi: {0}")]
    ItemNotFound(String),
    #[error("warehouse is empty")]
    EmptyWarehouse,
    #[error("warehouse topilmadi: {0}")]
    WarehouseNotFound(String),
}

#[async_trait]
pub trait CatalogStore: Send + Sync + 'static {
    async fn search_items(
        &self,
        query: &str,
        limit: i64,
        warehouse: &str,
    ) -> Result<Vec<Item>, StoreError>;

    async fn search_item_warehouses(
        &self,
        item_code: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<WarehouseStock>, StoreError>;

    async fn get_item(&self, item_code: &str) -> Result<ItemDetail, StoreError>;

    async fn get_warehouse(&self, warehouse: &str) -> Result<Warehouse, StoreError>;
}

#[derive(Clone)]
pub struct Store {
    pool: MySqlPool,
}

impl Store {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CatalogStore for Store {
    async fn search_items(
        &self,
        query: &str,
        limit: i64,
        warehouse: &str,
    ) -> Result<Vec<Item>, StoreError> {
        let warehouse = warehouse.trim();
        let terms = search_terms(query);
        let mut args = Vec::new();

        let mut sql_text = String::from(
            r#"
SELECT
    name,
    COALESCE(NULLIF(item_code, ''), name) AS item_code,
    COALESCE(NULLIF(item_name, ''), NULLIF(item_code, ''), name) AS item_name
FROM tabItem
"#,
        );

        let mut where_added = false;
        if !warehouse.is_empty() {
            sql_text.push_str(
                r#"
WHERE EXISTS (
    SELECT 1
    FROM `tabItem Default` item_default
    WHERE item_default.parent = tabItem.name
        AND item_default.default_warehouse = ?
)
"#,
            );
            args.push(SqlArg::String(warehouse.to_string()));
            where_added = true;
        }

        if !terms.is_empty() {
            if where_added {
                sql_text.push_str("\nAND (\n");
            } else {
                sql_text.push_str("\nWHERE (\n");
            }

            let mut filter_added = false;
            for term in &terms {
                let term = normalized_search_text(term);
                if term.is_empty() {
                    continue;
                }
                let compact = compact_field(&term);
                if compact.is_empty() {
                    continue;
                }
                if filter_added {
                    sql_text.push_str("\n    OR\n");
                }
                sql_text.push_str(
                    r#"(
        LOWER(COALESCE(NULLIF(tabItem.item_code, ''), tabItem.name)) LIKE ?
        OR LOWER(COALESCE(NULLIF(tabItem.item_name, ''), COALESCE(NULLIF(tabItem.item_code, ''), tabItem.name))) LIKE ?
        OR LOWER(tabItem.name) LIKE ?
        OR REPLACE(REPLACE(REPLACE(LOWER(COALESCE(NULLIF(tabItem.item_code, ''), tabItem.name)), ' ', ''), '-', ''), '_', '') LIKE ?
        OR REPLACE(REPLACE(REPLACE(LOWER(COALESCE(NULLIF(tabItem.item_name, ''), COALESCE(NULLIF(tabItem.item_code, ''), tabItem.name))), ' ', ''), '-', ''), '_', '') LIKE ?
        OR REPLACE(REPLACE(REPLACE(LOWER(tabItem.name), ' ', ''), '-', ''), '_', '') LIKE ?
    )"#,
                );
                args.extend([
                    SqlArg::String(format!("%{term}%")),
                    SqlArg::String(format!("%{term}%")),
                    SqlArg::String(format!("%{term}%")),
                    SqlArg::String(format!("%{compact}%")),
                    SqlArg::String(format!("%{compact}%")),
                    SqlArg::String(format!("%{compact}%")),
                ]);
                filter_added = true;
            }
            sql_text.push_str("\n)\n");
        }

        sql_text.push_str("\nORDER BY modified DESC\n");
        if limit > 0 {
            sql_text.push_str("LIMIT ?\n");
            args.push(SqlArg::I64(limit));
        }

        let mut query_builder = sqlx::query_as::<_, Item>(&sql_text);
        for arg in args {
            query_builder = arg.bind_to(query_builder);
        }

        let mut items = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::SearchItems)?;

        normalize_items(&mut items);
        items.retain(|item| !item.item_code.is_empty());

        if !terms.is_empty() {
            items = rank_items(items, &terms);
            if limit > 0 && items.len() > limit as usize {
                items.truncate(limit as usize);
            }
        }

        Ok(items)
    }

    async fn get_item(&self, item_code: &str) -> Result<ItemDetail, StoreError> {
        let item_code = item_code.trim();
        if item_code.is_empty() {
            return Err(StoreError::EmptyItemCode);
        }

        let mut item = sqlx::query_as::<_, ItemDetail>(
            r#"
SELECT
    name,
    COALESCE(NULLIF(item_code, ''), name) AS item_code,
    COALESCE(NULLIF(item_name, ''), NULLIF(item_code, ''), name) AS item_name,
    COALESCE(NULLIF(stock_uom, ''), '') AS stock_uom
FROM tabItem
WHERE item_code = ? OR name = ?
LIMIT 1
"#,
        )
        .bind(item_code)
        .bind(item_code)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::GetItem)?
        .ok_or_else(|| StoreError::ItemNotFound(item_code.to_string()))?;

        trim_item_detail(&mut item);
        if item.item_name.is_empty() {
            item.item_name.clone_from(&item.item_code);
        }
        Ok(item)
    }

    async fn search_item_warehouses(
        &self,
        item_code: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<WarehouseStock>, StoreError> {
        let item_code = item_code.trim();
        if item_code.is_empty() {
            return Err(StoreError::EmptyItemCode);
        }

        let limit = normalize_limit(limit);
        let query = query.trim();
        let mut args = vec![
            SqlArg::String(item_code.to_string()),
            SqlArg::String(item_code.to_string()),
            SqlArg::String(item_code.to_string()),
        ];

        let mut sql_text = String::from(
            r#"
SELECT warehouse, CAST(MAX(actual_qty) AS DOUBLE) AS actual_qty
FROM (
    SELECT warehouse, CAST(actual_qty AS DOUBLE) AS actual_qty
    FROM tabBin
    WHERE item_code = ? AND actual_qty > 0

    UNION ALL

    SELECT DISTINCT
        item_default.default_warehouse AS warehouse,
        CAST(0 AS DOUBLE) AS actual_qty
    FROM tabItem
    INNER JOIN `tabItem Default` item_default
        ON item_default.parent = tabItem.name
    WHERE (tabItem.item_code = ? OR tabItem.name = ?)
        AND COALESCE(NULLIF(item_default.default_warehouse, ''), '') <> ''
) warehouse_options
WHERE 1 = 1
"#,
        );

        if !query.is_empty() {
            sql_text.push_str("AND warehouse LIKE ?\n");
            args.push(SqlArg::String(format!("%{query}%")));
        }

        sql_text.push_str(
            r#"
GROUP BY warehouse
ORDER BY actual_qty DESC, warehouse ASC
LIMIT ?
"#,
        );
        args.push(SqlArg::I64(limit));

        let mut query_builder = sqlx::query_as::<_, WarehouseStock>(&sql_text);
        for arg in args {
            query_builder = arg.bind_to(query_builder);
        }

        let mut stocks = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::SearchWarehouses)?;
        for stock in &mut stocks {
            stock.warehouse = stock.warehouse.trim().to_string();
        }
        stocks.retain(|stock| !stock.warehouse.is_empty());
        Ok(stocks)
    }

    async fn get_warehouse(&self, warehouse: &str) -> Result<Warehouse, StoreError> {
        let warehouse = warehouse.trim();
        if warehouse.is_empty() {
            return Err(StoreError::EmptyWarehouse);
        }

        let mut out = sqlx::query_as::<_, Warehouse>(
            r#"
SELECT
    name,
    COALESCE(NULLIF(company, ''), '') AS company
FROM tabWarehouse
WHERE name = ?
LIMIT 1
"#,
        )
        .bind(warehouse)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::GetWarehouse)?
        .ok_or_else(|| StoreError::WarehouseNotFound(warehouse.to_string()))?;

        out.name = out.name.trim().to_string();
        out.company = out.company.trim().to_string();
        Ok(out)
    }
}

enum SqlArg {
    String(String),
    I64(i64),
}

impl SqlArg {
    fn bind_to<'q, O>(
        self,
        query: sqlx::query::QueryAs<'q, sqlx::MySql, O, sqlx::mysql::MySqlArguments>,
    ) -> sqlx::query::QueryAs<'q, sqlx::MySql, O, sqlx::mysql::MySqlArguments> {
        match self {
            Self::String(value) => query.bind(value),
            Self::I64(value) => query.bind(value),
        }
    }
}

fn normalize_items(items: &mut [Item]) {
    for item in items {
        item.name = item.name.trim().to_string();
        item.item_code = item.item_code.trim().to_string();
        item.item_name = item.item_name.trim().to_string();
        if item.item_name.is_empty() {
            item.item_name.clone_from(&item.item_code);
        }
    }
}

fn trim_item_detail(item: &mut ItemDetail) {
    item.name = item.name.trim().to_string();
    item.item_code = item.item_code.trim().to_string();
    item.item_name = item.item_name.trim().to_string();
    item.stock_uom = item.stock_uom.trim().to_string();
}

fn normalize_limit(limit: i64) -> i64 {
    if limit <= 0 {
        20
    } else if limit > 50 {
        50
    } else {
        limit
    }
}

fn search_terms(query: &str) -> Vec<String> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut seen = Vec::<String>::new();
    add_with_variants(query, &mut seen);
    add_with_variants(&transliterate_uzbek(query), &mut seen);
    seen
}

fn add_with_variants(value: &str, out: &mut Vec<String>) {
    let value = normalized_search_text(value);
    if value.is_empty() {
        return;
    }

    let mut queue = vec![value];
    while let Some(current) = queue.first().cloned() {
        queue.remove(0);
        if out.iter().any(|existing| existing == &current) {
            continue;
        }
        append_unique_search_value(out, &current);
        for variant in search_alias_variants(&current) {
            let variant = normalized_search_text(&variant);
            if variant.is_empty() || out.iter().any(|existing| existing == &variant) {
                continue;
            }
            queue.push(variant);
        }
    }
}

fn search_alias_variants(value: &str) -> Vec<String> {
    let value = normalized_search_text(value);
    if value.is_empty() {
        return Vec::new();
    }

    let tokens = value.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut phrases = vec![String::new()];
    for token in tokens {
        let variants = token_alias_variants(token);
        let mut next = Vec::with_capacity(phrases.len() * variants.len());
        for phrase in &phrases {
            for variant in &variants {
                let combined = [phrase.as_str(), variant.as_str()]
                    .join(" ")
                    .trim()
                    .to_string();
                if combined.is_empty() {
                    continue;
                }
                append_unique_search_value(&mut next, &combined);
                if next.len() >= 16 {
                    break;
                }
            }
            if next.len() >= 16 {
                break;
            }
        }
        phrases = next;
        if phrases.is_empty() {
            break;
        }
    }

    let mut out = Vec::with_capacity(phrases.len() * 2);
    for phrase in phrases {
        append_unique_search_value(&mut out, &phrase);
        append_unique_search_value(&mut out, &compact_field(&phrase));
    }
    out
}

fn token_alias_variants(token: &str) -> Vec<String> {
    let token = normalized_search_text(token);
    if token.is_empty() {
        return Vec::new();
    }

    let mut out = vec![token.clone()];
    if token.starts_with('x') && token.len() > 1 {
        append_unique_search_value(&mut out, &format!("h{}", &token[1..]));
    }
    if token.starts_with('h') && token.len() > 1 {
        append_unique_search_value(&mut out, &format!("x{}", &token[1..]));
    }

    for (from, to) in [
        ("lanch", "lunch"),
        ("lunch", "lanch"),
        ("launch", "lanch"),
        ("launch", "lunch"),
    ] {
        if token.contains(from) {
            append_unique_search_value(&mut out, &token.replace(from, to));
        }
    }

    out
}

fn append_unique_search_value(values: &mut Vec<String>, value: &str) {
    let value = normalized_search_text(value);
    if value.is_empty() || values.iter().any(|existing| existing == &value) {
        return;
    }
    values.push(value);
}

fn normalized_search_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_space = false;
    for ch in value.trim().to_lowercase().chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if (ch.is_whitespace() || matches!(ch, '-' | '_' | '\'' | '`' | '’')) && !last_space
        {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn transliterate_uzbek(value: &str) -> String {
    let mut out = value.to_lowercase();
    for (from, to) in [
        ("o'", "o"),
        ("g'", "g"),
        ("sh", "s"),
        ("ch", "c"),
        ("yo", "io"),
        ("yu", "iu"),
        ("ya", "ia"),
        ("ё", "yo"),
        ("ю", "yu"),
        ("я", "ya"),
        ("ш", "sh"),
        ("ч", "ch"),
        ("ғ", "g"),
        ("ў", "o"),
        ("қ", "q"),
        ("ҳ", "h"),
        ("й", "y"),
        ("ц", "s"),
        ("ы", "i"),
        ("э", "e"),
        ("ъ", ""),
        ("ь", ""),
        ("а", "a"),
        ("б", "b"),
        ("в", "v"),
        ("г", "g"),
        ("д", "d"),
        ("е", "e"),
        ("ж", "j"),
        ("з", "z"),
        ("и", "i"),
        ("к", "k"),
        ("л", "l"),
        ("м", "m"),
        ("н", "n"),
        ("о", "o"),
        ("п", "p"),
        ("р", "r"),
        ("с", "s"),
        ("т", "t"),
        ("у", "u"),
        ("ф", "f"),
        ("х", "x"),
    ] {
        out = out.replace(from, to);
    }
    out
}

fn rank_items(items: Vec<Item>, terms: &[String]) -> Vec<Item> {
    let mut scored = items
        .into_iter()
        .filter_map(|item| {
            let score = score_item_match(&item, terms);
            (score > 0).then_some((item, score))
        })
        .collect::<Vec<_>>();

    scored.sort_by(|(left_item, left_score), (right_item, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_item.item_code.cmp(&right_item.item_code))
    });

    scored.into_iter().map(|(item, _)| item).collect()
}

fn score_item_match(item: &Item, terms: &[String]) -> i32 {
    let fields = [
        normalized_search_text(&item.item_code),
        normalized_search_text(&item.item_name),
        normalized_search_text(&item.name),
        normalized_search_text(&transliterate_uzbek(&item.item_code)),
        normalized_search_text(&transliterate_uzbek(&item.item_name)),
        normalized_search_text(&transliterate_uzbek(&item.name)),
    ];

    terms
        .iter()
        .map(|term| {
            fields
                .iter()
                .map(|field| fuzzy_field_score(field, term))
                .max()
                .unwrap_or_default()
        })
        .sum()
}

fn fuzzy_field_score(field: &str, term: &str) -> i32 {
    let field = normalized_search_text(field);
    let term = normalized_search_text(term);
    if field.is_empty() || term.is_empty() {
        return 0;
    }

    let field_compact = compact_field(&field);
    let term_compact = compact_field(&term);
    let short_term = term_compact.chars().count() <= 3;

    if field == term {
        120
    } else if field.starts_with(&term) {
        100
    } else if field_compact == term_compact {
        99
    } else if field_compact.starts_with(&term_compact) {
        98
    } else if field.contains(&format!(" {term}")) {
        90
    } else if !short_term && field.contains(&term) {
        75
    } else if !short_term && field_compact.contains(&term_compact) {
        72
    } else if !short_term && token_typo_score(&field, &term) > 0 {
        token_typo_score(&field, &term)
    } else if !short_term && subsequence_match(&field_compact, &term_compact) {
        55
    } else if !short_term && levenshtein_distance(&field, &term) <= 1 {
        45
    } else if !short_term && levenshtein_distance(&field_compact, &term_compact) <= 1 {
        44
    } else if !short_term && levenshtein_distance(first_token(&field), &term) <= 1 {
        40
    } else if !short_term && levenshtein_distance(first_token(&field_compact), &term_compact) <= 1 {
        39
    } else {
        0
    }
}

fn compact_field(value: &str) -> String {
    normalized_search_text(value).replace(' ', "")
}

fn first_token(value: &str) -> &str {
    value.split_whitespace().next().unwrap_or_default()
}

fn subsequence_match(field: &str, term: &str) -> bool {
    if term.chars().count() < 3 {
        return false;
    }

    let target = term.chars().collect::<Vec<_>>();
    let mut idx = 0;
    for ch in field.chars() {
        if idx < target.len() && ch == target[idx] {
            idx += 1;
            if idx == target.len() {
                return true;
            }
        }
    }
    false
}

fn token_typo_score(field: &str, term: &str) -> i32 {
    if term.is_empty() {
        return 0;
    }

    let mut best = 0;
    for token in field.split_whitespace().map(compact_field) {
        if token.is_empty() {
            continue;
        }
        if token == term {
            return 110;
        }
        if token.starts_with(term) {
            best = best.max(97);
            continue;
        }
        if token.contains(term) {
            best = best.max(74);
            continue;
        }
        if levenshtein_distance(&token, term) <= 1 {
            best = best.max(68);
            continue;
        }
        if term.chars().count() >= 4
            && token.chars().count() >= 4
            && subsequence_match(&token, term)
        {
            best = best.max(58);
        }
    }
    best
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right.len()).collect::<Vec<_>>();
    for i in 1..=left.len() {
        let mut cur = vec![0; right.len() + 1];
        cur[0] = i;
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(code: &str) -> Item {
        Item {
            name: code.to_string(),
            item_code: code.to_string(),
            item_name: code.to_string(),
        }
    }

    #[test]
    fn rank_items_matches_xot_lanch_phrase() {
        let got = rank_items(
            vec![
                item("xot lanch sochnaya kuritsa 90gr"),
                item("xot lanch sochnaya kuritsa ostriy 90gr"),
                item("Asl Sifat Hot Dog"),
            ],
            &search_terms("xot lanch"),
        );

        assert!(got.len() >= 2);
        assert_eq!(got[0].item_code, "xot lanch sochnaya kuritsa 90gr");
        assert_eq!(got[1].item_code, "xot lanch sochnaya kuritsa ostriy 90gr");
    }

    #[test]
    fn rank_items_avoids_noisy_short_matches() {
        let got = rank_items(
            vec![
                item("Asl Sifat Hot Dog"),
                item("Asl sfat hot dog sosiski kuriniy"),
                item("elitex svitshot mujskoy zip paket"),
            ],
            &search_terms("hot"),
        );

        assert_eq!(got.len(), 2, "{got:#?}");
        assert!(
            got.iter()
                .all(|item| item.item_code != "elitex svitshot mujskoy zip paket")
        );
    }

    #[test]
    fn search_terms_expand_hot_lanch_variants() {
        let terms = search_terms("hotlunch");

        for expected in ["hotlunch", "hotlanch", "xotlunch", "xotlanch"] {
            assert!(
                terms.iter().any(|term| term == expected),
                "expected {expected} in {terms:#?}"
            );
        }
    }

    #[test]
    fn rank_items_matches_hot_lanch_aliases() {
        for query in [
            "hot lanch",
            "hotlanch",
            "hotlunch",
            "xot lunch",
            "hot launch",
            "xotlanch",
        ] {
            let got = rank_items(
                vec![
                    item("xot lanch sochnaya kuritsa 90gr"),
                    item("xot lanch sochnaya kuritsa ostriy 90gr"),
                    item("Asl Sifat Hot Dog"),
                ],
                &search_terms(query),
            );

            assert!(got.len() >= 2, "query {query} got {got:#?}");
            assert_eq!(got[0].item_code, "xot lanch sochnaya kuritsa 90gr");
        }
    }
}
