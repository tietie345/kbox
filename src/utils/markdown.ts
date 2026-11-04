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

function escapeAttribute(value: string): string {
  return escapeHtml(value).replace(/"/g, "&quot;").replace(/'/g, "&#39;");
}

// 渲染图片标记：支持 data:/http(s): 直接 URL，以及 kbox-image://<id>（KBOX 库存图，预览时解析为 data URL）。
// width 为用户在编辑器中拖拽缩放后持久化的像素宽度。
function imageTag(alt: string, rawSrc: string, width?: number | null): string {
  const a = escapeAttribute(alt);
  const w = Number.isFinite(width as number) && (width as number) > 0 ? Math.round(width as number) : null;
  const sizeAttr = w ? ` width="${w}" style="width:${w}px;max-width:100%;height:auto;"` : "";
  const m = rawSrc.match(/^kbox-image:\/\/([a-zA-Z0-9_-]+)$/);
  if (m) {
    return `<img class="kbox-preview-image" data-kbox-image="${m[1]}" alt="${a}"${sizeAttr} />`;
  }
  const isSafeDataImage = /^data:image\/(?:png|jpe?g|gif|webp|bmp|avif);base64,[a-z0-9+/=\s]+$/i.test(rawSrc);
  if (isSafeDataImage || /^https?:\/\//i.test(rawSrc)) {
    return `<img class="kbox-preview-image" src="${escapeAttribute(rawSrc)}" alt="${a}"${sizeAttr} />`;
  }
  // 其他相对路径（库内资产）在 markdown 预览中无法直接解析，退回为带说明的占位。
  return `<span class="kbox-image-fallback">🖼 <a href="#" onclick="return false">${a || "图片"}</a></span>`;
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

/**
 * 将所见即所得编辑器输出的 HTML 转回 KBOX 持久化的 Markdown。
 * 只处理编辑器允许的结构，不保留任意 HTML 标签或样式。
 */
export function htmlToMarkdown(html: string): string {
  if (typeof DOMParser === "undefined") return html;
  const document = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");

  const inline = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
    if (!(node instanceof HTMLElement)) return "";
    const children = Array.from(node.childNodes).map(inline).join("");
    switch (node.tagName.toLowerCase()) {
      case "strong":
      case "b":
        return `**${children}**`;
      case "em":
      case "i":
        return `*${children}*`;
      case "s":
      case "del":
        return `~~${children}~~`;
      case "code":
        return `\`${children.replace(/`/g, "\\`")}\``;
      case "a": {
        const href = node.getAttribute("href") || "#";
        return `[${children}](${href})`;
      }
      case "img": {
        const alt = node.getAttribute("alt") || "图片";
        const objectId = node.getAttribute("data-kbox-image");
        const src = objectId ? `kbox-image://${objectId}` : node.getAttribute("src") || "";
        if (!src) return "";
        const widthAttr = node.getAttribute("width");
        const width = widthAttr ? Number(widthAttr) : NaN;
        const suffix = Number.isFinite(width) && width > 0 ? ` =${Math.round(width)}` : "";
        return `![${alt}](${src}${suffix})`;
      }
      case "br":
        return "\n";
      default:
        return children;
    }
  };

  const block = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
    if (!(node instanceof HTMLElement)) return "";
    const tag = node.tagName.toLowerCase();
    if (/^h[1-6]$/.test(tag)) {
      return `${"#".repeat(Number(tag[1]))} ${Array.from(node.childNodes).map(inline).join("")}\n\n`;
    }
    if (tag === "p") {
      return `${Array.from(node.childNodes).map(inline).join("")}\n\n`;
    }
    if (tag === "pre") {
      const code = node.textContent ?? "";
      const language = node.querySelector("code")?.className.match(/(?:language|lang)-([^\s]+)/)?.[1] ?? "";
      return `\`\`\`${language}\n${code.replace(/\n$/, "")}\n\`\`\`\n\n`;
    }
    if (tag === "blockquote") {
      const text = Array.from(node.childNodes).map(block).join("").trim();
      return `${text.split("\n").map((line) => `> ${line}`).join("\n")}\n\n`;
    }
    if (tag === "ul" || tag === "ol") {
      const ordered = tag === "ol";
      const items = Array.from(node.children)
        .filter((child) => child.tagName.toLowerCase() === "li")
        .map((item, index) => {
          const text = Array.from(item.childNodes)
            .map((child) => child instanceof HTMLElement && ["ul", "ol"].includes(child.tagName.toLowerCase())
              ? `\n${block(child).trimEnd()}`
              : inline(child))
            .join("")
            .trim();
          return `${ordered ? `${index + 1}.` : "-"} ${text}`;
        });
      return `${items.join("\n")}\n\n`;
    }
    if (tag === "hr") return "---\n\n";
    if (tag === "img") return `${inline(node)}\n\n`;
    return Array.from(node.childNodes).map(block).join("");
  };

  return Array.from(document.body.childNodes)
    .map(block)
    .join("")
    .replace(/\n{3,}/g, "\n\n")
    .trimEnd();
}

function inline(text: string): string {
  let s = escapeHtml(text);
  // 行内代码
  s = s.replace(/`([^`]+)`/g, (_m, c) => `<code>${c}</code>`);
  // 粗体
  s = s.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  // 删除线
  s = s.replace(/~~([^~]+)~~/g, "<s>$1</s>");
  // 斜体
  s = s.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  // 图片（需先于链接，避免被误作 [alt](src) 链接）；支持可选宽度后缀 =300
  s = s.replace(/!\[([^\]]*)\]\(([^)\s]+)(?:\s*=\s*(\d+))?\)/g, (_m, alt, src, width) =>
    imageTag(alt as string, src as string, width ? Number(width) : null)
  );
  // 链接
  s = s.replace(
    /\[([^\]]+)\]\(([^)]+)\)/g,
    (_match, label: string, href: string) =>
      `<a href="${safeHref(href)}" target="_blank" rel="noopener noreferrer">${label}</a>`
  );
  return s;
}
