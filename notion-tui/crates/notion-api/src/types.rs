use serde_json::Value;

/// Concatenate the `plain_text` of every element of a rich-text array.
pub fn rich_text_plain(v: &Value) -> String {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t["plain_text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentRef {
    Workspace,
    Page(String),
    DataSource(String),
    Database(String),
    Block(String),
    Unknown,
}

impl ParentRef {
    pub fn parse(v: &Value) -> ParentRef {
        let s = |key: &str| v[key].as_str().unwrap_or_default().to_string();
        match v["type"].as_str() {
            Some("workspace") => ParentRef::Workspace,
            Some("page_id") => ParentRef::Page(s("page_id")),
            Some("data_source_id") => ParentRef::DataSource(s("data_source_id")),
            Some("database_id") => ParentRef::Database(s("database_id")),
            Some("block_id") => ParentRef::Block(s("block_id")),
            _ => ParentRef::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageMeta {
    pub id: String,
    pub parent: ParentRef,
    pub title: String,
    pub icon: Option<String>,
    pub archived: bool,
    pub last_edited_time: String,
}

impl PageMeta {
    pub fn parse(v: &Value) -> PageMeta {
        // Title lives in whichever property has type == "title".
        let title = v["properties"]
            .as_object()
            .and_then(|props| {
                props
                    .values()
                    .find(|p| p["type"] == "title")
                    .map(|p| rich_text_plain(&p["title"]))
            })
            .unwrap_or_default();
        PageMeta {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            parent: ParentRef::parse(&v["parent"]),
            title,
            icon: v["icon"]["emoji"].as_str().map(str::to_string),
            archived: v["archived"].as_bool().unwrap_or(false),
            last_edited_time: v["last_edited_time"].as_str().unwrap_or_default().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataSourceMeta {
    pub id: String,
    pub database_id: String,
    pub title: String,
    pub last_edited_time: String,
}

impl DataSourceMeta {
    pub fn parse(v: &Value) -> DataSourceMeta {
        let database_id = match ParentRef::parse(&v["parent"]) {
            ParentRef::Database(id) => id,
            _ => String::new(),
        };
        DataSourceMeta {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            database_id,
            title: rich_text_plain(&v["title"]),
            last_edited_time: v["last_edited_time"].as_str().unwrap_or_default().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SearchItem {
    Page(PageMeta),
    DataSource(DataSourceMeta),
    Other,
}

#[derive(Debug, Clone)]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: String,
    pub block_type: String,
    pub payload: Value,
    pub plain_text: String,
    pub has_children: bool,
}

impl Block {
    pub fn parse(v: &Value) -> Block {
        let block_type = v["type"].as_str().unwrap_or("unsupported").to_string();
        let payload = v[&block_type].clone();
        let plain_text = match block_type.as_str() {
            "child_page" | "child_database" => {
                payload["title"].as_str().unwrap_or_default().to_string()
            }
            _ => rich_text_plain(&payload["rich_text"]),
        };
        Block {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            block_type,
            payload,
            plain_text,
            has_children: v["has_children"].as_bool().unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlatBlock {
    pub block: Block,
    pub parent_block_id: Option<String>,
    pub ordinal: i64,
}

#[derive(Debug, Clone)]
pub struct DataSource {
    pub meta: DataSourceMeta,
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub properties: Value,
    pub last_edited_time: String,
    pub archived: bool,
}
