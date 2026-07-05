use crate::client::NotionClient;
use crate::error::ApiError;
use crate::types::{
    Block, DataSource, DataSourceMeta, FlatBlock, PageMeta, Row, SearchItem, SearchPage,
};
use serde_json::{json, Value};
use std::collections::VecDeque;

impl NotionClient {
    /// One page of workspace search, sorted by last_edited_time descending.
    pub async fn search_page(&self, cursor: Option<&str>) -> Result<SearchPage, ApiError> {
        let mut body = json!({
            "sort": {"timestamp": "last_edited_time", "direction": "descending"},
            "page_size": 100
        });
        if let Some(c) = cursor {
            body["start_cursor"] = json!(c);
        }
        let v = self.post_json("/v1/search", &body).await?;
        Ok(SearchPage {
            items: v["results"]
                .as_array()
                .map(|arr| arr.iter().map(parse_search_item).collect())
                .unwrap_or_default(),
            next_cursor: v["next_cursor"].as_str().map(str::to_string),
        })
    }

    pub async fn fetch_block_tree(&self, root_page_id: &str) -> Result<Vec<FlatBlock>, ApiError> {
        let mut out = Vec::new();
        // (container id to list children of, parent_block_id recorded on those children)
        let mut queue: VecDeque<(String, Option<String>)> = VecDeque::new();
        queue.push_back((root_page_id.to_string(), None));

        while let Some((container, parent_block_id)) = queue.pop_front() {
            let mut cursor: Option<String> = None;
            let mut ordinal: i64 = 0;
            loop {
                let mut path = format!("/v1/blocks/{container}/children?page_size=100");
                if let Some(c) = &cursor {
                    path.push_str(&format!("&start_cursor={c}"));
                }
                let v = self.get_json(&path).await?;
                for item in v["results"].as_array().into_iter().flatten() {
                    let block = Block::parse(item);
                    let recurse = block.has_children
                        && block.block_type != "child_page"
                        && block.block_type != "child_database";
                    if recurse {
                        queue.push_back((block.id.clone(), Some(block.id.clone())));
                    }
                    out.push(FlatBlock {
                        block,
                        parent_block_id: parent_block_id.clone(),
                        ordinal,
                    });
                    ordinal += 1;
                }
                cursor = v["next_cursor"].as_str().map(str::to_string);
                if cursor.is_none() {
                    break;
                }
            }
        }
        Ok(out)
    }

    pub async fn get_data_source(&self, id: &str) -> Result<DataSource, ApiError> {
        let v = self.get_json(&format!("/v1/data_sources/{id}")).await?;
        Ok(DataSource {
            meta: DataSourceMeta::parse(&v),
            schema: v["properties"].clone(),
        })
    }

    pub async fn query_data_source_all(&self, id: &str) -> Result<Vec<Row>, ApiError> {
        let mut rows = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut body = json!({"page_size": 100});
            if let Some(c) = &cursor {
                body["start_cursor"] = json!(c);
            }
            let v = self
                .post_json(&format!("/v1/data_sources/{id}/query"), &body)
                .await?;
            for item in v["results"].as_array().into_iter().flatten() {
                rows.push(Row {
                    id: item["id"].as_str().unwrap_or_default().to_string(),
                    properties: item["properties"].clone(),
                    last_edited_time: item["last_edited_time"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    archived: item["archived"].as_bool().unwrap_or(false),
                });
            }
            cursor = v["next_cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                return Ok(rows);
            }
        }
    }

    pub async fn update_block(
        &self,
        block_id: &str,
        block_type: &str,
        payload: &Value,
    ) -> Result<Value, ApiError> {
        let body = json!({ block_type: payload });
        self.patch_json(&format!("/v1/blocks/{block_id}"), &body).await
    }

    pub async fn delete_block(&self, block_id: &str) -> Result<Value, ApiError> {
        self.delete_json(&format!("/v1/blocks/{block_id}")).await
    }

    pub async fn append_children(
        &self,
        container_id: &str,
        after: Option<&str>,
        block: Value,
    ) -> Result<Value, ApiError> {
        let mut body = json!({"children": [block]});
        if let Some(a) = after {
            body["after"] = json!(a);
        }
        self.patch_json(&format!("/v1/blocks/{container_id}/children"), &body).await
    }

    pub async fn create_page(&self, parent: Value, properties: Value) -> Result<Value, ApiError> {
        let body = json!({"parent": parent, "properties": properties});
        self.post_json("/v1/pages", &body).await
    }

    pub async fn update_page(&self, page_id: &str, body: Value) -> Result<Value, ApiError> {
        self.patch_json(&format!("/v1/pages/{page_id}"), &body).await
    }

    pub async fn get_block_edited_time(&self, block_id: &str) -> Result<String, ApiError> {
        let v = self.get_json(&format!("/v1/blocks/{block_id}")).await?;
        Ok(v["last_edited_time"].as_str().unwrap_or_default().to_string())
    }

    pub async fn get_page_edited_time(&self, page_id: &str) -> Result<String, ApiError> {
        let v = self.get_json(&format!("/v1/pages/{page_id}")).await?;
        Ok(v["last_edited_time"].as_str().unwrap_or_default().to_string())
    }
}

fn parse_search_item(v: &Value) -> SearchItem {
    match v["object"].as_str() {
        Some("page") => SearchItem::Page(PageMeta::parse(v)),
        Some("data_source") => SearchItem::DataSource(DataSourceMeta::parse(v)),
        _ => SearchItem::Other,
    }
}
