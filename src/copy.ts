const COPY_LABEL = "복사";

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&" + "amp;")
    .replace(/</g, "&" + "lt;")
    .replace(/>/g, "&" + "gt;")
    .replace(/"/g, "&" + "quot;");
}

export async function copyRich(html: string, plain: string): Promise<void> {
  await navigator.clipboard.write([
    new ClipboardItem({
      "text/html": new Blob([html], { type: "text/html" }),
      "text/plain": new Blob([plain], { type: "text/plain" }),
    }),
  ]);
}

// innerText needs layout, so measure the clone while it is attached off-screen.
function innerTextOf(el: HTMLElement): string {
  const holder = document.createElement("div");
  holder.style.position = "fixed";
  holder.style.left = "-99999px";
  holder.style.top = "0";
  holder.style.width = "700px";
  holder.appendChild(el);
  document.body.appendChild(holder);
  const text = el.innerText;
  holder.remove();
  return text;
}

function codeHtml(raw: string): string {
  return `<pre><code>${escapeHtml(raw)}</code></pre>`;
}

export function copyCodeBlock(block: HTMLElement): Promise<void> {
  const raw = block.dataset.raw ?? "";
  return copyRich(codeHtml(raw), raw);
}

export function copyBlock(el: HTMLElement): Promise<void> {
  if (el.classList.contains("code-block")) return copyCodeBlock(el);
  const clone = el.cloneNode(true) as HTMLElement;
  clone.querySelectorAll(".copy-btn").forEach((b) => b.remove());
  const html = clone.outerHTML;
  const plain = innerTextOf(clone);
  return copyRich(html, plain);
}

export function copyAnswer(answer: HTMLElement): Promise<void> {
  const clone = answer.cloneNode(true) as HTMLElement;
  clone.querySelectorAll(".copy-btn").forEach((b) => b.remove());
  clone.querySelectorAll(".code-head").forEach((h) => h.remove());
  clone.querySelectorAll<HTMLElement>(".code-block").forEach((cb) => {
    const holder = document.createElement("div");
    holder.innerHTML = codeHtml(cb.dataset.raw ?? "");
    cb.replaceWith(holder.firstElementChild as Element);
  });
  const html = clone.innerHTML;
  const plain = innerTextOf(clone);
  return copyRich(html, plain);
}

export function flashCopied(btn: HTMLElement, original: string = COPY_LABEL): void {
  btn.textContent = "✓";
  setTimeout(() => {
    btn.textContent = original;
  }, 1000);
}
