import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export interface ObjectData {
  id: string;
  type: string;
  format: string;
  title: string;
  group_name: string;
  primary_file: string | null;
  file_hash: string | null;
  file_size: number;
  created_by: string;
  status: string;
  metadata: string;
  created_at: number;
  updated_at: number;
}

export interface ObjectDetail {
  object: ObjectData;
  tags: string[];
  relations: RelationData[];
  content: string;
}

export interface RelationData {
  target_id: string;
  target_title: string;
  relation_type: string;
}

export interface FullRelationData {
  source_id: string;
  target_id: string;
  relation_type: string;
  source_title: string;
  target_title: string;
  source_type: string;
  target_type: string;
}

export interface ObjectSummary {
  id: string;
  title: string;
  type: string;
  format: string;
  score: number;
  snippet: string;
  created_by: string;
}

export interface GroupNode {
  name: string;
  objects: ObjectSummary[];
}

export interface CollectionNode {
  id: string;
  name: string;
  description: string;
  created_by: string;
  sort_order: number;
  path: string;
  objects: ObjectSummary[];
  children: CollectionNode[];
}

export interface VaultStats {
  total_objects: number;
  by_group: Record<string, number>;
}

export interface AiConfig {
  provider: string;
  base_url: string;
  model: string;
  headers: Record<string, string>;
  max_tool_rounds: number;
  api_key_configured?: boolean;
}

// API 函数
export async function getFileTree(): Promise<GroupNode[]> {
  return invoke("get_file_tree");
}

export async function getCollectionTree(): Promise<CollectionNode[]> {
  return invoke("get_collection_tree");
}

export async function createCollection(
  name: string,
  parentId?: string,
  description = ""
): Promise<string> {
  return invoke("create_collection", { name, parentId: parentId || null, description });
}

export async function moveObjectToCollection(
  objectId: string,
  collectionId: string
): Promise<void> {
  return invoke("move_object_to_collection", { objectId, collectionId });
}

export async function getObject(id: string): Promise<ObjectDetail> {
  return invoke("get_object", { id });
}

export async function createNote(
  title: string,
  objType: string,
  group: string
): Promise<ObjectData> {
  return invoke("create_note", { title, objType, group });
}

export async function createGroup(name: string): Promise<boolean> {
  return invoke("create_group", { name });
}

export async function renameObject(id: string, title: string): Promise<void> {
  return invoke("rename_object", { id, title });
}

export async function duplicateObject(id: string): Promise<ObjectData> {
  return invoke("duplicate_object", { id });
}

export async function renameGroup(oldGroup: string, newGroup: string): Promise<number> {
  return invoke("rename_group", { oldGroup, newGroup });
}

export async function deleteGroup(group: string): Promise<number> {
  return invoke("delete_group", { group });
}

export async function saveObject(
  id: string,
  title: string,
  content: string
): Promise<void> {
  return invoke("save_object", { id, title, content });
}

export async function searchObjects(
  query: string,
  limit?: number
): Promise<ObjectSummary[]> {
  return invoke("search_objects", { query, limit });
}

export async function getStats(): Promise<VaultStats> {
  return invoke("get_stats");
}

export async function updateObjectGroup(
  id: string,
  group: string
): Promise<void> {
  return invoke("update_object_group", { id, group });
}

export async function deleteObject(id: string): Promise<void> {
  return invoke("delete_object", { id });
}

export async function getMcpConfig(): Promise<string> {
  return invoke("get_mcp_config");
}

export async function checkClaudeDesktopConfig(): Promise<boolean> {
  return invoke("check_claude_desktop_config");
}

export async function checkAgentConfig(): Promise<boolean> {
  return invoke("check_agent_config");
}

export async function getVaultPath(): Promise<string> {
  return invoke("get_vault_path");
}

export async function getAiConfig(): Promise<AiConfig> {
  return invoke("get_ai_config");
}

export async function saveAiConfig(
  config: Omit<AiConfig, "api_key_configured">,
  apiKey?: string
): Promise<void> {
  return invoke("save_ai_config", { config, apiKey: apiKey || null });
}

export async function testAiConnection(): Promise<string> {
  return invoke("test_ai_connection");
}

export async function aiEditText(
  instruction: string,
  text: string,
  documentContext?: string
): Promise<string> {
  return invoke("ai_edit_text", {
    instruction,
    text,
    documentContext: documentContext || null,
  });
}

export interface AiOrganizeResult {
  task_id: string;
  summary: string;
  actions: string[];
  moved_objects: number;
  created_directories: number;
  archived_objects: number;
  removed_directories: number;
}

export interface AiTask {
  id: string;
  scope: string;
  instruction: string;
  provider: string;
  model: string;
  status: "running" | "completed" | "failed" | "cancelled" | "interrupted";
  summary: string;
  started_at: number;
  finished_at: number | null;
}

export interface AiTaskAction {
  id: number;
  task_id: string;
  action_type: string;
  object_id: string | null;
  collection_id: string | null;
  description: string;
  created_at: number;
}

export interface AiTaskEvent {
  task_id: string;
  kind: "started" | "action" | "completed" | "failed" | "cancelled";
  description: string;
  result: AiOrganizeResult | null;
}

export async function startAiOrganize(
  instruction: string,
  collectionId?: string
): Promise<string> {
  return invoke("start_ai_organize", {
    instruction,
    scope: collectionId ? "collection" : "knowledge_base",
    collectionId: collectionId || null,
  });
}

export async function cancelAiTask(taskId: string): Promise<boolean> {
  return invoke("cancel_ai_task", { taskId });
}

export async function getAiTasks(limit = 30): Promise<AiTask[]> {
  return invoke("get_ai_tasks", { limit });
}

export async function getAiTaskActions(taskId: string): Promise<AiTaskAction[]> {
  return invoke("get_ai_task_actions", { taskId });
}

export async function resetVault(): Promise<void> {
  return invoke("reset_vault");
}

// ===== 文件导入（阶段 4） =====

/**
 * 调用 Tauri dialog 插件选择多个文件，返回文件路径数组。
 * 用户取消时返回 null。
 */
export async function pickFiles(): Promise<string[] | null> {
  const result = await open({
    multiple: true,
  });
  if (result === null) return null;
  return Array.isArray(result) ? result : [result];
}

export async function pickFolders(): Promise<string[] | null> {
  const result = await open({
    directory: true,
    multiple: true,
  });
  if (result === null) return null;
  return Array.isArray(result) ? result : [result];
}

export async function pickDiscoveryFolders(): Promise<string[] | null> {
  const result = await open({
    directory: true,
    multiple: true,
  });
  if (result === null) return null;
  return Array.isArray(result) ? result : [result];
}

/**
 * 调用后端 import_file 命令将指定路径的文件入库。
 */
export async function importFile(
  filePath: string,
  group?: string
): Promise<ObjectData> {
  return invoke("import_file", { filePath, group });
}

export interface FolderImportResult {
  imported: number;
  skipped: number;
  failed: number;
}

export async function importFolder(
  folderPath: string,
  group?: string
): Promise<FolderImportResult> {
  return invoke("import_folder", { folderPath, group });
}

export interface DiscoveryCandidate {
  path: string;
  name: string;
  size: number;
  modified_at: number;
  kind: "document" | "pdf" | "image" | "code" | "media";
  status: "new" | "updated" | "duplicate";
  existing_object_id: string | null;
}

export async function scanDiscoveryFolders(
  folders: string[]
): Promise<DiscoveryCandidate[]> {
  return invoke("scan_discovery_folders", { folders });
}

export async function importDiscoveryFiles(
  paths: string[],
  group: string
): Promise<number> {
  return invoke("import_discovery_files", { paths, group });
}

export async function ignoreDiscoveryFiles(paths: string[]): Promise<void> {
  return invoke("ignore_discovery_files", { paths });
}

/**
 * 获取对象主资产的文件路径（用于下载链接等）。
 */
export async function getAssetUrl(objectId: string): Promise<string | null> {
  return invoke("get_asset_url", { objectId });
}

/**
 * 获取对象主资产的 base64 data URL（用于图片/PDF 预览）。
 */
export async function getAssetBase64(objectId: string): Promise<string | null> {
  return invoke("get_asset_base64", { objectId });
}

// ===== 关系管理（阶段 5） =====

/**
 * 获取所有关系（用于关系图谱视图）。
 */
export async function getAllRelations(): Promise<FullRelationData[]> {
  return invoke("get_all_relations");
}

/**
 * 在两个对象之间建立关系。
 */
export async function linkObjects(
  fromId: string,
  toId: string,
  relationType: string
): Promise<void> {
  return invoke("link_objects", { fromId, toId, relationType });
}

/**
 * 删除两个对象之间的关系。
 */
export async function unlinkObjects(
  fromId: string,
  toId: string,
  relationType: string
): Promise<void> {
  return invoke("unlink_objects", { fromId, toId, relationType });
}

// ===== 回收站（阶段 5） =====

/**
 * 获取回收站中已删除对象列表。
 */
export async function getTrashObjects(): Promise<ObjectSummary[]> {
  return invoke("get_trash_objects");
}

/**
 * 从回收站恢复对象（将状态从 deleted 改为 draft）。
 */
export async function restoreObject(id: string): Promise<void> {
  return invoke("restore_object", { id });
}

/**
 * 永久删除对象（数据库记录 + 文件）。
 */
export async function permanentDelete(id: string): Promise<void> {
  return invoke("permanent_delete", { id });
}

// ===== 自动关系检测（阶段 6） =====

/**
 * 自动检测对象内容中的链接并建立关系。
 * 支持 [[标题]] 双向链接语法以及内容中提到的对象标题。
 */
export async function autoLink(objectId: string): Promise<number> {
  return invoke("auto_link", { objectId });
}

/**
 * 重建所有对象的自动关系（related_to）。
 * 只清除自动建立的关系，保留手动建立的 depends_on/part_of/implements 关系。
 */
export async function rebuildRelations(): Promise<number> {
  return invoke("rebuild_relations");
}

/** 检查一致性：返回孤儿对象/文件、变化文件 */
export async function checkConsistency(): Promise<{
  orphan_objects: string[];
  orphan_files: string[];
  changed_files: string[];
}> {
  return invoke("check_consistency");
}

/** 手动备份数据库 */
export async function backupDb(): Promise<string | null> {
  return invoke("backup_db");
}

/** 从文件系统重建数据库 */
export async function rebuildVault(): Promise<number> {
  return invoke("rebuild_vault");
}

/** 获取最近操作日志 */
export async function getOperationLog(
  limit?: number
): Promise<
  { id: number; timestamp: number; operation: string; object_id: string | null; status: string; error: string | null }[]
> {
  return invoke("get_operation_log", { limit });
}

/**
 * 递归查询关联关系（深度 N）。
 * 从指定对象出发，沿出链遍历，返回所有深度 ≤ max_depth 的关联对象。
 */
export async function getRelationsRecursive(
  id: string,
  maxDepth?: number
): Promise<RelationData[]> {
  return invoke("get_relations_recursive", { id, maxDepth });
}
