import type { ViewerController } from "./controller";

/**
 * The mounted viewer of each open document, so registry commands (next page,
 * go to page) and the self-test can drive it. A document whose tab is not
 * shown has no viewer; commands then only update the store.
 */
const viewers = new Map<number, ViewerController>();

export function registerViewer(documentId: number, viewer: ViewerController): void {
  viewers.set(documentId, viewer);
}

export function unregisterViewer(documentId: number, viewer: ViewerController): void {
  if (viewers.get(documentId) === viewer) viewers.delete(documentId);
}

export function getViewer(documentId: number): ViewerController | undefined {
  return viewers.get(documentId);
}
