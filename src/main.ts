import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { renderMarkdown, linkifyToNodes } from "./render";
import { copyAnswer, copyBlock, copyCodeBlock, flashCopied } from "./copy";
import { askApi, resetConversation, type ModelKey, type Source } from "./provider_api";
import { fileToAttachment, rawImageToAttachment, MAX_ATTACHMENTS, type Attachment } from "./images";
import { initSettingsPanel, loadSettings, capturingHotkey, type Settings } from "./settings";
import { normalizeFences } from "./export";
import { initHistoryPanel, type ConversationDetail } from "./history";

const IS_MAC = navigator.userAgent.includes("Mac");
if (IS_MAC) document.getElementById("history-btn")?.setAttribute("title", "기록 (Cmd+H)");
const input = document.getElementById("query") as HTMLTextAreaElement;
const results = document.getElementById("results") as HTMLDivElement;

const chips = document.getElementById("chips") as HTMLDivElement;
let attachments: Attachment[] = [];

const attachError = document.createElement("div");
attachError.id = "attach-error";
attachError.hidden = true;
chips.after(attachError);
let attachErrorTimer = 0;

function showAttachError(msg: string) {
  attachError.textContent = msg;
  attachError.hidden = false;
  clearTimeout(attachErrorTimer);
  attachErrorTimer = window.setTimeout(() => {
    attachError.hidden = true;
  }, 4000);
}

function truncateName(name: string): string {
  const chars = Array.from(name);
  return chars.length > 24 ? chars.slice(0, 24).join("") + "…" : name;
}

function renderChips() {
  chips.textContent = "";
  attachments.forEach((att, i) => {
    const chip = document.createElement("div");
    chip.className = "chip";
    if (att.kind === "image") {
      const img = document.createElement("img");
      img.src = att.thumb;
      chip.appendChild(img);
    } else {
      chip.classList.add("file-chip");
      const label = document.createElement("span");
      label.className = "chip-name";
      label.textContent = "📄 " + truncateName(att.name);
      chip.appendChild(label);
    }
    const rm = document.createElement("button");
    rm.type = "button";
    rm.className = "chip-remove";
    rm.textContent = "\u2715";
    rm.addEventListener("click", () => {
      attachments.splice(i, 1);
      renderChips();
      input.focus();
    });
    chip.appendChild(rm);
    chips.appendChild(chip);
  });
  chips.hidden = attachments.length === 0;
  updateModelPill();
}

function clearAttachments() {
  attachments = [];
  renderChips();
}

async function addFiles(files: File[]) {
  for (const file of files) {
    if (attachments.length >= MAX_ATTACHMENTS) break;
    try {
      const res = await fileToAttachment(file);
      if ("error" in res) {
        showAttachError(res.error);
      } else if (attachments.length < MAX_ATTACHMENTS) {
        attachments.push(res.att);
        renderChips();
      }
    } catch (_) {
      showAttachError(`지원하지 않는 형식입니다: ${file.name}`);
    }
  }
}

input.addEventListener("paste", (e) => {
  const items = Array.from(e.clipboardData?.items ?? []);
  const files = items
    .filter((it) => it.kind === "file")
    .map((it) => it.getAsFile())
    .filter((f): f is File => f !== null);
  if (files.length === 0) return;
  e.preventDefault();
  addFiles(files);
});

window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => {
  e.preventDefault();
  const files = Array.from(e.dataTransfer?.files ?? []);
  if (files.length > 0) addFiles(files);
});

let currentAnswer: HTMLDivElement | null = null;
let currentRaw = "";
let renderQueued = false;
let renderFrame = 0;
let settings: Settings = {
  provider: "cli",
  web_search: false,
  hotkey: "CommandOrControl+Shift+Space",
  capture_selection: true,
  default_model: "haiku",
};
let conversationId: number | null = null;
let convEpoch = 0;
let pendingSave: {
  text: string;
  thumbs: string[];
  provider: string;
  model: ModelKey;
  imageCount: number;
  fileNames: string[];
  ctxKey: string;
} | null = null;

interface Turn {
  question: string;
  answer: string;
  model: string;
  imageCount: number;
  fileNames: string[];
}
// Finished turns of the live conversation, and how many of them each context
// (API: "api"; CLI: one per model key) has already seen.
let transcript: Turn[] = [];
let seenCounts: Record<string, number> = {};

const PREFIX_KEYS: Record<string, ModelKey> = {
  h: "haiku",
  "ㅗ": "haiku",
  s: "sonnet",
  "ㄴ": "sonnet",
  o: "opus",
  "ㅐ": "opus",
};

function defaultModel(): ModelKey {
  const m = settings.default_model;
  return m === "sonnet" || m === "opus" ? m : "haiku";
}

const CODE_KEYS: Record<string, ModelKey> = {
  KeyH: "haiku",
  KeyS: "sonnet",
  KeyO: "opus",
};

// Pure decision part: which model a leading "/x " prefix selects, or null.
function decidePrefix(
  text: string,
  hasImages: boolean,
  prefixCode: string | null,
): { model: ModelKey; text: string } | null {
  if (text.length < 2 || text[0] !== "/") return null;
  let rest: string | null = null;
  if (text.length >= 3 && text[2] === " ") rest = text.slice(3);
  else if (hasImages && text.length === 2) rest = "";
  if (rest === null) return null;
  const ch = text[1].toLowerCase();
  const latin = ch === "h" || ch === "s" || ch === "o" ? PREFIX_KEYS[ch] : undefined;
  const model =
    latin ??
    (prefixCode ? CODE_KEYS[prefixCode] : undefined) ??
    PREFIX_KEYS[ch];
  if (!model) return null;
  return { model, text: rest };
}

// Physical key used for the prefix letter (works on any keyboard layout).
let prefixCode: string | null = null;

function parseModel(
  text: string,
  hasImages: boolean,
): { model: ModelKey; text: string; prefixed: boolean } {
  const d = decidePrefix(text, hasImages, prefixCode);
  if (d) return { model: d.model, text: d.text, prefixed: true };
  if (modelOverride) return { model: modelOverride, text, prefixed: true };
  return { model: defaultModel(), text, prefixed: false };
}

// One-question model override set with Alt+Up / Alt+Down (null = none).
let modelOverride: ModelKey | null = null;
const MODEL_ORDER: ModelKey[] = ["haiku", "sonnet", "opus"];

function stepModel(dir: 1 | -1) {
  const cur = parseModel(input.value, attachments.length > 0).model;
  const idx = MODEL_ORDER.indexOf(cur) + dir;
  if (idx < 0 || idx >= MODEL_ORDER.length) return;
  modelOverride = MODEL_ORDER[idx];
  updateModelPill();
}

const MODEL_NAMES: Record<ModelKey, string> = { haiku: "Haiku", sonnet: "Sonnet", opus: "Opus" };
const modelPill = document.getElementById("model-pill") as HTMLSpanElement;

function updateModelPill() {
  const parsed = parseModel(input.value, attachments.length > 0);
  modelPill.textContent = MODEL_NAMES[parsed.model] + (parsed.prefixed ? " (이번만)" : "");
  modelPill.classList.toggle("once", parsed.prefixed);
}

function transcriptToContextText(turns: Turn[]): string {
  const parts: string[] = [];
  for (const t of turns) {
    parts.push("## 질문");
    if (t.question.trim() !== "") parts.push(t.question);
    for (let i = 0; i < t.imageCount; i++) parts.push("(이미지 첨부됨)");
    for (const n of t.fileNames) parts.push(`(파일 첨부됨: ${n})`);
    parts.push("## 답변");
    parts.push(normalizeFences(t.answer));
  }
  return parts.join("\n\n") + "\n";
}

function buildProviderText(text: string, ctxKey: string): string {
  const unseen = transcript.slice(seenCounts[ctxKey] ?? 0);
  if (unseen.length === 0) return text;
  return (
    "다음은 이전 대화 기록입니다. 이 맥락을 이어서 마지막 질문에 답하세요.\n\n" +
    "<previous_conversation>\n" +
    transcriptToContextText(unseen) +
    "\n</previous_conversation>\n\n" +
    "질문: " +
    text
  );
}

// Live (question) screen is visible again: put the caret in the question input.
function focusQuery() {
  if (historyPanel.isOpen() || panel.isOpen()) return;
  input.focus();
  const n = input.value.length;
  input.setSelectionRange(n, n);
  autoGrow();
}

const panel = initSettingsPanel(
  (s) => {
    settings = s;
    updateModelPill();
  },
  () => focusQuery(),
  () => {
    for (const k of Object.keys(seenCounts)) {
      if (k !== "api") seenCounts[k] = 0;
    }
  },
);
loadSettings().then((s) => {
  settings = s;
  updateModelPill();
});

function buildQuestion(
  text: string,
  thumbs: string[],
  model: string,
  fileNames: string[] = [],
): HTMLDivElement {
  const q = document.createElement("div");
  q.className = "question";
  if (thumbs.length > 0 || fileNames.length > 0) {
    const row = document.createElement("div");
    row.className = "question-images";
    thumbs.forEach((src) => {
      const img = document.createElement("img");
      img.src = src;
      row.appendChild(img);
    });
    fileNames.forEach((n) => {
      const f = document.createElement("span");
      f.className = "file-chip-static";
      f.textContent = "📄 " + truncateName(n);
      row.appendChild(f);
    });
    q.appendChild(row);
  }
  const label = document.createElement("div");
  if (model === "sonnet" || model === "opus") {
    const badge = document.createElement("span");
    badge.className = "model-badge";
    badge.textContent = model === "sonnet" ? "Sonnet" : "Opus";
    label.appendChild(badge);
  }
  linkifyToNodes(text).forEach((n) => label.appendChild(n));
  q.appendChild(label);
  return q;
}

const DEFAULT_PLACEHOLDER = "무엇이든 물어보세요";

function renderMessages(body: HTMLElement, conv: ConversationDetail) {
  conv.messages.forEach((m) => {
    const a = document.createElement("div");
    a.className = "answer";
    a.innerHTML = renderMarkdown(m.answer);
    body.appendChild(buildQuestion(m.question, m.images, m.model));
    body.appendChild(a);
    addCopyButtons(a);
    addPreviews(a);
    addSources(a, m.sources ?? []);
  });
}

const historyPanel = initHistoryPanel({
  renderConversation: renderMessages,
  onOpen: () => panel.close(true),
  onClose: () => focusQuery(),
  onViewChange: (conv) => {
    if (!conv) {
      input.placeholder = DEFAULT_PLACEHOLDER;
      return;
    }
    const t = conv.title.length > 30 ? conv.title.slice(0, 30) + "…" : conv.title;
    input.placeholder = `「${t}」에 이어서 질문`;
  },
  onDeleted: (id) => {
    if (conversationId === id) conversationId = null;
  },
});
document.getElementById("settings-btn")!.addEventListener("click", () => {
  historyPanel.close(true);
});

invoke<string | null>("get_hotkey_error").then(async (err) => {
  if (!err) return;
  historyPanel.close(true);
  await panel.open();
  const stored = await loadSettings();
  panel.showHotkeyMessage(
    `단축키(${stored.hotkey})를 등록하지 못했습니다. 다른 단축키를 지정하세요.`,
  );
});

function renderCurrent() {
  renderQueued = false;
  if (!currentAnswer) return;
  currentAnswer.innerHTML = renderMarkdown(currentRaw);
  scrollToBottom();
}

function scheduleRender() {
  if (renderQueued) return;
  renderQueued = true;
  renderFrame = requestAnimationFrame(renderCurrent);
}

function cancelPendingRender() {
  if (renderQueued) cancelAnimationFrame(renderFrame);
  renderQueued = false;
}

function addCopyButtons(answer: HTMLDivElement) {
  Array.from(answer.children).forEach((child) => {
    if (!(child instanceof HTMLElement)) return;
    if (child.tagName === "HR") return;
    if (child.classList.contains("code-block")) return;
    child.classList.add("has-copy");
    const btn = document.createElement("button");
    btn.className = "copy-btn block-copy";
    btn.textContent = "복사";
    child.appendChild(btn);
  });
  const actions = document.createElement("div");
  actions.className = "answer-actions";
  const all = document.createElement("button");
  all.className = "copy-all";
  all.textContent = "전체 복사";
  actions.appendChild(all);
  answer.after(actions);
}

type LinkKind = { kind: "youtube"; id: string } | { kind: "image" } | null;

function classifyLink(href: string): LinkKind {
  let u: URL;
  try {
    u = new URL(href);
  } catch (_) {
    return null;
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return null;
  const host = u.hostname.toLowerCase().replace(/^(www|m)\./, "");
  const idOk = (s: string | null | undefined) => (s && /^[A-Za-z0-9_-]{6,20}$/.test(s) ? s : null);
  if (host === "youtube.com") {
    if (u.pathname === "/watch") {
      const id = idOk(u.searchParams.get("v"));
      if (id) return { kind: "youtube", id };
    }
    const m = u.pathname.match(/^\/shorts\/([^/]+)/);
    const id = idOk(m && m[1]);
    if (id) return { kind: "youtube", id };
  } else if (host === "youtu.be") {
    const id = idOk(u.pathname.slice(1).split("/")[0]);
    if (id) return { kind: "youtube", id };
  }
  if (/\.(png|jpe?g|gif|webp)$/i.test(u.pathname)) return { kind: "image" };
  return null;
}

// Previews for links in a finished answer. Placed right after the answer element
// (before its answer-actions); never part of the copied answer.
function addPreviews(answer: HTMLElement) {
  const inline = new Set<string>();
  answer.querySelectorAll("img").forEach((img) => {
    img.loading = "lazy";
    img.addEventListener("error", () => img.remove());
    inline.add(img.src);
  });
  const seen = new Set<string>();
  const cards: HTMLElement[] = [];
  let yt = 0;
  let imgs = 0;
  answer.querySelectorAll("a").forEach((a) => {
    const href = a.href;
    if (!href || seen.has(href)) return;
    const c = classifyLink(href);
    if (!c) return;
    const label = (a.textContent || "").trim();
    if (c.kind === "youtube") {
      if (yt >= 3) return;
      seen.add(href);
      yt++;
      const card = document.createElement("div");
      card.className = "preview-card preview-yt";
      card.dataset.url = href;
      const wrap = document.createElement("div");
      wrap.className = "preview-thumb";
      const img = document.createElement("img");
      img.src = `https://i.ytimg.com/vi/${c.id}/hqdefault.jpg`;
      img.loading = "lazy";
      img.addEventListener("error", () => card.remove());
      const play = document.createElement("span");
      play.className = "preview-play";
      play.textContent = "▶";
      wrap.appendChild(img);
      wrap.appendChild(play);
      const cap = document.createElement("div");
      cap.className = "preview-caption";
      cap.textContent = label && label !== href ? label : href;
      card.appendChild(wrap);
      card.appendChild(cap);
      cards.push(card);
    } else {
      if (imgs >= 4 || inline.has(href)) return;
      seen.add(href);
      imgs++;
      const card = document.createElement("div");
      card.className = "preview-card preview-img";
      card.dataset.url = href;
      const img = document.createElement("img");
      img.src = href;
      img.loading = "lazy";
      img.addEventListener("error", () => card.remove());
      card.appendChild(img);
      cards.push(card);
    }
  });
  if (cards.length === 0) return;
  const strip = document.createElement("div");
  strip.className = "preview-strip";
  cards.forEach((c) => strip.appendChild(c));
  answer.after(strip);
}

function sourceDomain(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch (_) {
    return url;
  }
}

// Sources block: placed before the answer's answer-actions (after any preview
// strip); never part of the copied answer.
function addSources(answer: HTMLElement, sources: Source[]) {
  if (sources.length === 0) return;
  const box = document.createElement("div");
  box.className = "sources";
  const head = document.createElement("div");
  head.className = "sources-title";
  head.textContent = "출처";
  box.appendChild(head);
  sources.forEach((s, i) => {
    const row = document.createElement("div");
    row.className = "source-item";
    row.dataset.url = s.url;
    const domain = sourceDomain(s.url);
    row.textContent = `${i + 1}. ${s.title && s.title.trim() !== "" ? s.title : domain} — ${domain}`;
    box.appendChild(row);
  });
  let n = answer.nextElementSibling;
  while (n && !n.classList.contains("answer-actions")) n = n.nextElementSibling;
  if (n) n.before(box);
  else answer.after(box);
}

function scrollToBottom() {
  results.scrollTop = results.scrollHeight;
}

function enableInput(focus: boolean) {
  input.disabled = false;
  if (focus) input.focus();
}

function send(text: string, model: ModelKey, forceWeb = false) {
  const web = settings.web_search || forceWeb;
  const atts = attachments;
  clearAttachments();
  historyPanel.close(true);
  const thumbs: string[] = [];
  const fileNames: string[] = [];
  for (const a of atts) {
    if (a.kind === "image") thumbs.push(a.thumb);
    else fileNames.push(a.name);
  }
  const q = buildQuestion(text, thumbs, model, fileNames);
  const ctxKey = settings.provider === "api" ? "api" : web ? model + ":web" : model;
  const providerText = buildProviderText(text, ctxKey);
  pendingSave = {
    text,
    thumbs,
    provider: settings.provider,
    model,
    imageCount: thumbs.length,
    fileNames,
    ctxKey,
  };
  const a = document.createElement("div");
  a.className = "answer";
  results.appendChild(q);
  results.appendChild(a);
  currentAnswer = a;
  currentRaw = "";
  input.value = "";
  prefixCode = null;
  modelOverride = null;
  autoGrow();
  input.disabled = true;
  scrollToBottom();
  if (settings.provider === "api") {
    askApi(providerText, atts, web, model, {
      onChunk,
      onDone,
      onError,
      onStatus: showStatus,
    });
  } else {
    invoke("ask", {
      text: providerText,
      model,
      webSearch: web,
      attachments: atts.map((a) =>
        a.kind === "image"
          ? { kind: "image", media_type: a.mediaType, data: a.data, name: a.name }
          : a.kind === "pdf"
            ? { kind: "pdf", media_type: "application/pdf", data: a.data, name: a.name }
            : { kind: "text", text: a.text, name: a.name },
      ),
    });
  }
}

function autoGrow() {
  input.style.height = "auto";
  const lh = parseFloat(getComputedStyle(input).lineHeight) || 28;
  const pad = input.offsetHeight - input.clientHeight;
  const maxH = lh * 6 + 32 + pad;
  input.style.height = Math.min(input.scrollHeight + pad, maxH) + "px";
  updateModelPill();
}
input.addEventListener("input", () => {
  if (input.value.length < 2 || input.value[0] !== "/") prefixCode = null;
  autoGrow();
});
input.addEventListener("keydown", (e) => {
  if (e.ctrlKey || e.altKey || e.metaKey) return;
  if (input.value === "/" && input.selectionStart === 1 && input.selectionEnd === 1) {
    prefixCode = e.code;
  }
});

listen<{ media_type: string; data: string }>("popsearch://selection-image", (e) => {
  if (attachments.length >= MAX_ATTACHMENTS) return;
  rawImageToAttachment(e.payload)
    .then((att) => {
      if (att && attachments.length < MAX_ATTACHMENTS) {
        attachments.push(att);
        renderChips();
      }
    })
    .catch(() => {});
});

listen<{
  selection: string | null;
  image?: { media_type: string; data: string } | null;
}>("popsearch://shown", (e) => {
  const sel = e.payload?.selection;
  const img = e.payload?.image;
  if (img) {
    rawImageToAttachment(img)
      .then((att) => {
        if (att && attachments.length < MAX_ATTACHMENTS) {
          attachments.push(att);
          renderChips();
        }
      })
      .catch(() => {});
  }
  const hasSel = typeof sel === "string" && sel.trim() !== "";
  if (historyPanel.isListOpen() && !panel.isOpen() && !hasSel) {
    historyPanel.focusSearch();
  } else {
    input.focus();
  }
  if (typeof sel === "string" && sel.trim() !== "") {
    input.value = sel;
    prefixCode = null;
    input.setSelectionRange(sel.length, sel.length);
    autoGrow();
    input.scrollTop = input.scrollHeight;
  } else {
    input.select();
  }
});

let statusEl: HTMLElement | null = null;

// Muted "searching" line right below the current answer; never part of the answer.
function showStatus(text: string | null) {
  if (text === null || !currentAnswer) {
    statusEl?.remove();
    statusEl = null;
    return;
  }
  if (!statusEl) {
    statusEl = document.createElement("div");
    statusEl.className = "answer-status";
  }
  statusEl.textContent = text;
  if (statusEl.previousElementSibling !== currentAnswer) currentAnswer.after(statusEl);
  scrollToBottom();
}

function onChunk(text: string) {
  if (!currentAnswer) return;
  currentRaw += text;
  scheduleRender();
}

async function onDone(sources: Source[] = []) {
  showStatus(null);
  cancelPendingRender();
  if (currentAnswer) {
    renderCurrent();
    addCopyButtons(currentAnswer);
    addPreviews(currentAnswer);
    addSources(currentAnswer, sources);
    scrollToBottom();
  }
  const save = pendingSave;
  pendingSave = null;
  if (save) {
    transcript.push({
      question: save.text,
      answer: currentRaw,
      model: save.model,
      imageCount: save.imageCount,
      fileNames: save.fileNames,
    });
    seenCounts[save.ctxKey] = transcript.length;
    const epoch = convEpoch;
    try {
      const id = await invoke<number>("history_add", {
        conversationId,
        provider: save.provider,
        model: save.model,
        question: save.text + save.fileNames.map((n) => `\n(첨부: ${n})`).join(""),
        images: save.thumbs,
        answer: currentRaw,
        sources,
      });
      if (epoch === convEpoch) conversationId = id;
    } catch (_) {
      // history is best-effort; do not block the conversation
    }
  }
  enableInput(true);
}

function onError(message: string) {
  showStatus(null);
  cancelPendingRender();
  pendingSave = null;
  if (currentAnswer) {
    currentRaw = "";
    currentAnswer.textContent = message;
    currentAnswer.classList.add("error");
  }
  enableInput(true);
}

listen<{ text: string }>("popsearch://chunk", (e) => onChunk(e.payload.text));
listen<{ text: string | null }>("popsearch://status", (e) => showStatus(e.payload.text));
listen("popsearch://done", () => onDone());
listen<{ message: string }>("popsearch://error", (e) => onError(e.payload.message));

async function onResultsClick(e: MouseEvent) {
  const target = e.target as HTMLElement;
  const link = target.closest("a");
  if (link && link.href) {
    e.preventDefault();
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed && sel.toString() !== "") return;
    openUrl(link.href);
    return;
  }
  const card = target.closest(".preview-card, .source-item") as HTMLElement | null;
  if (card && card.dataset.url) {
    openUrl(card.dataset.url);
    return;
  }
  const btn = target.closest("button") as HTMLButtonElement | null;
  if (!btn) return;
  try {
    if (btn.classList.contains("copy-all")) {
      let answer = btn.parentElement?.previousElementSibling as HTMLElement | null;
      while (answer && !answer.classList.contains("answer")) {
        answer = answer.previousElementSibling as HTMLElement | null;
      }
      if (!answer) return;
      await copyAnswer(answer);
      flashCopied(btn, "전체 복사");
    } else if (btn.dataset.copy === "code") {
      const block = btn.closest(".code-block") as HTMLElement | null;
      if (!block) return;
      await copyCodeBlock(block);
      flashCopied(btn);
    } else if (btn.classList.contains("block-copy")) {
      const block = btn.parentElement as HTMLElement | null;
      if (!block) return;
      await copyBlock(block);
      flashCopied(btn);
    }
  } catch (_) {
    // clipboard write failed; leave the button unchanged
  }
}
results.addEventListener("click", onResultsClick);
document.getElementById("history-view-body")!.addEventListener("click", onResultsClick);

async function resetAll() {
  showStatus(null);
  cancelPendingRender();
  convEpoch++;
  conversationId = null;
  pendingSave = null;
  transcript = [];
  seenCounts = {};
  resetConversation();
  await invoke("new_conversation");
  results.textContent = "";
  currentAnswer = null;
  input.value = "";
  prefixCode = null;
  modelOverride = null;
  autoGrow();
  clearAttachments();
  enableInput(false);
}

async function continueFrom(conv: ConversationDetail) {
  cancelPendingRender();
  convEpoch++;
  pendingSave = null;
  transcript = conv.messages.map((m) => ({
    question: m.question,
    answer: m.answer,
    model: m.model,
    imageCount: m.images.length,
    fileNames: [],
  }));
  seenCounts = {};
  resetConversation();
  await invoke("new_conversation");
  results.textContent = "";
  currentAnswer = null;
  renderMessages(results, conv);
  conversationId = conv.id;
  modelOverride = null;
  updateModelPill();
  focusQuery();
}

let pendingWebSend = false;
let pendingWebTimer: number | null = null;
let imeComposing = false;

function clearPendingWebSend() {
  pendingWebSend = false;
  if (pendingWebTimer !== null) {
    clearTimeout(pendingWebTimer);
    pendingWebTimer = null;
  }
}

function submitQuery(forceWeb: boolean) {
  clearPendingWebSend();
  const parsed = parseModel(input.value, attachments.length > 0);
  const text = parsed.text;
  if (text.trim() === "" && attachments.length === 0) return;
  if (historyPanel.isViewing()) {
    const conv = historyPanel.viewing();
    if (!conv) return;
    historyPanel.close(true);
    const keepAttachments = attachments;
    input.disabled = true;
    continueFrom(conv).then(() => {
      attachments = keepAttachments;
      renderChips();
      send(text, parsed.model, forceWeb);
    });
    return;
  }
  send(text, parsed.model, forceWeb);
}

input.addEventListener("compositionstart", () => {
  imeComposing = true;
  clearPendingWebSend();
});

input.addEventListener("compositionend", () => {
  imeComposing = false;
  if (!pendingWebSend) return;
  clearPendingWebSend();
  setTimeout(() => submitQuery(true), 0);
});

input.addEventListener("keydown", (e) => {
  const isEnterKey = e.key === "Enter" || e.code === "Enter" || e.code === "NumpadEnter";
  if (!isEnterKey) return;
  const forceWeb = e.altKey || (IS_MAC && e.metaKey);
  if (e.isComposing || e.keyCode === 229) {
    if (e.shiftKey || !forceWeb) return;
    e.preventDefault();
    clearPendingWebSend();
    pendingWebSend = true;
    pendingWebTimer = window.setTimeout(() => {
      pendingWebTimer = null;
      if (!pendingWebSend) return;
      pendingWebSend = false;
      if (!imeComposing) submitQuery(true);
    }, 150);
    return;
  }
  if (e.key !== "Enter") return;
  if (e.shiftKey) return;
  e.preventDefault();
  submitQuery(forceWeb);
});

window.addEventListener("keydown", async (e) => {
  if (capturingHotkey) return;
  if (
    historyPanel.isListOpen() &&
    !panel.isOpen() &&
    !e.ctrlKey &&
    !e.altKey &&
    !e.metaKey &&
    (e.key.length === 1 || e.key === "Backspace")
  ) {
    const ae = document.activeElement;
    if (!ae || (ae.tagName !== "INPUT" && ae.tagName !== "TEXTAREA")) {
      historyPanel.focusSearch();
    }
  }
  if (
    !historyPanel.isOpen() &&
    !panel.isOpen() &&
    !e.ctrlKey &&
    !e.altKey &&
    !e.metaKey &&
    (e.key.length === 1 || e.key === "Backspace")
  ) {
    const ae = document.activeElement;
    if (!ae || (ae.tagName !== "INPUT" && ae.tagName !== "TEXTAREA")) {
      input.focus();
    }
  }
  if (
    e.altKey &&
    !e.ctrlKey &&
    !e.metaKey &&
    !e.shiftKey &&
    (e.key === "ArrowUp" || e.key === "ArrowDown") &&
    !panel.isOpen() &&
    !historyPanel.isListOpen()
  ) {
    e.preventDefault();
    stepModel(e.key === "ArrowUp" ? 1 : -1);
    return;
  }
  if (e.key === "F5") {
    e.preventDefault();
    if (panel.isOpen()) {
      panel.close();
    } else {
      historyPanel.close(true);
      panel.toggle();
    }
    return;
  }
  if (e.key === "Escape") {
    e.preventDefault();
    if (panel.isOpen()) {
      panel.close();
      return;
    }
    if (historyPanel.isOpen()) {
      historyPanel.close();
      return;
    }
    await resetAll();
    invoke("hide_window");
  }
  if (
    (e.ctrlKey || (IS_MAC && e.metaKey)) &&
    !e.shiftKey &&
    !e.altKey &&
    (e.key === "h" || e.key === "H" || e.code === "KeyH")
  ) {
    e.preventDefault();
    historyPanel.toggle();
  }
});
