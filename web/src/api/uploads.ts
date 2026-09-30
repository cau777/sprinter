import type { ChatAttachment } from "./chats";
import { ApiError, type ApiErrorBody } from "./client";
import { PDF_TEXT_EXTRACTOR, type ExtractedPdfText } from "../pdf/extractText";

export type UploadRecord = Pick<ChatAttachment, "upload_id" | "filename" | "kind" | "mime" | "size" | "text_chars" | "text_pages" | "text_empty_pages"> & {
  text: PdfTextStats | null;
};
export type PdfTextStats = { chars: number; pages: number; empty_pages: number };

const isGif = (file: File) => file.type === "image/gif" || /\.gif$/i.test(file.name);
const isImage = (file: File) => file.type.startsWith("image/") || /\.(png|jpe?g|webp|gif)$/i.test(file.name);

export function prepareImage(file: File): Promise<File> {
  if (isGif(file)) return Promise.resolve(file);
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const image = new Image();
    image.onload = () => {
      URL.revokeObjectURL(url);
      const scale = Math.min(1, 2048 / Math.max(image.naturalWidth, image.naturalHeight));
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
      canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
      const context = canvas.getContext("2d");
      if (!context) { reject(new Error("Your browser could not process this image.")); return; }
      context.drawImage(image, 0, 0, canvas.width, canvas.height);
      canvas.toBlob((blob) => {
        if (!blob) { reject(new Error("Your browser could not process this image.")); return; }
        const stem = file.name.replace(/\.[^.]+$/, "") || "image";
        resolve(new File([blob], `${stem}.webp`, { type: "image/webp", lastModified: file.lastModified }));
      }, "image/webp", 0.85);
    };
    image.onerror = () => { URL.revokeObjectURL(url); reject(new Error(`Could not read ${file.name}.`)); };
    image.src = url;
  });
}

export async function uploadFile(file: File, onProgress: (progress: number) => void): Promise<UploadRecord> {
  const prepared = isImage(file) ? await prepareImage(file) : file;
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", "/api/uploads");
    request.withCredentials = true;
    request.setRequestHeader("X-Sprinter", "1");
    request.setRequestHeader("X-Filename", encodeURIComponent(prepared.name));
    request.setRequestHeader("Content-Type", "application/octet-stream");
    request.upload.onprogress = (event) => { if (event.lengthComputable) onProgress(Math.round(event.loaded / event.total * 100)); };
    request.onerror = () => reject(new Error(`Could not upload ${prepared.name}. Check your connection and try again.`));
    request.onload = () => {
      if (request.status < 200 || request.status >= 300) {
        try {
          const body = JSON.parse(request.responseText) as { error?: { message?: string; code?: string; request_id?: string } };
          reject(new ApiError(request.status, body));
        } catch { reject(new Error(`Upload failed (${request.status}).`)); }
        return;
      }
      try {
        const result = JSON.parse(request.responseText) as { id: string; filename: string; kind: string; mime: string; size: number; text: PdfTextStats | null };
        resolve({
          upload_id: result.id,
          filename: result.filename,
          kind: result.kind,
          mime: result.mime,
          size: result.size,
          text: result.text,
          text_chars: result.text?.chars ?? null,
          text_pages: result.text?.pages ?? null,
          text_empty_pages: result.text?.empty_pages ?? null,
        });
      } catch { reject(new Error("The server returned an invalid upload response.")); }
    };
    request.send(prepared);
  });
}

export async function storePdfText(uploadId: string, extracted: ExtractedPdfText): Promise<PdfTextStats> {
  const response = await fetch(`/api/uploads/${encodeURIComponent(uploadId)}/text`, {
    method: "PUT",
    credentials: "same-origin",
    headers: {
      "X-Sprinter": "1",
      "X-Pdf-Pages": String(extracted.pages),
      "X-Pdf-Empty-Pages": String(extracted.empty_pages),
      "X-Pdf-Extractor": PDF_TEXT_EXTRACTOR,
      "Content-Type": "text/plain; charset=utf-8",
    },
    body: extracted.text,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => ({})) as ApiErrorBody;
    throw new ApiError(response.status, body);
  }
  return await response.json() as PdfTextStats;
}

export async function fetchPdfFile(uploadId: string, filename: string): Promise<File> {
  const response = await fetch(`/api/uploads/${encodeURIComponent(uploadId)}`, { credentials: "same-origin" });
  if (!response.ok) {
    const body = await response.json().catch(() => ({})) as ApiErrorBody;
    throw new ApiError(response.status, body);
  }
  return new File([await response.blob()], filename, { type: "application/pdf" });
}

export async function deleteUpload(id: string) {
  const response = await fetch(`/api/uploads/${encodeURIComponent(id)}`, { method: "DELETE", credentials: "same-origin", headers: { "X-Sprinter": "1" } });
  if (!response.ok && response.status !== 404) throw new Error("Could not remove the uploaded file.");
}
