// ===== 类型定义 =====
export type ObjType =
  | "concept"
  | "project"
  | "code"
  | "document"
  | "inbox"
  | "image";

export type ObjFormat =
  | "markdown"
  | "code"
  | "image"
  | "pdf"
  | "attachment";

export type CreatedBy = "human" | "agent";

export interface TreeNode {
  id: string;
  name: string;
  type: "group" | "object";
  collectionId?: string;
  collectionPath?: string;
  children?: TreeNode[];
  metadata?: {
    objType: ObjType;
    objFormat: ObjFormat;
    createdBy: CreatedBy;
    count?: number;
  };
}

export interface ObjectMetadata {
  id: string;
  type: ObjType;
  format: ObjFormat;
  title: string;
  tags: string[];
  group: string;
  createdBy: CreatedBy;
  status: "curated" | "draft" | "processing";
  createdAt: string;
  updatedAt: string;
  fileSize: number;
  fileHash: string;
  source?: string;
}

export interface Relation {
  targetId: string;
  targetTitle: string;
  relationType: "related_to" | "depends_on" | "extends" | "part_of";
}

// ===== 文件树 Mock =====
export const mockFileTree: TreeNode[] = [
  {
    id: "g1",
    name: "技术概念",
    type: "group",
    children: [
      {
        id: "obj1",
        name: "RAG 检索增强生成",
        type: "object",
        metadata: {
          objType: "concept",
          objFormat: "markdown",
          createdBy: "human",
        },
      },
      {
        id: "obj2",
        name: "MCP 协议详解",
        type: "object",
        metadata: {
          objType: "concept",
          objFormat: "markdown",
          createdBy: "agent",
        },
      },
      {
        id: "obj3",
        name: "rag_pipeline.py",
        type: "object",
        metadata: {
          objType: "code",
          objFormat: "code",
          createdBy: "human",
        },
      },
    ],
  },
  {
    id: "g2",
    name: "产品设计",
    type: "group",
    children: [
      {
        id: "obj4",
        name: "知识库产品文档",
        type: "object",
        metadata: {
          objType: "document",
          objFormat: "markdown",
          createdBy: "human",
        },
      },
      {
        id: "obj5",
        name: "架构图.png",
        type: "object",
        metadata: {
          objType: "image",
          objFormat: "image",
          createdBy: "human",
        },
      },
    ],
  },
  {
    id: "g3",
    name: "收件箱",
    type: "group",
    children: [
      {
        id: "obj6",
        name: "周报.docx",
        type: "object",
        metadata: {
          objType: "document",
          objFormat: "attachment",
          createdBy: "agent",
        },
      },
    ],
  },
];

// ===== 元数据 Mock =====
export const mockMetadata: Record<string, ObjectMetadata> = {
  obj1: {
    id: "obj1",
    type: "concept",
    format: "markdown",
    title: "RAG 检索增强生成",
    tags: ["AI", "检索", "大模型"],
    group: "技术概念",
    createdBy: "human",
    status: "curated",
    createdAt: "2026-07-30T10:30:00",
    updatedAt: "2026-07-30T15:20:00",
    fileSize: 2048,
    fileHash: "sha256:abc123def456...",
    source: "手动创建",
  },
  obj2: {
    id: "obj2",
    type: "concept",
    format: "markdown",
    title: "MCP 协议详解",
    tags: ["MCP", "协议", "Agent"],
    group: "技术概念",
    createdBy: "agent",
    status: "curated",
    createdAt: "2026-07-29T09:00:00",
    updatedAt: "2026-07-30T11:00:00",
    fileSize: 3072,
    fileHash: "sha256:xyz789...",
    source: "Agent 自动生成",
  },
  obj3: {
    id: "obj3",
    type: "code",
    format: "code",
    title: "rag_pipeline.py",
    tags: ["Python", "RAG"],
    group: "技术概念",
    createdBy: "human",
    status: "draft",
    createdAt: "2026-07-28T14:00:00",
    updatedAt: "2026-07-28T14:30:00",
    fileSize: 1536,
    fileHash: "sha256:code001...",
    source: "手动创建",
  },
  obj4: {
    id: "obj4",
    type: "document",
    format: "markdown",
    title: "知识库产品文档",
    tags: ["产品", "文档"],
    group: "产品设计",
    createdBy: "human",
    status: "curated",
    createdAt: "2026-07-25T10:00:00",
    updatedAt: "2026-07-30T16:00:00",
    fileSize: 4096,
    fileHash: "sha256:doc004...",
    source: "导入",
  },
  obj5: {
    id: "obj5",
    type: "image",
    format: "image",
    title: "架构图.png",
    tags: ["架构", "图片"],
    group: "产品设计",
    createdBy: "human",
    status: "curated",
    createdAt: "2026-07-26T11:00:00",
    updatedAt: "2026-07-26T11:00:00",
    fileSize: 102400,
    fileHash: "sha256:img005...",
    source: "导入",
  },
  obj6: {
    id: "obj6",
    type: "document",
    format: "attachment",
    title: "周报.docx",
    tags: ["周报"],
    group: "收件箱",
    createdBy: "agent",
    status: "processing",
    createdAt: "2026-07-30T18:00:00",
    updatedAt: "2026-07-30T18:00:00",
    fileSize: 25600,
    fileHash: "sha256:doc006...",
    source: "邮件采集",
  },
};

// ===== 关系 Mock =====
export const mockRelations: Record<string, Relation[]> = {
  obj1: [
    {
      targetId: "obj2",
      targetTitle: "MCP 协议详解",
      relationType: "related_to",
    },
    {
      targetId: "obj3",
      targetTitle: "rag_pipeline.py",
      relationType: "depends_on",
    },
  ],
  obj2: [
    {
      targetId: "obj1",
      targetTitle: "RAG 检索增强生成",
      relationType: "related_to",
    },
  ],
  obj3: [
    {
      targetId: "obj1",
      targetTitle: "RAG 检索增强生成",
      relationType: "part_of",
    },
  ],
  obj4: [
    {
      targetId: "obj5",
      targetTitle: "架构图.png",
      relationType: "related_to",
    },
  ],
  obj5: [],
  obj6: [],
};

// ===== 内容 Mock =====
export const mockContent: Record<string, string> = {
  obj1: `# RAG 检索增强生成

## 概述
RAG（Retrieval-Augmented Generation）是一种结合检索和生成的技术，用于解决大语言模型知识更新滞后和幻觉问题。

## 核心流程
1. 文档分块（Chunking）
2. 向量化（Embedding）
3. 检索相关片段（Retrieval）
4. 生成回答（Generation）

## 代码示例
\`\`\`python
def rag_pipeline(query, knowledge_base):
    chunks = retrieve(query, knowledge_base)
    answer = generate(query, chunks)
    return answer
\`\`\`

## 优势
- 知识可更新，无需重新训练
- 可引用来源，减少幻觉
- 适用于领域知识问答
`,
  obj2: `# MCP 协议详解

## 概述
MCP（Model Context Protocol）是一种标准化的协议，用于在 AI Agent 和外部工具/资源之间建立通信。

## 核心概念
- **Server**: 暴露能力的服务端
- **Client**: 调用能力的客户端
- **Resource**: 可读写的资源
- **Tool**: 可调用的工具函数

## 工作流程
1. Server 注册能力
2. Client 发现能力
3. Client 调用 Tool / 读写 Resource
4. Server 返回结果
`,
  obj3: `# rag_pipeline.py

\`\`\`python
"""RAG 检索增强生成管线"""
from typing import List
import numpy as np


def chunk_document(text: str, chunk_size: int = 200) -> List[str]:
    """将文档切分为固定大小的块"""
    return [text[i:i+chunk_size] for i in range(0, len(text), chunk_size)]


def embed(text: str) -> np.ndarray:
    """将文本向量化"""
    # 使用 embedding 模型
    return np.random.rand(768)


def retrieve(query: str, kb: List[str], top_k: int = 3) -> List[str]:
    """检索相关片段"""
    q_vec = embed(query)
    chunks = []
    for doc in kb:
        chunks.extend(chunk_document(doc))
    scores = [(c, np.dot(q_vec, embed(c))) for c in chunks]
    scores.sort(key=lambda x: -x[1])
    return [c for c, _ in scores[:top_k]]


def generate(query: str, chunks: List[str]) -> str:
    """生成回答"""
    return f"基于 {len(chunks)} 个片段回答: {query}"


def rag_pipeline(query, knowledge_base):
    chunks = retrieve(query, knowledge_base)
    return generate(query, chunks)
\`\`\`
`,
  obj4: `# 知识库产品文档

## 产品定位
面向个人和团队的知识管理工具，结合 Obsidian 风格的笔记编辑和 AI 增强的知识图谱。

## 核心功能
- 📝 Markdown 笔记编辑
- 🗂️ 对象化知识管理（概念/项目/代码/文档）
- 🔗 关系图谱可视化
- 🤖 Agent 自动入库
- 🔍 全文检索 + 语义搜索

## 技术栈
- 前端: Tauri + React + TypeScript
- 后端: Rust
- 存储: SQLite + 向量数据库
`,
  obj5: `# 架构图.png

这是一张架构图（mock 中以文本代替）。

\`\`\`
[用户] → [Tauri UI]
            ↓
       [Rust Core]
       /    |    \\
[SQLite] [VecDB] [Agent]
            ↓
       [MCP Server]
\`\`\`
`,
  obj6: `# 周报.docx

> 该文件正在处理中（status: processing），由 Agent 通过邮件采集入库。

待 Agent 完成解析后，将自动生成结构化内容。
`,
};

// ===== 工具函数 =====
export function findNodeById(
  tree: TreeNode[],
  id: string
): TreeNode | undefined {
  for (const node of tree) {
    if (node.id === id) return node;
    if (node.children) {
      const found = findNodeById(node.children, id);
      if (found) return found;
    }
  }
  return undefined;
}

export function findGroupOfNode(
  tree: TreeNode[],
  id: string
): TreeNode | undefined {
  for (const node of tree) {
    if (node.children) {
      if (node.children.some((c) => c.id === id)) return node;
      const found = findGroupOfNode(node.children, id);
      if (found) return found;
    }
  }
  return undefined;
}

export function countObjects(tree: TreeNode[]): number {
  let count = 0;
  for (const node of tree) {
    if (node.type === "object") count++;
    if (node.children) count += countObjects(node.children);
  }
  return count;
}
