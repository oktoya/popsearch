import { invoke } from "@tauri-apps/api/core";

export type MediaType = "image/png" | "image/jpeg" | "image/gif" | "image/webp";

export type Attachment =
  | { kind: "image"; mediaType: MediaType; data: string; thumb: string; name: string }
  | { kind: "pdf"; data: string; name: string }
  | { kind: "text"; text: string; name: string };

const MAX_PDF_BYTES = 25 * 1024 * 1024;
const MAX_TEXT_BYTES = 2 * 1024 * 1024;
const MAX_OFFICE_BYTES = 25 * 1024 * 1024;
const OFFICE_EXTS = [".docx", ".xlsx", ".pptx", ".hwpx"];

export const MAX_ATTACHMENTS = 5;
const MAX_SIDE = 1568;
const THUMB_SIDE = 96;
const TYPES: string[] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

function loadImage(blob: Blob): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(blob);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve(img);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("image load failed"));
    };
    img.src = url;
  });
}

function drawScaled(img: HTMLImageElement, scale: number): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
  const ctx = canvas.getContext("2d");
  if (ctx) ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
  return canvas;
}

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = String(reader.result);
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

export async function rawImageToAttachment(raw: {
  media_type: string;
  data: string;
}): Promise<Attachment | null> {
  const bin = atob(raw.data);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  if (TYPES.includes(raw.media_type)) {
    const ext = raw.media_type === "image/jpeg" ? "jpg" : raw.media_type.slice(6);
    return imageToAttachment(
      new File([bytes], `selection.${ext}`, { type: raw.media_type }),
    );
  }
  if (raw.media_type === "image/bmp") {
    const img = await loadImage(new Blob([bytes], { type: "image/bmp" }));
    const canvas = drawScaled(img, 1);
    const blob: Blob | null = await new Promise((resolve) =>
      canvas.toBlob(resolve, "image/png"),
    );
    if (!blob) return null;
    return imageToAttachment(new File([blob], "selection.png", { type: "image/png" }));
  }
  return null;
}

async function imageToAttachment(file: File): Promise<Attachment | null> {
  if (!TYPES.includes(file.type)) return null;
  let mediaType = file.type as MediaType;
  const img = await loadImage(file);
  const longer = Math.max(img.naturalWidth, img.naturalHeight);

  let data: string;
  if (longer > MAX_SIDE) {
    const canvas = drawScaled(img, MAX_SIDE / longer);
    if (mediaType === "image/png") {
      data = canvas.toDataURL("image/png");
    } else {
      mediaType = "image/jpeg";
      data = canvas.toDataURL("image/jpeg", 0.9);
    }
    data = data.slice(data.indexOf(",") + 1);
  } else {
    data = await blobToBase64(file);
  }

  if (data.length > 5_000_000) {
    let scale = Math.min(1, MAX_SIDE / longer);
    mediaType = "image/jpeg";
    do {
      data = drawScaled(img, scale).toDataURL("image/jpeg", 0.85);
      data = data.slice(data.indexOf(",") + 1);
      scale /= 2;
    } while (data.length > 5_000_000 && scale * longer >= 1);
  }

  const thumbScale = Math.min(1, THUMB_SIDE / longer);
  const thumb = drawScaled(img, thumbScale).toDataURL("image/jpeg", 0.8);
  return { kind: "image", mediaType, data, thumb, name: file.name };
}

export async function fileToAttachment(
  file: File,
): Promise<{ att: Attachment } | { error: string }> {
  if (TYPES.includes(file.type)) {
    const att = await imageToAttachment(file);
    return att ? { att } : { error: `지원하지 않는 형식입니다: ${file.name}` };
  }
  if (file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf")) {
    if (file.size > MAX_PDF_BYTES) {
      return { error: `PDF가 너무 큽니다 (최대 25MB): ${file.name}` };
    }
    return { att: { kind: "pdf", data: await blobToBase64(file), name: file.name } };
  }
  if (OFFICE_EXTS.some((ext) => file.name.toLowerCase().endsWith(ext))) {
    if (file.size > MAX_OFFICE_BYTES) {
      return { error: `문서가 너무 큽니다 (최대 25MB): ${file.name}` };
    }
    try {
      const text = await invoke<string>("extract_office", {
        name: file.name,
        data: await blobToBase64(file),
      });
      if (text.length > MAX_TEXT_BYTES) {
        return { error: `문서 내용이 너무 깁니다 (최대 2MB): ${file.name}` };
      }
      return { att: { kind: "text", text, name: file.name } };
    } catch (e) {
      return { error: String(e) };
    }
  }
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(await file.arrayBuffer());
  } catch (_) {
    return { error: `지원하지 않는 형식입니다: ${file.name}` };
  }
  if (file.size > MAX_TEXT_BYTES) {
    return { error: `텍스트 파일이 너무 큽니다 (최대 2MB): ${file.name}` };
  }
  return { att: { kind: "text", text, name: file.name } };
}
