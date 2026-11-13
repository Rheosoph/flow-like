import { create } from "zustand";

/** The composer's "+" menu lists the sources in this order. */
export const COMPOSER_ATTACH_KINDS = [
	"scan_document",
	"scan_text",
	"live_text",
	"barcode",
	"take_photo",
	"location",
	"storage",
	"page_snapshot",
] as const;

export type ComposerAttachKind = (typeof COMPOSER_ATTACH_KINDS)[number];

/** Sources whose text alone is useful, so they show in chats that take no files. */
export const COMPOSER_TEXT_KINDS: ReadonlySet<ComposerAttachKind> = new Set([
	"scan_text",
	"live_text",
	"barcode",
	"location",
]);

/** What a source adds to the draft: files go to the attachments, text into the message box. */
export interface ComposerAttachResult {
	readonly files?: readonly File[];
	readonly text?: string;
}

export interface ComposerAttachSource {
	readonly kind: ComposerAttachKind;
	/** Runs the capture. Resolves an empty result when the person cancels; rejects on failure. */
	readonly attach: () => Promise<ComposerAttachResult>;
}

interface ComposerCaptureState {
	readonly sources: readonly ComposerAttachSource[];
	/** Adds device sources for every composer and returns the function that removes them again. */
	readonly register: (sources: readonly ComposerAttachSource[]) => () => void;
}

/**
 * Device capture the host app offers to every chat composer (scanner, camera, location). The
 * desktop shell registers only what the device has; web registers nothing. Sources that depend on
 * one chat's context, like its app storage, go through the ChatBox `attachSources` prop instead.
 */
export const useComposerCaptureStore = create<ComposerCaptureState>((set) => ({
	sources: [],
	register: (sources) => {
		set((state) => ({ sources: [...state.sources, ...sources] }));
		return () =>
			set((state) => ({
				sources: state.sources.filter((source) => !sources.includes(source)),
			}));
	},
}));
