import MarkdownIt from "markdown-it";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import type { ConversationDetail } from "./history";

// Same options as the renderer in render.ts.
const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: false,
});

function longestBacktickRun(s: string): number {
  let max = 0;
  for (const m of s.matchAll(/`+/g)) {
    if (m[0].length > max) max = m[0].length;
  }
  return max;
}

// Re-fence every top-level fenced code block so the fence is longer than any
// backtick run inside it. Text outside fences is left untouched.
export function normalizeFences(src: string): string {
  const tokens = md.parse(src, {});
  const lines = src.split("\n");
  const fences = tokens.filter((t) => t.type === "fence" && t.map && t.level === 0);
  for (let i = fences.length - 1; i >= 0; i--) {
    const t = fences[i];
    const [start, end] = t.map as [number, number];
    const fence = "`".repeat(Math.max(3, longestBacktickRun(t.content) + 1));
    const body = t.content.endsWith("\n") ? t.content.slice(0, -1) : t.content;
    const block = [fence + t.info, ...(t.content === "" ? [] : body.split("\n")), fence];
    lines.splice(start, end - start, ...block);
  }
  return lines.join("\n");
}

const MODEL_NAMES: Record<string, string> = { haiku: "Haiku", sonnet: "Sonnet", opus: "Opus" };

export function conversationToMarkdown(conv: ConversationDetail): string {
  const when = new Date(conv.created_at).toLocaleString();
  const parts: string[] = [`# ${conv.title}`, `_${when} · ${conv.provider}_`];
  for (const m of conv.messages) {
    parts.push("## 질문");
    if (m.question.trim() !== "") parts.push(m.question);
    for (const img of m.images) parts.push(`![image](${img})`);
    parts.push("## 답변");
    parts.push(`_${MODEL_NAMES[m.model] ?? m.model}${m.web ? " · 웹 검색" : ""}_`);
    parts.push(normalizeFences(m.answer));
    if (m.sources && m.sources.length > 0) {
      parts.push("### 출처");
      parts.push(m.sources.map((s) => `- [${s.title || s.url}](${s.url})`).join("\n"));
    }
  }
  return parts.join("\n\n") + "\n";
}

function safeFileName(title: string): string {
  const name = title
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, "_")
    .trim()
    .slice(0, 80)
    .trim();
  return name === "" ? "conversation" : name;
}

export async function saveConversationMarkdown(conv: ConversationDetail): Promise<void> {
  // The save dialog takes focus from the window, which would hide it.
  await invoke("set_suppress_hide", { value: true });
  try {
    const path = await save({
      defaultPath: `${safeFileName(conv.title)}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (path) {
      await invoke("save_text_file", { path, content: conversationToMarkdown(conv) });
    }
  } finally {
    await invoke("set_suppress_hide", { value: false });
  }
}
