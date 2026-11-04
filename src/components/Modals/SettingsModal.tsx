import { useEffect, useState } from "react";
import {
  checkClaudeDesktopConfig,
  checkConsistency,
  backupDb,
  rebuildVault,
  getMcpConfig,
  getVaultPath,
  rebuildRelations,
  resetVault,
  getAiConfig,
  saveAiConfig,
  testAiConnection,
} from "../../api";

interface SettingsModalProps {
  onClose: () => void;
  aiEnhanced: boolean;
  onToggleAI: (v: boolean) => void;
  onRedetectAgent: () => Promise<boolean>;
  onRefreshTree?: () => void;
}

interface ParsedMcpConfig {
  mcpServers: Record<
    string,
    { command: string; args: string[]; env: Record<string, string> }
  >;
}

type AiProvider = "openai" | "deepseek" | "ollama" | "openai-compatible";
type AuthMode = "bearer" | "x-api-key" | "custom" | "none";

const providerPresets: Record<Exclude<AiProvider, "openai-compatible">, {
  baseUrl: string;
  model: string;
  authMode: AuthMode;
}> = {
  openai: {
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
    authMode: "bearer",
  },
  deepseek: {
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
    authMode: "bearer",
  },
  ollama: {
    baseUrl: "http://127.0.0.1:11434/v1",
    model: "llama3.2",
    authMode: "none",
  },
};

function validateAiEndpoint(baseUrl: string, model: string) {
  const normalizedBaseUrl = baseUrl.trim().toLowerCase();
  const normalizedModel = model.trim().toLowerCase();
  if (normalizedBaseUrl.includes("api.openai.com") && normalizedModel.includes("deepseek")) {
    throw new Error("Base URL 与模型不匹配：当前地址是 OpenAI，但模型名称是 DeepSeek。请改用模型服务商提供的 Base URL。");
  }
}

export function SettingsModal({
  onClose,
  aiEnhanced,
  onToggleAI,
  onRedetectAgent,
  onRefreshTree,
}: SettingsModalProps) {
  const [detecting, setDetecting] = useState(false);
  const [mcpConfig, setMcpConfig] = useState<string>("");
  const [mcpBinaryPath, setMcpBinaryPath] = useState<string>("");
  const [claudeDetected, setClaudeDetected] = useState<boolean | null>(null);
  const [checkingClaude, setCheckingClaude] = useState(false);
  const [copied, setCopied] = useState(false);
  const [rebuilding, setRebuilding] = useState(false);
  const [rebuildResult, setRebuildResult] = useState<string | null>(null);
  const [vaultPath, setVaultPath] = useState<string>("");
  const [resetting, setResetting] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [checkingConsistency, setCheckingConsistency] = useState(false);
  const [consistencyResult, setConsistencyResult] = useState<string | null>(null);
  const [backingUp, setBackingUp] = useState(false);
  const [backupResult, setBackupResult] = useState<string | null>(null);
  const [rebuildingVault, setRebuildingVault] = useState(false);
  const [rebuildVaultResult, setRebuildVaultResult] = useState<string | null>(null);
  const [aiProvider, setAiProvider] = useState<AiProvider>("openai-compatible");
  const [aiBaseUrl, setAiBaseUrl] = useState("https://api.openai.com/v1");
  const [aiModel, setAiModel] = useState("");
  const [embeddingModel, setEmbeddingModel] = useState("");
  const [authMode, setAuthMode] = useState<AuthMode>("bearer");
  const [apiKeyHeader, setApiKeyHeader] = useState("Authorization");
  const [apiKeyPrefix, setApiKeyPrefix] = useState("Bearer ");
  const [aiApiKey, setAiApiKey] = useState("");
  const [aiKeyConfigured, setAiKeyConfigured] = useState(false);
  const [aiHeaders, setAiHeaders] = useState("{}");
  const [savingAi, setSavingAi] = useState(false);
  const [testingAi, setTestingAi] = useState(false);
  const [aiResult, setAiResult] = useState<string | null>(null);
  const aiEndpointMismatch = aiBaseUrl.trim().toLowerCase().includes("api.openai.com")
    && aiModel.trim().toLowerCase().includes("deepseek");

  useEffect(() => {
    getMcpConfig()
      .then((cfg) => {
        setMcpConfig(cfg);
        try {
          const parsed: ParsedMcpConfig = JSON.parse(cfg);
          const server = parsed.mcpServers?.["knowledge-base"];
          if (server?.command) {
            setMcpBinaryPath(server.command);
          }
        } catch {
          // ignore parse error
        }
      })
      .catch(() => {
        setMcpConfig("");
      });
    getVaultPath()
      .then((path) => setVaultPath(path))
      .catch(() => setVaultPath(""));
    getAiConfig()
      .then((config) => {
        setAiProvider((config.provider as AiProvider) || "openai-compatible");
        setAiBaseUrl(config.base_url);
        setAiModel(config.model);
        setEmbeddingModel(config.embedding_model || "");
        setAuthMode(config.auth_mode || "bearer");
        setApiKeyHeader(config.api_key_header || "Authorization");
        setApiKeyPrefix(config.api_key_prefix ?? "Bearer ");
        setAiHeaders(JSON.stringify(config.headers || {}, null, 2));
        setAiKeyConfigured(!!config.api_key_configured);
      })
      .catch(() => undefined);
  }, []);

  const handleProviderChange = (provider: AiProvider) => {
    setAiProvider(provider);
    setAiResult(null);
    if (provider === "openai-compatible") return;
    const preset = providerPresets[provider];
    setAiBaseUrl(preset.baseUrl);
    setAiModel(preset.model);
    setAuthMode(preset.authMode);
    setApiKeyHeader("Authorization");
    setApiKeyPrefix("Bearer ");
  };

  const currentAiConfig = (headers: Record<string, string>) => ({
    provider: aiProvider,
    base_url: aiBaseUrl.trim(),
    model: aiModel.trim(),
    embedding_model: embeddingModel.trim(),
    auth_mode: authMode,
    api_key_header: authMode === "custom" ? apiKeyHeader.trim() : "Authorization",
    api_key_prefix: authMode === "custom" ? apiKeyPrefix : "Bearer ",
    headers,
    max_tool_rounds: 12,
  });

  const handleSaveAi = async () => {
    setSavingAi(true);
    setAiResult(null);
    try {
      validateAiEndpoint(aiBaseUrl, aiModel);
      const headers = JSON.parse(aiHeaders || "{}");
      if (!headers || Array.isArray(headers) || typeof headers !== "object") {
        throw new Error("自定义 Header 必须是 JSON 对象");
      }
      await saveAiConfig(currentAiConfig(headers), aiApiKey.trim() || undefined);
      if (aiApiKey.trim()) setAiKeyConfigured(true);
      setAiApiKey("");
      setAiResult("✅ 模型配置已保存");
    } catch (e: any) {
      setAiResult(`保存失败: ${e}`);
    } finally {
      setSavingAi(false);
    }
  };

  const handleTestAi = async () => {
    setTestingAi(true);
    setAiResult(null);
    try {
      validateAiEndpoint(aiBaseUrl, aiModel);
      const headers = JSON.parse(aiHeaders || "{}");
      if (!headers || Array.isArray(headers) || typeof headers !== "object") {
        throw new Error("自定义 Header 必须是 JSON 对象");
      }
      if (authMode !== "none" && !aiApiKey.trim() && !aiKeyConfigured) {
        throw new Error("当前认证方式需要 API Key，请先填写");
      }
      await saveAiConfig(currentAiConfig(headers), aiApiKey.trim() || undefined);
      if (aiApiKey.trim()) setAiKeyConfigured(true);
      setAiApiKey("");
      const result = await testAiConnection();
      setAiResult(`✅ 连接成功：${result.trim() || "OK"}`);
    } catch (e: any) {
      const message = String(e);
      if (/401|unauthorized|didn't provide an api key|尚未配置 api key/i.test(message)) {
        setAiResult("❌ 连接失败：服务端未收到有效 API Key。请重新输入 API Key，点击“保存并测试”；如果使用自定义 Header，请不要覆盖 Authorization。");
      } else {
        setAiResult(`连接失败: ${message}`);
      }
    } finally {
      setTestingAi(false);
    }
  };

  const handleRedetect = async () => {
    setDetecting(true);
    try {
      await onRedetectAgent();
    } finally {
      setDetecting(false);
    }
  };

  const handleCheckClaude = () => {
    setCheckingClaude(true);
    checkClaudeDesktopConfig()
      .then((exists) => setClaudeDetected(exists))
      .catch(() => setClaudeDetected(false))
      .finally(() => setCheckingClaude(false));
  };

  const handleCopy = async () => {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await navigator.clipboard.writeText(mcpConfig);
      } else {
        // 降级：选中文本
        const ta = document.createElement("textarea");
        ta.value = mcpConfig;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        document.body.removeChild(ta);
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore
    }
  };

  const handleRebuildRelations = async () => {
    setRebuilding(true);
    setRebuildResult(null);
    try {
      const count = await rebuildRelations();
      setRebuildResult(`已重建 ${count} 个关系`);
    } catch (e: any) {
      setRebuildResult(`重建失败: ${e}`);
    } finally {
      setRebuilding(false);
      // 5 秒后清除结果
      setTimeout(() => setRebuildResult(null), 5000);
    }
  };

  const handleResetVault = async () => {
    setResetting(true);
    try {
      await resetVault();
      setShowResetConfirm(false);
      alert("知识库已重置，请刷新页面");
    } catch (e: any) {
      alert(`重置失败: ${e}`);
    } finally {
      setResetting(false);
    }
  };

  const handleCheckConsistency = async () => {
    setCheckingConsistency(true);
    setConsistencyResult(null);
    try {
      const result = await checkConsistency();
      const parts: string[] = [];
      if (
        result.orphan_objects.length === 0 &&
        result.orphan_files.length === 0 &&
        result.changed_files.length === 0
      ) {
        setConsistencyResult("✅ 数据库一致，无异常");
      } else {
        if (result.orphan_objects.length > 0) {
          parts.push(`⚠️ ${result.orphan_objects.length} 个孤儿对象（数据库有记录但文件丢失）`);
        }
        if (result.orphan_files.length > 0) {
          parts.push(`⚠️ ${result.orphan_files.length} 个孤儿文件（文件存在但无数据库记录）`);
        }
        if (result.changed_files.length > 0) {
          parts.push(`⚠️ ${result.changed_files.length} 个文件 hash 已变化`);
        }
        setConsistencyResult(parts.join("\n"));
      }
    } catch (e: any) {
      setConsistencyResult(`检查失败: ${e}`);
    } finally {
      setCheckingConsistency(false);
      setTimeout(() => setConsistencyResult(null), 5000);
    }
  };

  const handleBackupDb = async () => {
    setBackingUp(true);
    setBackupResult(null);
    try {
      const path = await backupDb();
      if (path) {
        const lastPart = path.split(/[/\\]/).pop() || path;
        setBackupResult(`✅ 备份成功：${lastPart}`);
      } else {
        setBackupResult("✅ 备份成功");
      }
    } catch (e: any) {
      setBackupResult(`备份失败: ${e}`);
    } finally {
      setBackingUp(false);
      setTimeout(() => setBackupResult(null), 5000);
    }
  };

  const handleRebuildVault = async () => {
    setRebuildingVault(true);
    setRebuildVaultResult(null);
    try {
      const count = await rebuildVault();
      setRebuildVaultResult(`✅ 重建完成，已恢复 ${count} 个对象`);
      onRefreshTree?.();
    } catch (e: any) {
      setRebuildVaultResult(`重建失败: ${e}`);
    } finally {
      setRebuildingVault(false);
      setTimeout(() => setRebuildVaultResult(null), 5000);
    }
  };

  return (
    <div
      className="modal-overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="modal settings-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-modal-title"
      >
        <div className="modal-header settings-modal-header">
          <span id="settings-modal-title">设置</span>
          <button
            className="icon-btn settings-modal-close"
            type="button"
            onClick={onClose}
            aria-label="关闭设置"
            title="关闭"
          >
            ×
          </button>
        </div>
        <div className="modal-body">
          <div className="settings-row">
            <div>
              <div className="settings-label">知识库路径</div>
              <div className="settings-desc">本地知识库存储目录</div>
            </div>
            <div className="settings-value" style={{
                  maxWidth: 280,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
                title={vaultPath}
              >
                {vaultPath || "加载中..."}
              </div>
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-label">AI 增强</div>
              <div className="settings-desc">保存 AI 增强偏好（模型能力接入后生效）</div>
            </div>
            <div
              className={`toggle${aiEnhanced ? " on" : ""}`}
              onClick={() => onToggleAI(!aiEnhanced)}
              role="switch"
              aria-checked={aiEnhanced}
            />
          </div>
          <div className="settings-ai-card">
            <div className="settings-label">内置 AI 模型</div>
            <div className="settings-desc" style={{ marginBottom: 10 }}>
              支持 OpenAI、DeepSeek、Ollama 及任意 OpenAI-compatible 接口。选择预设或自由填写服务地址与模型。
            </div>
            <label className="settings-ai-field">
              <span>模型服务</span>
              <select
                value={aiProvider}
                onChange={(event) => handleProviderChange(event.target.value as AiProvider)}
              >
                <option value="openai">OpenAI</option>
                <option value="deepseek">DeepSeek</option>
                <option value="ollama">Ollama / 本地模型</option>
                <option value="openai-compatible">自定义兼容服务</option>
              </select>
            </label>
            <label className="settings-ai-field">
              <span>Base URL</span>
              <input
                value={aiBaseUrl}
                onChange={(e) => setAiBaseUrl(e.target.value)}
                placeholder="https://api.openai.com/v1"
              />
              <small className="settings-ai-help">
                填写模型服务商提供的兼容接口地址，例如 OpenAI 或 DeepSeek 的 /v1 地址
              </small>
            </label>
            <label className="settings-ai-field">
              <span>模型</span>
              <input
                value={aiModel}
                onChange={(e) => setAiModel(e.target.value)}
                placeholder="模型名称"
              />
              {aiEndpointMismatch && (
                <small className="settings-ai-endpoint-warning">
                  当前是 OpenAI 地址，但填写了 DeepSeek 模型，请改成该模型服务商提供的 Base URL
                </small>
              )}
            </label>
            {authMode !== "none" ? (
              <label className="settings-ai-field">
                <span>API Key</span>
                <input
                  type="password"
                  value={aiApiKey}
                  onChange={(e) => setAiApiKey(e.target.value)}
                  placeholder={aiKeyConfigured ? "已安全保存；留空保持不变" : "输入 API Key"}
                />
                <small className={`settings-ai-key-hint${aiKeyConfigured ? " configured" : ""}`}>
                  {aiKeyConfigured ? "已检测到安全存储的 API Key" : "尚未配置 API Key"}
                </small>
              </label>
            ) : (
              <div className="settings-ai-no-key">当前服务无需 API Key，适用于本机 Ollama 等无认证接口</div>
            )}
            <details className="settings-ai-advanced">
              <summary>高级设置（可选）</summary>
              <label className="settings-ai-field">
                <span>认证方式</span>
                <select
                  value={authMode}
                  onChange={(event) => setAuthMode(event.target.value as AuthMode)}
                >
                  <option value="bearer">Bearer API Key</option>
                  <option value="x-api-key">x-api-key</option>
                  <option value="custom">自定义认证 Header</option>
                  <option value="none">无需认证</option>
                </select>
              </label>
              {authMode === "custom" && (
                <>
                  <label className="settings-ai-field">
                    <span>认证 Header</span>
                    <input
                      value={apiKeyHeader}
                      onChange={(event) => setApiKeyHeader(event.target.value)}
                      placeholder="Authorization"
                    />
                  </label>
                  <label className="settings-ai-field">
                    <span>Key 前缀</span>
                    <input
                      value={apiKeyPrefix}
                      onChange={(event) => setApiKeyPrefix(event.target.value)}
                      placeholder="Bearer "
                    />
                  </label>
                </>
              )}
              <label className="settings-ai-field">
                <span>Embedding 模型</span>
                <input
                  value={embeddingModel}
                  onChange={(e) => setEmbeddingModel(e.target.value)}
                  placeholder="留空即可使用全文检索"
                />
              </label>
              <label className="settings-ai-field settings-ai-field-stack">
                <span>自定义 Header（JSON）</span>
                <textarea
                  value={aiHeaders}
                  onChange={(e) => setAiHeaders(e.target.value)}
                  spellCheck={false}
                  rows={3}
                />
              </label>
            </details>
            {aiResult && (
              <div className={aiResult.includes("失败") ? "settings-ai-result error" : "settings-ai-result"}>
                {aiResult}
              </div>
            )}
            <div className="settings-ai-actions">
              <button className="btn btn-ghost" onClick={handleSaveAi} disabled={savingAi || testingAi}>
                {savingAi ? "保存中..." : "保存配置"}
              </button>
              <button className="btn btn-primary" onClick={handleTestAi} disabled={savingAi || testingAi}>
                {testingAi ? "测试中..." : "保存并测试"}
              </button>
            </div>
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-label">Agent 接入</div>
              <div className="settings-desc">检测 MCP Server 连接状态</div>
            </div>
            <button
              className="btn btn-ghost"
              style={{ height: 26, fontSize: 11 }}
              onClick={handleRedetect}
              disabled={detecting}
            >
              {detecting ? "检测中..." : "重新检测"}
            </button>
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-label">重建关系</div>
              <div className="settings-desc">
                扫描所有对象内容，自动建立 [[双向链接]] 和标题匹配关系
              </div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              {rebuildResult && (
                <span
                  style={{
                    fontSize: 11,
                    color: rebuildResult.includes("失败")
                      ? "var(--danger, #da3633)"
                      : "var(--success, #2ea043)",
                  }}
                >
                  {rebuildResult}
                </span>
              )}
              <button
                className="btn btn-ghost"
                style={{ height: 26, fontSize: 11 }}
                onClick={handleRebuildRelations}
                disabled={rebuilding}
              >
                {rebuilding ? "重建中..." : "重建所有关系"}
              </button>
            </div>
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-label">MCP 通信方式</div>
              <div className="settings-desc">由 Agent 按需启动本地进程</div>
            </div>
            <div className="settings-value">stdio</div>
          </div>

          {/* 数据库维护 */}
          <div
            style={{
              marginTop: 16,
              paddingTop: 12,
              borderTop: "1px solid var(--border-color, #eee)",
            }}
          >
            <div className="settings-label" style={{ marginBottom: 8 }}>
              数据库维护
            </div>
            <div className="settings-row" style={{ marginBottom: 8 }}>
              <div>
                <div className="settings-label">重建自动关系</div>
                <div className="settings-desc">
                  扫描所有对象内容，重新建立自动关系
                </div>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                {rebuildResult && (
                  <span
                    style={{
                      fontSize: 11,
                      color: rebuildResult.includes("失败")
                        ? "var(--danger, #da3633)"
                        : "var(--success, #2ea043)",
                    }}
                  >
                    {rebuildResult}
                  </span>
                )}
                <button
                  className="btn btn-ghost"
                  style={{ height: 26, fontSize: 11 }}
                  onClick={handleRebuildRelations}
                  disabled={rebuilding}
                >
                  {rebuilding ? "重建中..." : "重建关系"}
                </button>
                <button
                  className="btn btn-ghost"
                  style={{ height: 26, fontSize: 11 }}
                  onClick={handleCheckConsistency}
                  disabled={checkingConsistency}
                >
                  {checkingConsistency ? "检查中..." : "一致性检查"}
                </button>
              </div>
            </div>
            {consistencyResult && (
              <div
                style={{
                  fontSize: 11,
                  marginBottom: 8,
                  padding: "4px 8px",
                  background: consistencyResult.includes("✅")
                    ? "var(--success-bg, #dafbe1)"
                    : "var(--warning-bg, #fff8c5)",
                  borderRadius: 4,
                  whiteSpace: "pre-wrap",
                  color: consistencyResult.includes("失败")
                    ? "var(--danger, #da3633)"
                    : "inherit",
                }}
              >
                {consistencyResult}
              </div>
            )}
            <div className="settings-row" style={{ marginBottom: 8 }}>
              <div>
                <div className="settings-label">手动备份</div>
                <div className="settings-desc">
                  备份当前数据库文件
                </div>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                {backupResult && (
                  <span
                    style={{
                      fontSize: 11,
                      color: backupResult.includes("失败")
                        ? "var(--danger, #da3633)"
                        : "var(--success, #2ea043)",
                    }}
                  >
                    {backupResult}
                  </span>
                )}
                <button
                  className="btn btn-ghost"
                  style={{ height: 26, fontSize: 11 }}
                  onClick={handleBackupDb}
                  disabled={backingUp}
                >
                  {backingUp ? "备份中..." : "手动备份"}
                </button>
              </div>
            </div>
            <div className="settings-row" style={{ marginBottom: 8 }}>
              <div>
                <div className="settings-label">从文件重建</div>
                <div className="settings-desc">
                  扫描文件系统，重新构建数据库（保留已有数据）
                </div>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                {rebuildVaultResult && (
                  <span
                    style={{
                      fontSize: 11,
                      color: rebuildVaultResult.includes("失败")
                        ? "var(--danger, #da3633)"
                        : "var(--success, #2ea043)",
                    }}
                  >
                    {rebuildVaultResult}
                  </span>
                )}
                <button
                  className="btn btn-ghost"
                  style={{ height: 26, fontSize: 11 }}
                  onClick={handleRebuildVault}
                  disabled={rebuildingVault}
                >
                  {rebuildingVault ? "重建中..." : "从文件重建"}
                </button>
              </div>
            </div>
            <div className="settings-row" style={{ marginBottom: 0 }}>
              <div>
                <div className="settings-label" style={{ color: "var(--danger, #da3633)" }}>
                  重置知识库
                </div>
                <div className="settings-desc" style={{ color: "var(--danger, #da3633)" }}>
                  删除所有数据，恢复初始化状态
                </div>
              </div>
              <button
                className="btn btn-ghost"
                style={{
                  height: 26,
                  fontSize: 11,
                  color: "var(--danger, #da3633)",
                  borderColor: "var(--danger, #da3633)",
                }}
                onClick={() => setShowResetConfirm(true)}
                disabled={resetting}
              >
                {resetting ? "重置中..." : "重置知识库"}
              </button>
            </div>
          </div>

          <div
            style={{
              marginTop: 16,
              paddingTop: 12,
              borderTop: "1px solid var(--border-color, #eee)",
            }}
          >
            <div className="settings-label" style={{ marginBottom: 4 }}>
              MCP 配置
            </div>
            <div className="settings-desc" style={{ marginBottom: 8 }}>
              Claude Desktop 等 AI 客户端接入所需的 JSON-RPC 配置
            </div>

            <div className="settings-row" style={{ marginBottom: 8 }}>
              <div>
                <div className="settings-label">kb-mcp 二进制</div>
                <div className="settings-desc">MCP Server 可执行文件路径</div>
              </div>
              <div
                className="settings-value"
                style={{
                  maxWidth: 280,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
                title={mcpBinaryPath}
              >
                {mcpBinaryPath || "(未配置)"}
              </div>
            </div>

            <div className="settings-row" style={{ marginBottom: 8 }}>
              <div>
                <div className="settings-label">Claude Desktop</div>
                <div className="settings-desc">检测配置文件是否存在</div>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                {claudeDetected !== null && (
                  <span
                    style={{
                      fontSize: 11,
                      color: claudeDetected
                        ? "var(--success, #2ea043)"
                        : "var(--danger, #da3633)",
                    }}
                  >
                    {claudeDetected ? "已检测到" : "未检测到"}
                  </span>
                )}
                <button
                  className="btn btn-ghost"
                  style={{ height: 26, fontSize: 11 }}
                  onClick={handleCheckClaude}
                  disabled={checkingClaude}
                >
                  {checkingClaude ? "检测中..." : "检测 Claude Desktop"}
                </button>
              </div>
            </div>

            <div style={{ marginBottom: 8 }}>
              <div className="settings-label" style={{ marginBottom: 4 }}>
                配置 JSON
              </div>
              <pre
                style={{
                  margin: 0,
                  padding: 8,
                  maxHeight: 180,
                  overflow: "auto",
                  fontSize: 11,
                  lineHeight: 1.5,
                  background: "var(--bg-secondary, #f6f8fa)",
                  border: "1px solid var(--border-color, #eee)",
                  borderRadius: 4,
                  whiteSpace: "pre-wrap",
                  wordBreak: "break-all",
                }}
              >
                {mcpConfig || "加载中..."}
              </pre>
            </div>

            <div style={{ display: "flex", justifyContent: "flex-end" }}>
              <button
                className="btn btn-primary"
                style={{ height: 28, fontSize: 12 }}
                onClick={handleCopy}
                disabled={!mcpConfig}
              >
                {copied ? "已复制 ✓" : "复制配置"}
              </button>
            </div>
          </div>
        </div>
        {showResetConfirm && (
          <div
            style={{
              position: "fixed",
              inset: 0,
              background: "rgba(0,0,0,0.5)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              zIndex: 1000,
            }}
            onClick={() => setShowResetConfirm(false)}
          >
            <div
              style={{
                background: "var(--bg-primary, #fff)",
                borderRadius: 8,
                padding: 24,
                maxWidth: 400,
                textAlign: "center",
              }}
              onClick={(e) => e.stopPropagation()}
            >
              <div style={{ fontSize: 16, fontWeight: 600, marginBottom: 12 }}>
                确认重置知识库？
              </div>
              <div
                style={{
                  fontSize: 13,
                  color: "var(--danger, #da3633)",
                  marginBottom: 20,
                }}
              >
                此操作将删除所有数据和文件，且不可恢复！
              </div>
              <div style={{ display: "flex", gap: 12, justifyContent: "center" }}>
                <button
                  className="btn btn-ghost"
                  style={{ height: 32, fontSize: 12 }}
                  onClick={() => setShowResetConfirm(false)}
                >
                  取消
                </button>
                <button
                  className="btn btn-primary"
                  style={{
                    height: 32,
                    fontSize: 12,
                    background: "var(--danger, #da3633)",
                    borderColor: "var(--danger, #da3633)",
                  }}
                  onClick={handleResetVault}
                  disabled={resetting}
                >
                  {resetting ? "重置中..." : "确认重置"}
                </button>
              </div>
            </div>
          </div>
        )}
        <div className="modal-footer">
          <button className="btn btn-primary" onClick={onClose}>
            关闭
          </button>
        </div>
      </div>
    </div>
  );
}
