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

export function renderMarkdown(src: string): string {
  return md.render(src);
}
