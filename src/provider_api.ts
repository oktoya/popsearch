import Anthropic from "@anthropic-ai/sdk";
import { invoke } from "@tauri-apps/api/core";
import type { Attachment } from "./images";

export interface ApiHandlers {
  onChunk: (text: string) => void;
  onDone: () => void;
  onError: (message: string) => void;
}

export type ModelKey = "haiku" | "sonnet" | "opus";

const MODEL_IDS: Record<ModelKey, string> = {
  haiku: "claude-haiku-5-5",
  sonnet: "claude-sonnet-5-5",
  opus: "claude-opus-5-5",
};

let conversation: Anthropic.Beta.BetaMessageParam[] = [];
let systemPrompt: string | null = null;

let generation = 0;
let currentAbort: AbortController | null = null;

export function resetConversation() {
  conversation = [];
  generation++;
  if (currentAbort) {
    currentAbort.abort();
    currentAbort = null;
  }
}

async function getSystemPrompt(): Promise<string> {
  if (systemPrompt === null) {
    systemPrompt = await invoke<string>("get_system_prompt");
  }
  return systemPrompt;
}

function errorMessage(error: unknown): string {
  if (error instanceof Anthropic.AuthenticationError) {
    return "API 키가 올바르지 않습니다.";
  }
  if (error instanceof Anthropic.RateLimitError) {
    return "요청이 너무 많습니다. 잠시 후 다시 시도하세요.";
  }
  const message = error instanceof Error ? error.message : String(error);
  return `API 오류: ${message}`;
}

export async function askApi(
  text: string,
  images: Attachment[],
  webSearch: boolean,
  model: ModelKey,
  handlers: ApiHandlers,
) {
  const myGeneration = generation;
  try {
    const key = await invoke<string | null>("get_api_key");
    if (!key) {
      handlers.onError("API 키가 없습니다. 설정에서 입력하세요.");
      return;
    }
    const system = await getSystemPrompt();
    if (generation !== myGeneration) return;
    const client = new Anthropic({ apiKey: key, dangerouslyAllowBrowser: true });

    let content: string | Anthropic.Beta.BetaContentBlockParam[] = text;
    if (images.length > 0) {
      const blocks: Anthropic.Beta.BetaContentBlockParam[] = [];
      for (const att of images) {
        if (att.kind === "image") {
          blocks.push({
            type: "image",
            source: { type: "base64", media_type: att.mediaType, data: att.data },
          });
        }
      }
      for (const att of images) {
        if (att.kind === "pdf") {
          blocks.push({
            type: "document",
            source: { type: "base64", media_type: "application/pdf", data: att.data },
            title: att.name,
          });
        } else if (att.kind === "text") {
          blocks.push({
            type: "document",
            source: { type: "text", media_type: "text/plain", data: att.text },
            title: att.name,
          });
        }
      }
      if (text.trim() !== "") blocks.push({ type: "text", text });
      content = blocks;
    }
    const userMessage: Anthropic.Beta.BetaMessageParam = { role: "user", content };
    const messages = [...conversation, userMessage];
    const effort = model === "haiku" ? "low" : "medium";

    const controller = new AbortController();
    currentAbort = controller;
    let stream;
    if (model === "haiku") {
      const params: Anthropic.MessageStreamParams = {
        model: MODEL_IDS.haiku,
        max_tokens: 16000,
        output_config: { effort },
        system,
        messages: messages as Anthropic.MessageParam[],
      };
      if (webSearch) {
        params.tools = [{ type: "web_search_20250305", name: "web_search", max_uses: 5 }];
      }
      stream = client.messages.stream(params, { signal: controller.signal });
    } else {
      // Sonnet/Opus: beta endpoint with server-side refusal fallbacks.
      const params: Anthropic.Beta.Messages.MessageCreateParams = {
        model: MODEL_IDS[model],
        max_tokens: 16000,
        output_config: { effort },
        system,
        messages,
        betas: ["server-side-fallback-2026-07-01"],
        fallbacks: "default",
      };
      if (webSearch) {
        params.tools = [{ type: "web_search_20260209", name: "web_search", max_uses: 5 }];
      }
      stream = client.beta.messages.stream(params, { signal: controller.signal });
    }
    for await (const event of stream) {
      if (generation !== myGeneration) return;
      if (event.type === "content_block_delta" && event.delta.type === "text_delta") {
        handlers.onChunk(event.delta.text);
      }
    }
    const finalMessage = await stream.finalMessage();
    if (generation !== myGeneration) return;
    if (currentAbort === controller) currentAbort = null;

    if (finalMessage.stop_reason === "refusal") {
      handlers.onError("요청이 거부되었습니다.");
      return;
    }

    conversation.push(userMessage);
    conversation.push({
      role: "assistant",
      content: finalMessage.content as Anthropic.Beta.BetaContentBlockParam[],
    });

    if (finalMessage.stop_reason === "max_tokens") {
      handlers.onChunk("\n\n(응답이 길이 제한으로 잘렸습니다)");
    } else if (finalMessage.stop_reason === "pause_turn") {
      handlers.onChunk("\n\n(검색이 길어져 중단되었습니다)");
    }
    handlers.onDone();
  } catch (error) {
    if (generation !== myGeneration) return;
    handlers.onError(errorMessage(error));
  }
}
