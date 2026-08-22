use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub id: String,
    pub r#type: String,
    pub format: String,
    pub title: String,
    pub group_name: String,
    pub primary_file: Option<String>,
    pub file_hash: Option<String>,
    pub file_size: i64,
    pub created_by: String,
    pub status: String,
    pub metadata: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSummary {
    pub id: String,
    pub title: String,
    pub r#type: String,
    pub format: String,
    pub score: f64,
    pub snippet: String,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub target_id: String,
    pub target_title: String,
    pub relation_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultStats {
    pub total_objects: usize,
    pub by_group: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupNode {
    pub name: String,
    pub objects: Vec<ObjectSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionNode {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_by: String,
    pub sort_order: i64,
    pub path: String,
    pub objects: Vec<ObjectSummary>,
    pub children: Vec<CollectionNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTask {
    pub id: String,
    pub scope: String,
    pub instruction: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub summary: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiTaskAction {
    pub id: i64,
    pub task_id: String,
    pub action_type: String,
    pub object_id: Option<String>,
    pub collection_id: Option<String>,
    pub description: String,
    pub created_at: i64,
}
