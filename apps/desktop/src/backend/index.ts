import { isTauri } from "@tauri-apps/api/core";
import { mockBackend } from "./mock";
import { tauriBackend } from "./tauri";
import type { Backend } from "./types";

export * from "./rendered";
export * from "./types";

/** The backend for this runtime: the Rust shell inside Tauri, the mock elsewhere. */
export const backend: Backend = isTauri() ? tauriBackend : mockBackend;
