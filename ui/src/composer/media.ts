// A file the person adds: stored through the platform's file route, with the facts the browser knows of it (its size, type,
// and for a picture or a video its dimensions and length) so the rules can be checked before the post is saved.

import { api } from "@ferrum/client";
import type { MediaState } from "../draft";

/** What the browser can say of a file without a server: width, height, seconds. Anything it cannot read is left out (the server checks only what it is told). */
export async function factsOf(file: File): Promise<Pick<MediaState, "width" | "height" | "seconds">> {
  try {
    if (file.type.startsWith("image/") && typeof createImageBitmap === "function") {
      const bitmap = await createImageBitmap(file);
      const out = { width: bitmap.width, height: bitmap.height };
      bitmap.close?.();
      return out;
    }
    if (file.type.startsWith("video/") && typeof document !== "undefined" && typeof URL.createObjectURL === "function") {
      const url = URL.createObjectURL(file);
      try {
        return await new Promise((resolve) => {
          const v = document.createElement("video");
          const done = (r: Pick<MediaState, "width" | "height" | "seconds">) => resolve(r);
          v.preload = "metadata";
          v.onloadedmetadata = () => done({ width: v.videoWidth || undefined, height: v.videoHeight || undefined, seconds: Number.isFinite(v.duration) ? v.duration : undefined });
          v.onerror = () => done({});
          setTimeout(() => done({}), 4000);
          v.src = url;
        });
      } finally {
        URL.revokeObjectURL(url);
      }
    }
  } catch {
    // an unreadable file is simply not measured
  }
  return {};
}

/** Stores a thumbnail picture and gives its address. */
export async function uploadThumbnail(file: File): Promise<string> {
  const form = new FormData();
  form.append("file", file);
  return (await api.post<{ url: string }>("/api/files", form)).url;
}
