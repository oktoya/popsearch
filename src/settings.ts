import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  provider: "cli" | "api";
  web_search: boolean;
  hotkey: string;
  capture_selection: boolean;
  default_model: "haiku" | "sonnet" | "opus";
  autostart?: boolean;
}

const DEFAULT_HOTKEY = "CommandOrControl+Shift+Space";

export let capturingHotkey = false;

function friendlyHotkey(acc: string): string {
  const isMac = navigator.userAgent.includes("Mac");
  return acc
    .split("+")
    .map((p) => {
      if (p === "CommandOrControl") return isMac ? "Cmd" : "Ctrl";
      if (p === "Super") return isMac ? "Cmd" : "Win";
      return p;
    })
    .join("+");
}

export async function loadSettings(): Promise<Settings> {
  return await invoke<Settings>("get_settings");
}

export function initSettingsPanel(
  onChange: (s: Settings) => void,
  onClose?: () => void,
  onCliReset?: () => void,
): {
  toggle: () => void;
  close: (silent?: boolean) => void;
  isOpen: () => boolean;
  open: () => Promise<void>;
  showHotkeyMessage: (msg: string) => void;
} {
  const panel = document.getElementById("settings") as HTMLDivElement;
  const results = document.getElementById("results") as HTMLDivElement;
  const gear = document.getElementById("settings-btn") as HTMLButtonElement;
  const radios = Array.from(
    panel.querySelectorAll<HTMLInputElement>('input[name="provider"]'),
  );
  const webSearch = document.getElementById("web-search") as HTMLInputElement;
  const keyInput = document.getElementById("api-key") as HTMLInputElement;
  const keySave = document.getElementById("api-key-save") as HTMLButtonElement;
  const keyDelete = document.getElementById("api-key-delete") as HTMLButtonElement;
  const keyStatus = document.getElementById("api-key-status") as HTMLSpanElement;

  const modelRadios = Array.from(
    panel.querySelectorAll<HTMLInputElement>('input[name="default-model"]'),
  );
  const captureSel = document.getElementById("capture-selection") as HTMLInputElement;
  const hotkeyField = document.getElementById("hotkey") as HTMLInputElement;
  const hotkeyChange = document.getElementById("hotkey-change") as HTMLButtonElement;
  const hotkeyReset = document.getElementById("hotkey-reset") as HTMLButtonElement;
  const hotkeyMsg = document.getElementById("hotkey-msg") as HTMLDivElement;
  const autostart = document.getElementById("autostart") as HTMLInputElement;
  const autostartLabel = document.getElementById("autostart-label") as HTMLSpanElement;
  const autostartNote = document.getElementById("autostart-note") as HTMLDivElement;
  if (navigator.userAgent.includes("Mac")) {
    autostartLabel.textContent = "로그인 시 자동 실행";
  }
  invoke<boolean>("is_release")
    .then((release) => {
      autostart.disabled = !release;
      autostartNote.hidden = release;
    })
    .catch(() => {});

  let current: Settings = {
    provider: "cli",
    web_search: false,
    hotkey: DEFAULT_HOTKEY,
    capture_selection: true,
    default_model: "haiku",
    autostart: true,
  };

  function showHotkeyMessage(msg: string) {
    hotkeyMsg.textContent = msg;
    hotkeyMsg.hidden = msg === "";
  }

  function stopCapture() {
    capturingHotkey = false;
    window.removeEventListener("keydown", onCaptureKey, true);
    hotkeyField.value = friendlyHotkey(current.hotkey);
  }

  async function applyHotkey(acc: string) {
    showHotkeyMessage("");
    try {
      await invoke("set_hotkey", { accelerator: acc });
      current = { ...current, hotkey: acc };
    } catch (e) {
      showHotkeyMessage(String(e));
    }
    hotkeyField.value = friendlyHotkey(current.hotkey);
  }

  function onCaptureKey(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      stopCapture();
      return;
    }
    if (["Control", "Shift", "Alt", "Meta", "AltGraph"].includes(e.key)) return;
    if (!(e.ctrlKey || e.altKey || e.shiftKey || e.metaKey)) return;
    const mods: string[] = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    const acc = [...mods, e.code].join("+");
    stopCapture();
    applyHotkey(acc);
  }

  function reflect() {
    radios.forEach((r) => {
      r.checked = r.value === current.provider;
    });
    webSearch.checked = current.web_search;
    captureSel.checked = current.capture_selection;
    autostart.checked = current.autostart ?? true;
    modelRadios.forEach((r) => {
      r.checked = r.value === current.default_model;
    });
    hotkeyField.value = friendlyHotkey(current.hotkey);
  }

  async function refreshKeyStatus() {
    const key = await invoke<string | null>("get_api_key");
    keyStatus.textContent = key ? "저장됨" : "";
    keyDelete.hidden = !key;
  }

  async function save() {
    await invoke("set_settings", { settings: current });
    onChange(current);
  }

  async function open() {
    if (capturingHotkey) stopCapture();
    showHotkeyMessage("");
    current = await loadSettings();
    reflect();
    keyInput.value = "";
    await refreshKeyStatus();
    panel.hidden = false;
    results.hidden = true;
  }

  function close(silent = false) {
    if (capturingHotkey) stopCapture();
    panel.hidden = true;
    results.hidden = false;
    if (!silent) onClose?.();
  }

  radios.forEach((r) => {
    r.addEventListener("change", () => {
      if (!r.checked) return;
      current = { ...current, provider: r.value === "api" ? "api" : "cli" };
      reflect();
      save();
    });
  });

  webSearch.addEventListener("change", () => {
    current = { ...current, web_search: webSearch.checked };
    save()
      .then(() => invoke("reset_cli_sessions"))
      .then(() => onCliReset?.())
      .catch(() => {});
  });

  modelRadios.forEach((r) => {
    r.addEventListener("change", () => {
      if (!r.checked) return;
      const v = r.value === "sonnet" || r.value === "opus" ? r.value : "haiku";
      current = { ...current, default_model: v };
      save();
    });
  });

  captureSel.addEventListener("change", () => {
    current = { ...current, capture_selection: captureSel.checked };
    save();
  });

  autostart.addEventListener("change", () => {
    current = { ...current, autostart: autostart.checked };
    save();
  });

  keySave.addEventListener("click", async () => {
    const key = keyInput.value.trim();
    if (key === "") return;
    await invoke("set_api_key", { key });
    keyInput.value = "";
    await refreshKeyStatus();
  });

  keyDelete.addEventListener("click", async () => {
    if (!window.confirm("저장된 API 키를 삭제할까요?")) return;
    await invoke("set_api_key", { key: "" });
    keyInput.value = "";
    await refreshKeyStatus();
  });

  hotkeyChange.addEventListener("click", () => {
    if (capturingHotkey) return;
    showHotkeyMessage("");
    capturingHotkey = true;
    hotkeyField.value = "새 단축키를 누르세요…";
    window.addEventListener("keydown", onCaptureKey, true);
  });

  hotkeyReset.addEventListener("click", () => {
    if (capturingHotkey) stopCapture();
    applyHotkey(DEFAULT_HOTKEY);
  });

  gear.addEventListener("click", () => {
    if (panel.hidden) open();
    else close();
  });

  return {
    toggle: () => (panel.hidden ? open() : close()),
    close,
    isOpen: () => !panel.hidden,
    open,
    showHotkeyMessage,
  };
}
