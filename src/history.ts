import { invoke } from "@tauri-apps/api/core";
import { saveConversationMarkdown } from "./export";
import type { Source } from "./provider_api";

export interface HistoryMessage {
  question: string;
  images: string[];
  answer: string;
  model: string;
  sources: Source[];
  web: boolean;
}

export interface ConversationDetail {
  id: number;
  title: string;
  provider: string;
  model: string;
  created_at: number;
  messages: HistoryMessage[];
}

interface ConversationSummary {
  id: number;
  title: string;
  provider: string;
  model: string;
  updated_at: number;
}

export interface HistoryPanel {
  toggle: () => void;
  close: (silent?: boolean) => void;
  isOpen: () => boolean;
  isViewing: () => boolean;
  viewing: () => ConversationDetail | null;
  isListOpen: () => boolean;
  focusSearch: () => void;
}

export function initHistoryPanel(opts: {
  renderConversation: (body: HTMLElement, conv: ConversationDetail) => void;
  onOpen: () => void;
  onClose?: () => void;
  onViewChange?: (conv: ConversationDetail | null) => void;
  onDeleted?: (id: number) => void;
}): HistoryPanel {
  const panel = document.getElementById("history") as HTMLDivElement;
  const search = document.getElementById("history-search") as HTMLInputElement;
  const list = document.getElementById("history-list") as HTMLDivElement;
  const view = document.getElementById("history-view") as HTMLDivElement;
  const viewTitle = document.getElementById("history-view-title") as HTMLSpanElement;
  const viewBody = document.getElementById("history-view-body") as HTMLDivElement;
  const results = document.getElementById("results") as HTMLDivElement;
  const btn = document.getElementById("history-btn") as HTMLButtonElement;
  const exportBtn = document.getElementById("history-export") as HTMLButtonElement;
  const deleteBtn = document.getElementById("history-delete") as HTMLButtonElement;
  const closeBtn = document.getElementById("history-close") as HTMLButtonElement;

  let viewing: ConversationDetail | null = null;
  let debounce = 0;

  async function refreshList() {
    const items = await invoke<ConversationSummary[]>("history_list", {
      query: search.value,
      limit: 100,
    });
    list.textContent = "";
    if (items.length === 0) {
      const empty = document.createElement("div");
      empty.className = "history-empty";
      empty.textContent = "기록이 없습니다";
      list.appendChild(empty);
      return;
    }
    items.forEach((it) => {
      const row = document.createElement("div");
      row.className = "history-item";
      const title = document.createElement("div");
      title.className = "history-title";
      title.textContent = it.title;
      const meta = document.createElement("div");
      meta.className = "history-meta";
      meta.textContent = `${new Date(it.updated_at).toLocaleString()} · ${it.provider}`;
      const del = document.createElement("button");
      del.className = "history-item-delete";
      del.type = "button";
      del.title = "삭제";
      del.textContent = "✕";
      del.addEventListener("click", async (e) => {
        e.stopPropagation();
        try {
          await invoke("history_delete", { id: it.id });
          opts.onDeleted?.(it.id);
        } catch (_) {
          // ignore; list is refreshed below
        }
        await refreshList();
        search.focus();
      });
      row.appendChild(title);
      row.appendChild(meta);
      row.appendChild(del);
      row.addEventListener("click", () => openConversation(it.id));
      list.appendChild(row);
    });
  }

  async function openConversation(id: number) {
    const conv = await invoke<ConversationDetail>("history_get", { id });
    viewing = conv;
    viewTitle.textContent = conv.title;
    viewBody.textContent = "";
    opts.renderConversation(viewBody, conv);
    panel.hidden = true;
    view.hidden = false;
    results.hidden = true;
    opts.onViewChange?.(conv);
  }

  async function showList() {
    viewing = null;
    opts.onViewChange?.(null);
    view.hidden = true;
    viewBody.textContent = "";
    panel.hidden = false;
    results.hidden = true;
    await refreshList();
    search.focus();
  }

  async function open() {
    opts.onOpen();
    search.value = "";
    await showList();
  }

  function close(silent = false) {
    viewing = null;
    opts.onViewChange?.(null);
    panel.hidden = true;
    view.hidden = true;
    viewBody.textContent = "";
    results.hidden = false;
    if (!silent) opts.onClose?.();
  }

  function isOpen() {
    return !panel.hidden || !view.hidden;
  }

  btn.addEventListener("click", () => {
    if (isOpen()) close();
    else open();
  });

  search.addEventListener("input", () => {
    clearTimeout(debounce);
    debounce = window.setTimeout(refreshList, 200);
  });

  exportBtn.addEventListener("click", async () => {
    if (!viewing) return;
    try {
      await saveConversationMarkdown(viewing);
    } catch (_) {
      // dialog or write failed; nothing to do
    }
  });

  deleteBtn.addEventListener("click", async () => {
    if (!viewing) return;
    if (!window.confirm("이 기록을 삭제할까요?")) return;
    const delId = viewing.id;
    await invoke("history_delete", { id: delId });
    opts.onDeleted?.(delId);
    await showList();
  });

  closeBtn.addEventListener("click", () => {
    showList();
  });

  return {
    toggle: () => (isOpen() ? close() : open()),
    close,
    isOpen,
    isViewing: () => !view.hidden,
    viewing: () => viewing,
    isListOpen: () => !panel.hidden,
    focusSearch: () => search.focus(),
  };
}
