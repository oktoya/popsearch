import MarkdownIt from "markdown-it";
import hljs from "highlight.js/lib/common";

const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: false,
});

md.renderer.rules.fence = (tokens, idx) => {
  const token = tokens[idx];
  const raw = token.content;
  const lang = token.info.trim().split(/\s+/)[0] ?? "";
  const label = lang === "" ? "text" : lang;
  let body: string;
  if (lang !== "" && hljs.getLanguage(lang)) {
    body = hljs.highlight(raw, { language: lang, ignoreIllegals: true }).value;
  } else {
    body = md.utils.escapeHtml(raw);
  }
  const l = md.utils.escapeHtml(label);
  return (
    `<div class="code-block" data-lang="${md.utils.escapeHtml(lang)}" data-raw="${md.utils.escapeHtml(raw)}">` +
    `<div class="code-head"><span class="code-lang">${l}</span>` +
    `<button class="copy-btn" data-copy="code">복사</button></div>` +
    `<pre><code class="hljs">${body}</code></pre></div>`
  );
};

// Hangul / CJK / full-width characters (a URL stops before these, except in a path after "/").
const CJK_RE = /[ᄀ-ᇿ　-ヿ㄰-㆏㐀-鿿가-힣豈-﫿＀-￯]/;

// Length of the URL at the start of `s` (which must begin with a scheme or "www.").
export function urlLength(s: string): number {
  let end = 0;
  let prevCjkOk = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (/\s/.test(c) || c === "<" || c === ">") break;
    if (CJK_RE.test(c)) {
      if (i > 0 && (s[i - 1] === "/" || prevCjkOk)) {
        prevCjkOk = true;
      } else {
        break;
      }
    } else {
      prevCjkOk = false;
    }
    end = i + 1;
  }
  let url = s.slice(0, end);
  for (;;) {
    const last = url[url.length - 1];
    if (last === undefined) break;
    if (".,!?'\"]".includes(last)) {
      url = url.slice(0, -1);
    } else if (last === ")") {
      const open = url.split("(").length - 1;
      const close = url.split(")").length - 1;
      if (close > open) url = url.slice(0, -1);
      else break;
    } else {
      break;
    }
  }
  return url.length;
}

// Split plain text into text nodes and <a> elements (DOM only, never innerHTML).
export function linkifyToNodes(text: string): Node[] {
  const nodes: Node[] = [];
  const re = /(?<![\w@./-])(?:https?:\/\/|www\.)\S+/gi;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const len = urlLength(m[0]);
    const prefix = /^https?:\/\//i.exec(m[0])?.[0] ?? "www.";
    if (len <= prefix.length) {
      re.lastIndex = m.index + prefix.length;
      continue;
    }
    const url = m[0].slice(0, len);
    if (m.index > last) nodes.push(document.createTextNode(text.slice(last, m.index)));
    const a = document.createElement("a");
    a.href = /^https?:\/\//i.test(url) ? url : "https://" + url;
    a.textContent = url;
    nodes.push(a);
    last = m.index + len;
    re.lastIndex = last;
  }
  if (last < text.length) nodes.push(document.createTextNode(text.slice(last)));
  return nodes;
}

// Shorten linkify-generated links so they stop before Hangul/CJK and trailing punctuation.
md.core.ruler.after("linkify", "linkfix", (state) => {
  for (const block of state.tokens) {
    if (block.type !== "inline" || !block.children) continue;
    const kids = block.children;
    for (let i = 0; i < kids.length; i++) {
      const open = kids[i];
      if (open.type !== "link_open" || open.markup !== "linkify") continue;
      const textTok = kids[i + 1];
      const close = kids[i + 2];
      if (!textTok || textTok.type !== "text" || !close || close.type !== "link_close") continue;
      const href = String(open.attrGet("href") ?? "");
      if (!/^https?:/i.test(href)) continue;
      const text = textTok.content;
      const len = urlLength(text);
      if (len <= 0 || len >= text.length) continue;
      const newText = text.slice(0, len);
      open.attrSet("href", md.normalizeLink(/^https?:\/\//i.test(newText) ? newText : "http://" + newText));
      textTok.content = newText;
      const rest = new state.Token("text", "", 0);
      rest.content = text.slice(len);
      kids.splice(i + 3, 0, rest);
    }
  }
});

export function renderMarkdown(src: string): string {
  return md.render(src);
}
