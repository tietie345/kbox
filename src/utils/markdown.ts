// 简易 markdown -> HTML 转换，仅覆盖常用语法（不依赖外部库）
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function safeHref(raw: string): string {
  const value = raw.trim();
  if (/^(https?:|mailto:)/i.test(value) || value.startsWith("#")) {
    return value.replace(/&/g, "&amp;").replace(/"/g, "&quot;");
  }
  return "#";
}

export function markdownToHtml(md: string): string {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  let inCode = false;
  let codeLang = "";
  let codeBuf: string[] = [];
  let listType: "ul" | "ol" | null = null;

  const closeList = () => {
    if (listType) {
      out.push(`</${listType}>`);
      listType = null;
    }
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // 代码块
    const fence = line.match(/^```(.*)$/);
    if (fence) {
      if (!inCode) {
        closeList();
        inCode = true;
        codeLang = fence[1].trim();
        codeBuf = [];
      } else {
        out.push(
          `<pre><code class="lang-${escapeHtml(codeLang)}">${escapeHtml(
            codeBuf.join("\n")
          )}</code></pre>`
        );
        inCode = false;
        codeLang = "";
        codeBuf = [];
      }
      continue;
    }
    if (inCode) {
      codeBuf.push(line);
      continue;
    }

    // 空行
    if (line.trim() === "") {
      closeList();
      continue;
    }

    // 标题
    const h = line.match(/^(#{1,6})\s+(.*)$/);
    if (h) {
      closeList();
      const level = h[1].length;
      out.push(`<h${level}>${inline(h[2])}</h${level}>`);
      continue;
    }

    // 引用
    const bq = line.match(/^>\s?(.*)$/);
    if (bq) {
      closeList();
      out.push(`<blockquote>${inline(bq[1])}</blockquote>`);
      continue;
    }

    // 无序列表
    const ul = line.match(/^[-*+]\s+(.*)$/);
    if (ul) {
      if (listType !== "ul") {
        closeList();
        out.push("<ul>");
        listType = "ul";
      }
      out.push(`<li>${inline(ul[1])}</li>`);
      continue;
    }

    // 有序列表
    const ol = line.match(/^\d+\.\s+(.*)$/);
    if (ol) {
      if (listType !== "ol") {
        closeList();
        out.push("<ol>");
        listType = "ol";
      }
      out.push(`<li>${inline(ol[1])}</li>`);
      continue;
    }

    // 普通段落
    closeList();
    out.push(`<p>${inline(line)}</p>`);
  }

  if (inCode) {
    out.push(`<pre><code>${escapeHtml(codeBuf.join("\n"))}</code></pre>`);
  }
  closeList();

  return out.join("\n");
}

function inline(text: string): string {
  let s = escapeHtml(text);
  // 行内代码
  s = s.replace(/`([^`]+)`/g, (_m, c) => `<code>${c}</code>`);
  // 粗体
  s = s.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  // 斜体
  s = s.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  // 链接
  s = s.replace(
    /\[([^\]]+)\]\(([^)]+)\)/g,
    (_match, label: string, href: string) =>
      `<a href="${safeHref(href)}" target="_blank" rel="noopener noreferrer">${label}</a>`
  );
  return s;
}
