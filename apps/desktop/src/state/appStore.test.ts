import { beforeEach, describe, expect, it } from "vitest";
import { selectActiveDocument, useAppStore } from "./appStore";

const doc = (id: number, pageCount = 5) => ({ id, name: `doc-${id}.pdf`, pageCount });

beforeEach(() => {
  useAppStore.setState({ documents: [], activeDocumentId: null, errorCode: null });
});

describe("app store documents", () => {
  it("activates a newly opened document and clears a stale error", () => {
    useAppStore.getState().setError("invalid_pdf");
    useAppStore.getState().addDocument(doc(1));
    const state = useAppStore.getState();
    expect(selectActiveDocument(state)?.name).toBe("doc-1.pdf");
    expect(state.errorCode).toBeNull();
  });

  it("moves focus to a neighbour when the active document closes", () => {
    const store = useAppStore.getState();
    store.addDocument(doc(1));
    store.addDocument(doc(2));
    store.addDocument(doc(3));
    store.activateDocument(2);
    store.removeDocument(2);
    expect(useAppStore.getState().activeDocumentId).toBe(3);
    store.removeDocument(3);
    expect(useAppStore.getState().activeDocumentId).toBe(1);
    store.removeDocument(1);
    expect(useAppStore.getState().activeDocumentId).toBeNull();
  });

  it("keeps the page index and zoom inside their valid ranges", () => {
    const store = useAppStore.getState();
    store.addDocument(doc(1, 3));
    store.updateDocument(1, { page: 99, zoom: 500 });
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({ page: 2, zoom: 8 });
    store.updateDocument(1, { page: -4, zoom: 0 });
    expect(selectActiveDocument(useAppStore.getState())).toMatchObject({ page: 0, zoom: 0.1 });
  });
});
