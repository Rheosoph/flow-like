import {
	type FormEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";
import {
	type InlineAnnotationOpenRequest,
	installInlineAnnotations,
} from "../lib/inline-annotations";
import {
	type ReaderChapter,
	type ReadingBookmark,
	type ReadingComment,
	type ReadingProgressRecord,
	bookmarkRecordId,
	clampProgress,
	mergeReadingProgress,
	normalizeReadingPath,
	passageBookmarkRecordId,
	progressRecordId,
	summarizeReadingProgress,
} from "../lib/reading-progress";
import {
	type ReadingData,
	deleteReadingBookmark,
	deleteReadingComment,
	getReadingData,
	saveReadingBookmark,
	saveReadingComment,
	saveReadingProgress,
} from "../lib/reading-store";
import "./ReadingExperience.css";

interface ReadingExperienceProps {
	editionId: string;
	chapters: ReaderChapter[];
	currentEntryId: string;
	currentPath: string;
	currentTitle: string;
	isReadingPage: boolean;
	isLandingPage: boolean;
}

interface DomReadingLocation {
	percent: number;
	scrollY: number;
	headingId: string;
	headingText: string;
	headingOffset: number;
	sectionProgress?: number;
}

interface ReadingSelection {
	quote: string;
	location: DomReadingLocation;
	left: number;
	top: number;
}

type ReaderTab = "overview" | "bookmarks" | "comments";

const EMPTY_DATA: ReadingData = {
	progress: [],
	bookmarks: [],
	comments: [],
};

const SAVE_DELAY_MS = 650;

function mergeRecordsById<T extends { id: string }>(
	preferred: readonly T[],
	fallback: readonly T[],
): T[] {
	const preferredIds = new Set(preferred.map((record) => record.id));
	return [
		...preferred,
		...fallback.filter((record) => !preferredIds.has(record.id)),
	];
}

function percentage(value: number): string {
	return `${Math.round(clampProgress(value) * 100)}%`;
}

function absoluteTop(element: HTMLElement): number {
	return element.getBoundingClientRect().top + window.scrollY;
}

function readDomLocation(
	anchorPosition?: number,
): DomReadingLocation | undefined {
	const main = document.querySelector<HTMLElement>("main[data-pagefind-body]");
	const content = main?.querySelector<HTMLElement>(".sl-markdown-content");
	if (!main || !content) return undefined;

	const headingElements = Array.from(
		main.querySelectorAll<HTMLElement>(
			"h1#_top, .sl-markdown-content h2[id], .sl-markdown-content h3[id]",
		),
	);
	const marker =
		anchorPosition ?? window.scrollY + Math.min(window.innerHeight * 0.32, 260);
	let activeHeading = headingElements[0];
	for (const heading of headingElements) {
		if (absoluteTop(heading) <= marker) activeHeading = heading;
		else break;
	}

	const startElement = headingElements[0] ?? content;
	const start = absoluteTop(startElement);
	const end = absoluteTop(content) + content.scrollHeight;
	const activeHeadingIndex = activeHeading
		? headingElements.indexOf(activeHeading)
		: -1;
	const nextHeading =
		activeHeadingIndex >= 0
			? headingElements[activeHeadingIndex + 1]
			: undefined;
	const headingTop = activeHeading ? absoluteTop(activeHeading) : start;
	const sectionEnd = nextHeading ? absoluteTop(nextHeading) : end;
	const readingDistance = Math.max(1, end - start - window.innerHeight * 0.72);
	let percent = clampProgress((window.scrollY - start) / readingDistance);
	if (content.getBoundingClientRect().bottom <= window.innerHeight * 0.92) {
		percent = 1;
	}

	return {
		percent,
		scrollY: Math.max(0, window.scrollY),
		headingId: activeHeading?.id || "_top",
		headingText:
			activeHeading?.textContent?.replace(/\s+/g, " ").trim() ||
			"Chapter opening",
		headingOffset: activeHeading
			? window.scrollY - absoluteTop(activeHeading)
			: window.scrollY,
		sectionProgress: clampProgress(
			(marker - headingTop) / Math.max(1, sectionEnd - headingTop),
		),
	};
}

async function waitForStableLayout(): Promise<void> {
	if (document.readyState !== "complete") {
		await new Promise<void>((resolve) => {
			window.addEventListener("load", () => resolve(), { once: true });
		});
	}
	try {
		await document.fonts?.ready;
	} catch {
		// Font loading must never block restoring a saved location.
	}
	await new Promise<void>((resolve) => {
		requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
	});
}

async function restoreLocation(
	location: Pick<
		DomReadingLocation,
		"scrollY" | "headingId" | "headingOffset" | "sectionProgress" | "percent"
	>,
	focusTarget = false,
): Promise<void> {
	await waitForStableLayout();
	const main = document.querySelector<HTMLElement>("main[data-pagefind-body]");
	const content = main?.querySelector<HTMLElement>(".sl-markdown-content");
	const headingElements = main
		? Array.from(
				main.querySelectorAll<HTMLElement>(
					"h1#_top, .sl-markdown-content h2[id], .sl-markdown-content h3[id]",
				),
			)
		: [];
	const heading = headingElements.find(
		(candidate) => candidate.id === location.headingId,
	);
	const firstHeading = headingElements[0];
	const start = firstHeading ? absoluteTop(firstHeading) : 0;
	const readingDistance = content
		? Math.max(
				1,
				absoluteTop(content) +
					content.scrollHeight -
					start -
					window.innerHeight * 0.72,
			)
		: 0;
	const percentTarget = readingDistance
		? start + clampProgress(location.percent) * readingDistance
		: location.scrollY;
	let target = percentTarget;
	if (heading && content) {
		const headingIndex = headingElements.indexOf(heading);
		const nextHeading = headingElements[headingIndex + 1];
		const headingTop = absoluteTop(heading);
		const sectionEnd = nextHeading
			? absoluteTop(nextHeading)
			: absoluteTop(content) + content.scrollHeight;
		const markerOffset = Math.min(window.innerHeight * 0.32, 260);
		if (
			typeof location.sectionProgress === "number" &&
			Number.isFinite(location.sectionProgress)
		) {
			target =
				headingTop +
				clampProgress(location.sectionProgress) * (sectionEnd - headingTop) -
				markerOffset;
		} else {
			// Older records only have a page percentage and a pixel offset. Keep
			// them inside the same semantic section without trusting stale pixels.
			target = Math.min(
				sectionEnd - markerOffset,
				Math.max(headingTop - markerOffset, percentTarget),
			);
		}
	}
	const maxScroll = Math.max(
		0,
		document.documentElement.scrollHeight - window.innerHeight,
	);
	window.scrollTo({
		top: Math.min(maxScroll, Math.max(0, target)),
		behavior: "auto",
	});
	if (focusTarget) {
		const targetElement = heading ?? firstHeading ?? main;
		if (targetElement) {
			if (!targetElement.hasAttribute("tabindex")) {
				targetElement.setAttribute("tabindex", "-1");
			}
			targetElement.focus({ preventScroll: true });
		}
	}
}

function locationHref(
	location: { path: string; id?: string; headingId?: string },
	mode: "resume" | "location",
): string {
	const params = new URLSearchParams();
	if (mode === "resume") params.set("reader-resume", "1");
	if (mode === "location" && location.id) {
		params.set("reader-location", location.id);
	}
	const hash = location.headingId
		? `#${encodeURIComponent(location.headingId)}`
		: "";
	return `${normalizeReadingPath(location.path)}?${params.toString()}${hash}`;
}

function formatSavedDate(value: string): string {
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return "Saved locally";
	return new Intl.DateTimeFormat(undefined, {
		month: "short",
		day: "numeric",
	}).format(date);
}

function readReadingSelection(): ReadingSelection | undefined {
	const selection = window.getSelection();
	if (!selection || selection.isCollapsed || selection.rangeCount === 0)
		return undefined;
	const range = selection.getRangeAt(0);
	const main = document.querySelector<HTMLElement>("main[data-pagefind-body]");
	const content = main?.querySelector<HTMLElement>(".sl-markdown-content");
	if (
		!content ||
		!content.contains(range.startContainer) ||
		!content.contains(range.endContainer)
	) {
		return undefined;
	}
	const text = selection.toString().replace(/\s+/g, " ").trim();
	if (!text) return undefined;

	const rect = range.getBoundingClientRect();
	const headerBottom = Math.max(
		0,
		document.querySelector<HTMLElement>(".header")?.getBoundingClientRect()
			.bottom ?? 0,
		document
			.querySelector<HTMLElement>("mobile-starlight-toc")
			?.getBoundingClientRect().bottom ?? 0,
	);
	if (rect.bottom < headerBottom || rect.top > window.innerHeight) {
		return undefined;
	}
	const location = readDomLocation(
		window.scrollY + rect.top + Math.max(0, rect.height / 2),
	);
	if (!location) return undefined;

	const halfToolbarWidth = Math.min(
		130,
		Math.max(82, (window.innerWidth - 20) / 2),
	);
	const left = Math.min(
		window.innerWidth - halfToolbarWidth - 10,
		Math.max(halfToolbarWidth + 10, rect.left + rect.width / 2),
	);
	const below = rect.bottom + 10;
	const minimumTop = Math.max(10, headerBottom + 10);
	const top = Math.max(
		minimumTop,
		below + 48 <= window.innerHeight
			? below
			: Math.max(minimumTop, rect.top - 48),
	);

	return {
		quote: text.slice(0, 360),
		location,
		left,
		top,
	};
}

function createCommentId(): string {
	if (
		typeof crypto !== "undefined" &&
		typeof crypto.randomUUID === "function"
	) {
		return crypto.randomUUID();
	}
	return `comment-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

export default function ReadingExperience({
	editionId,
	chapters,
	currentEntryId,
	currentPath,
	currentTitle,
	isReadingPage,
	isLandingPage,
}: ReadingExperienceProps) {
	const [readerData, setReaderData] = useState<ReadingData>(EMPTY_DATA);
	const [activeTab, setActiveTab] = useState<ReaderTab | null>(null);
	const [draftComment, setDraftComment] = useState("");
	const [draftQuote, setDraftQuote] = useState<string | undefined>();
	const [draftLocation, setDraftLocation] = useState<
		DomReadingLocation | undefined
	>();
	const [readingSelection, setReadingSelection] = useState<
		ReadingSelection | undefined
	>();
	const [liveLocation, setLiveLocation] = useState<
		DomReadingLocation | undefined
	>();
	const [portalHost, setPortalHost] = useState<HTMLElement | null>(null);
	const [editingCommentId, setEditingCommentId] = useState<
		string | undefined
	>();
	const [resumeTarget, setResumeTarget] = useState<
		ReadingProgressRecord | undefined
	>();
	const [resumeCardDismissed, setResumeCardDismissed] = useState(false);
	const [storageAvailable, setStorageAvailable] = useState(true);
	const [announcement, setAnnouncement] = useState("");
	const [activeAnnotationId, setActiveAnnotationId] = useState<
		string | undefined
	>();
	const readerDataRef = useRef(readerData);
	const panelRef = useRef<HTMLDialogElement>(null);
	const closeButtonRef = useRef<HTMLButtonElement>(null);
	const panelTriggerRef = useRef<HTMLElement | null>(null);
	const awaitingResumeRef = useRef(false);
	const storageWritableRef = useRef(true);
	const progressFlushRef = useRef<(() => Promise<void>) | undefined>(undefined);
	const annotationWritesRef = useRef<Set<Promise<void>>>(new Set());
	const navigationStartedRef = useRef(false);
	const readingSelectionRef = useRef<ReadingSelection | undefined>(undefined);
	const liveLocationRef = useRef<DomReadingLocation | undefined>(undefined);
	const announcementTimerRef = useRef<
		ReturnType<typeof setTimeout> | undefined
	>(undefined);

	const updateReaderData = useCallback(
		(updater: (current: ReadingData) => ReadingData) => {
			setReaderData((current) => {
				const next = updater(current);
				readerDataRef.current = next;
				return next;
			});
		},
		[],
	);

	const updateReadingSelection = useCallback(
		(next: ReadingSelection | undefined) => {
			readingSelectionRef.current = next;
			setReadingSelection(next);
		},
		[],
	);
	const clearReadingSelection = useCallback(() => {
		window.getSelection()?.removeAllRanges();
		updateReadingSelection(undefined);
	}, [updateReadingSelection]);

	useEffect(() => {
		setPortalHost(document.body);
	}, []);

	useEffect(() => {
		if (!isReadingPage) return;
		let frame = 0;

		const captureSelection = (clearWhenEmpty: boolean) => {
			if (frame) cancelAnimationFrame(frame);
			frame = requestAnimationFrame(() => {
				frame = 0;
				const next = readReadingSelection();
				if (next || clearWhenEmpty) updateReadingSelection(next);
			});
		};
		const handleSelectionChange = () => captureSelection(false);
		const handlePointerUp = (event: PointerEvent) => {
			const target = event.target;
			const main = document.querySelector("main[data-pagefind-body]");
			if (
				target instanceof Element &&
				target.closest(
					".flowbook-inline-annotations, .flowbook-selection-tools, .flowbook-reader-dock, .flowbook-reader-panel",
				)
			) {
				return;
			}
			if (target instanceof Node && main?.contains(target)) {
				captureSelection(true);
				return;
			}
			updateReadingSelection(undefined);
		};
		const handleKeyUp = (event: KeyboardEvent) => {
			const target = event.target;
			if (
				target instanceof Element &&
				target.closest(
					".flowbook-inline-annotations, .flowbook-selection-tools, .flowbook-reader-dock, .flowbook-reader-panel",
				)
			) {
				return;
			}
			captureSelection(true);
		};
		const repositionSelectionTools = () => captureSelection(true);
		const preserveSelectionThroughResize = () => captureSelection(false);

		document.addEventListener("selectionchange", handleSelectionChange);
		document.addEventListener("pointerup", handlePointerUp);
		document.addEventListener("keyup", handleKeyUp);
		window.addEventListener("scroll", repositionSelectionTools, {
			passive: true,
		});
		window.addEventListener("resize", preserveSelectionThroughResize, {
			passive: true,
		});
		return () => {
			document.removeEventListener("selectionchange", handleSelectionChange);
			document.removeEventListener("pointerup", handlePointerUp);
			document.removeEventListener("keyup", handleKeyUp);
			window.removeEventListener("scroll", repositionSelectionTools);
			window.removeEventListener("resize", preserveSelectionThroughResize);
			if (frame) cancelAnimationFrame(frame);
		};
	}, [isReadingPage, updateReadingSelection]);

	useEffect(
		() => () => {
			if (announcementTimerRef.current) {
				clearTimeout(announcementTimerRef.current);
			}
		},
		[],
	);

	const trackAnnotationWrite = useCallback(
		(write: Promise<void>): Promise<void> => {
			const settled = write.then(
				() => undefined,
				() => undefined,
			);
			annotationWritesRef.current.add(settled);
			void settled.then(() => annotationWritesRef.current.delete(settled));
			return write;
		},
		[],
	);

	const drainWrites = useCallback(async (): Promise<void> => {
		const writes = [...annotationWritesRef.current];
		const progressWrite = progressFlushRef.current?.();
		await Promise.all(progressWrite ? [progressWrite, ...writes] : writes);
	}, []);

	const navigateAfterSaving = useCallback(
		(href: string) => {
			if (navigationStartedRef.current) return;
			navigationStartedRef.current = true;
			const timeout = new Promise<void>((resolve) =>
				window.setTimeout(resolve, 350),
			);
			void Promise.race([drainWrites(), timeout]).finally(() => {
				window.location.assign(href);
			});
		},
		[drainWrites],
	);

	const currentChapter = useMemo(
		() => chapters.find((chapter) => chapter.entryId === currentEntryId),
		[chapters, currentEntryId],
	);
	const currentProgress = readerData.progress.find(
		(record) => record.entryId === currentEntryId,
	);
	const canonicalEntryIds = useMemo(
		() => new Set(chapters.map((chapter) => chapter.entryId)),
		[chapters],
	);
	const mostRecentProgress = readerData.progress.find((record) =>
		canonicalEntryIds.has(record.entryId),
	);
	const canonicalBookmarks = useMemo(
		() =>
			readerData.bookmarks.filter((bookmark) =>
				canonicalEntryIds.has(bookmark.entryId),
			),
		[canonicalEntryIds, readerData.bookmarks],
	);
	const canonicalComments = useMemo(
		() =>
			readerData.comments.filter((comment) =>
				canonicalEntryIds.has(comment.entryId),
			),
		[canonicalEntryIds, readerData.comments],
	);
	const currentEntryBookmarks = useMemo(
		() =>
			canonicalBookmarks.filter(
				(bookmark) => bookmark.entryId === currentEntryId,
			),
		[canonicalBookmarks, currentEntryId],
	);
	const currentEntryComments = useMemo(
		() =>
			canonicalComments.filter((comment) => comment.entryId === currentEntryId),
		[canonicalComments, currentEntryId],
	);
	const archivedAnnotationCount =
		readerData.bookmarks.length +
		readerData.comments.length -
		canonicalBookmarks.length -
		canonicalComments.length;
	const summary = useMemo(
		() => summarizeReadingProgress(chapters, readerData.progress),
		[chapters, readerData.progress],
	);
	const activeLocation = liveLocation ?? currentProgress;
	const currentBookmarkId = activeLocation
		? bookmarkRecordId(editionId, currentEntryId, activeLocation.headingId)
		: undefined;
	const currentBookmark = readerData.bookmarks.find(
		(bookmark) => bookmark.id === currentBookmarkId,
	);

	useEffect(() => {
		let disposed = false;
		let stopTracking = () => {};

		const initialize = async () => {
			let loaded = EMPTY_DATA;
			try {
				loaded = await getReadingData(editionId);
			} catch {
				storageWritableRef.current = false;
				if (!disposed) setStorageAvailable(false);
			}
			if (disposed) return;
			const optimistic = readerDataRef.current;
			loaded = {
				progress: mergeRecordsById(optimistic.progress, loaded.progress),
				bookmarks: mergeRecordsById(optimistic.bookmarks, loaded.bookmarks),
				comments: mergeRecordsById(optimistic.comments, loaded.comments),
			};
			readerDataRef.current = loaded;
			setReaderData(loaded);

			if (!isReadingPage || !currentChapter) return;

			let latest = loaded.progress.find(
				(record) => record.entryId === currentEntryId,
			);
			const params = new URLSearchParams(window.location.search);
			const requestedLocationId = params.get("reader-location");
			const requestedRecord = requestedLocationId
				? (loaded.bookmarks.find((item) => item.id === requestedLocationId) ??
					loaded.comments.find((item) => item.id === requestedLocationId))
				: undefined;
			const requestedLocation =
				requestedRecord?.entryId === currentEntryId
					? requestedRecord
					: undefined;
			const navigation = performance.getEntriesByType("navigation")[0] as
				| PerformanceNavigationTiming
				| undefined;
			const shouldRestore = Boolean(
				requestedLocation ||
					params.has("reader-resume") ||
					(navigation?.type === "reload" && !window.location.hash),
			);

			const restoreTarget = requestedLocation ?? latest;
			if (shouldRestore && restoreTarget) {
				await restoreLocation(
					restoreTarget,
					Boolean(requestedLocation || params.has("reader-resume")),
				);
				if (disposed) return;
				params.delete("reader-location");
				params.delete("reader-resume");
				const search = params.toString();
				let hash = window.location.hash;
				try {
					if (decodeURIComponent(hash.slice(1)) === restoreTarget.headingId) {
						hash = "";
					}
				} catch {
					// Preserve malformed or user-authored hashes instead of rewriting them.
				}
				history.replaceState(
					history.state,
					"",
					`${window.location.pathname}${search ? `?${search}` : ""}${hash}`,
				);
			} else if (
				latest &&
				latest.percent > 0.04 &&
				latest.percent < 0.98 &&
				window.scrollY < 80 &&
				!window.location.hash
			) {
				awaitingResumeRef.current = true;
				setActiveTab(null);
				setResumeTarget(latest);
			}

			let frame = 0;
			let saveTimer: ReturnType<typeof setTimeout> | undefined;
			let pending: ReadingProgressRecord | undefined;
			let writeChain = Promise.resolve();

			const persist = (): Promise<void> => {
				if (!pending || !storageWritableRef.current) return writeChain;
				const record = pending;
				pending = undefined;
				writeChain = writeChain.then(async () => {
					try {
						const stored = await saveReadingProgress(record);
						if (latest) {
							latest = {
								...latest,
								furthestPercent: Math.max(
									latest.furthestPercent,
									stored.furthestPercent,
								),
								completed: latest.completed || stored.completed,
								completedAt: latest.completedAt ?? stored.completedAt,
							};
						}
						if (!disposed) {
							updateReaderData((current) => ({
								...current,
								progress: current.progress.map((currentRecord) =>
									currentRecord.id === stored.id
										? {
												...currentRecord,
												furthestPercent: Math.max(
													currentRecord.furthestPercent,
													stored.furthestPercent,
												),
												completed: currentRecord.completed || stored.completed,
												completedAt:
													currentRecord.completedAt ?? stored.completedAt,
											}
										: currentRecord,
								),
							}));
						}
					} catch {
						if (!pending) pending = record;
						storageWritableRef.current = false;
						if (!disposed) setStorageAvailable(false);
					}
				});
				return writeChain;
			};

			const measure = (queueSave = true, updateUi = true) => {
				frame = 0;
				const location = readDomLocation();
				if (!location) return;
				liveLocationRef.current = location;
				const now = new Date().toISOString();
				const nextRecord = mergeReadingProgress(latest, {
					id: progressRecordId(editionId, currentEntryId),
					editionId,
					entryId: currentEntryId,
					path: normalizeReadingPath(currentPath),
					title: currentTitle,
					...location,
					updatedAt: now,
				});
				latest = nextRecord;
				pending = nextRecord;

				if (updateUi) {
					setLiveLocation(location);
					updateReaderData((current) => ({
						...current,
						progress: [
							nextRecord,
							...current.progress.filter(
								(record) => record.id !== nextRecord.id,
							),
						],
					}));
				}

				if (queueSave) {
					if (saveTimer) clearTimeout(saveTimer);
					saveTimer = setTimeout(() => void persist(), SAVE_DELAY_MS);
				}
			};

			const scheduleMeasure = () => {
				if (awaitingResumeRef.current) return;
				if (!frame) frame = requestAnimationFrame(() => measure());
			};
			const handleScroll = () => {
				if (awaitingResumeRef.current) {
					awaitingResumeRef.current = false;
					setResumeTarget(undefined);
				}
				scheduleMeasure();
			};
			const flushLatest = (updateUi = false): Promise<void> => {
				if (!awaitingResumeRef.current) {
					if (frame) cancelAnimationFrame(frame);
					frame = 0;
					if (saveTimer) clearTimeout(saveTimer);
					saveTimer = undefined;
					measure(false, updateUi);
				}
				return persist();
			};
			const flushWhenHidden = () => {
				if (document.visibilityState === "hidden") void flushLatest();
			};
			const flushOnPageHide = () => {
				void flushLatest();
			};
			const resetNavigationAfterHistoryRestore = () => {
				navigationStartedRef.current = false;
			};
			const flushProgress = () => flushLatest();
			progressFlushRef.current = flushProgress;
			const handleNavigationClick = (event: MouseEvent) => {
				if (
					navigationStartedRef.current ||
					event.defaultPrevented ||
					event.button !== 0 ||
					event.metaKey ||
					event.ctrlKey ||
					event.shiftKey ||
					event.altKey ||
					awaitingResumeRef.current ||
					!(event.target instanceof Element)
				) {
					return;
				}
				const anchor = event.target.closest<HTMLAnchorElement>("a[href]");
				if (
					!anchor ||
					anchor.download ||
					(anchor.target && anchor.target !== "_self")
				) {
					return;
				}
				const destination = new URL(anchor.href, window.location.href);
				const currentLocation = new URL(window.location.href);
				const isHttpNavigation =
					destination.protocol === "http:" || destination.protocol === "https:";
				const isHashOnlyNavigation =
					destination.origin === currentLocation.origin &&
					destination.pathname === currentLocation.pathname &&
					destination.search === currentLocation.search &&
					destination.hash !== currentLocation.hash;
				if (!isHttpNavigation || isHashOnlyNavigation) {
					return;
				}

				event.preventDefault();
				navigateAfterSaving(destination.href);
			};

			if (!awaitingResumeRef.current) measure();
			window.addEventListener("scroll", handleScroll, { passive: true });
			window.addEventListener("resize", scheduleMeasure, { passive: true });
			window.addEventListener("flowbook-reader:measure", scheduleMeasure);
			window.addEventListener("pagehide", flushOnPageHide);
			window.addEventListener("pageshow", resetNavigationAfterHistoryRestore);
			document.addEventListener("visibilitychange", flushWhenHidden);
			document.addEventListener("click", handleNavigationClick);

			stopTracking = () => {
				window.removeEventListener("scroll", handleScroll);
				window.removeEventListener("resize", scheduleMeasure);
				window.removeEventListener("flowbook-reader:measure", scheduleMeasure);
				window.removeEventListener("pagehide", flushOnPageHide);
				window.removeEventListener(
					"pageshow",
					resetNavigationAfterHistoryRestore,
				);
				document.removeEventListener("visibilitychange", flushWhenHidden);
				document.removeEventListener("click", handleNavigationClick);
				if (frame) cancelAnimationFrame(frame);
				if (saveTimer) clearTimeout(saveTimer);
				if (progressFlushRef.current === flushProgress) {
					progressFlushRef.current = undefined;
				}
				void flushLatest();
			};
		};

		void initialize();
		return () => {
			disposed = true;
			stopTracking();
		};
	}, [
		currentChapter,
		currentEntryId,
		currentPath,
		currentTitle,
		editionId,
		isReadingPage,
		navigateAfterSaving,
		updateReaderData,
	]);

	useEffect(() => {
		const progressByPath = new Map(
			readerData.progress.map((record) => [
				normalizeReadingPath(record.path),
				record,
			]),
		);
		const links = document.querySelectorAll<HTMLAnchorElement>(
			".sidebar-content a[href]",
		);
		for (const link of links) {
			const chapter = chapters.find(
				(item) =>
					normalizeReadingPath(
						new URL(link.href, window.location.origin).pathname,
					) === normalizeReadingPath(item.path),
			);
			if (!chapter) continue;
			const progress = progressByPath.get(normalizeReadingPath(chapter.path));
			const value = progress?.furthestPercent ?? 0;
			link.classList.add("flowbook-reading-link");
			link.style.setProperty("--flowbook-link-progress", percentage(value));
			link.dataset.readingStatus = progress?.completed
				? "Read"
				: value > 0
					? percentage(value)
					: "";
			link.dataset.readingComplete = progress?.completed ? "true" : "false";
			let accessibleStatus = link.querySelector<HTMLSpanElement>(
				".flowbook-reading-status-sr",
			);
			if (!accessibleStatus) {
				accessibleStatus = document.createElement("span");
				accessibleStatus.className = "flowbook-reading-status-sr";
				link.append(accessibleStatus);
			}
			accessibleStatus.textContent = progress?.completed
				? ", completed"
				: value > 0
					? `, ${percentage(value)} read`
					: "";
		}
	}, [chapters, readerData.progress]);

	useEffect(() => {
		if (!isLandingPage || !mostRecentProgress) return;
		const resumeHref = locationHref(mostRecentProgress, "resume");
		const heroAction = document.querySelector<HTMLAnchorElement>(
			'.flowbook-hero__actions a[href="/introduction/"]',
		);
		const closingAction = document.querySelector<HTMLAnchorElement>(
			'.book-home__closing-link[href="/introduction/"]',
		);
		for (const action of [heroAction, closingAction]) {
			if (!action) continue;
			action.href = resumeHref;
			const label = action.querySelector("span");
			if (label)
				label.textContent = `Continue reading · ${percentage(mostRecentProgress.percent)}`;
		}
	}, [isLandingPage, mostRecentProgress]);

	const announce = (message: string) => {
		if (announcementTimerRef.current) {
			clearTimeout(announcementTimerRef.current);
		}
		setAnnouncement(message);
		announcementTimerRef.current = setTimeout(() => {
			setAnnouncement("");
			announcementTimerRef.current = undefined;
		}, 2600);
	};
	const markStorageUnavailable = () => {
		storageWritableRef.current = false;
		setStorageAvailable(false);
	};

	const closePanel = useCallback(() => {
		setActiveTab(null);
		const trigger = panelTriggerRef.current;
		requestAnimationFrame(() => trigger?.focus());
	}, []);

	const openInlineAnnotation = useCallback(
		(request: InlineAnnotationOpenRequest) => {
			clearReadingSelection();
			if (document.activeElement instanceof HTMLElement) {
				panelTriggerRef.current = document.activeElement;
			}
			setActiveAnnotationId(request.id);
			setActiveTab(request.kind === "bookmark" ? "bookmarks" : "comments");
			requestAnimationFrame(() => {
				const target = Array.from(
					panelRef.current?.querySelectorAll<HTMLElement>(
						"[data-flowbook-annotation-id]",
					) ?? [],
				).find(
					(candidate) => candidate.dataset.flowbookAnnotationId === request.id,
				);
				target?.scrollIntoView({ block: "nearest" });
				target
					?.querySelector<HTMLButtonElement>("button")
					?.focus({ preventScroll: true });
			});
		},
		[clearReadingSelection],
	);

	useEffect(() => {
		if (!isReadingPage) return;
		const content = document.querySelector<HTMLElement>(
			"main[data-pagefind-body] .sl-markdown-content",
		);
		if (!content) return;
		return installInlineAnnotations(
			content,
			currentEntryBookmarks,
			currentEntryComments,
			openInlineAnnotation,
		);
	}, [
		currentEntryBookmarks,
		currentEntryComments,
		isReadingPage,
		openInlineAnnotation,
	]);

	const prepareCommentDraft = (selection?: ReadingSelection) => {
		if (editingCommentId) return;
		const snapshot =
			selection ?? readReadingSelection() ?? readingSelectionRef.current;
		if (!snapshot) return;
		setDraftQuote(snapshot.quote);
		setDraftLocation(snapshot.location);
		clearReadingSelection();
	};
	const preserveCurrentSelection = () => {
		const snapshot = readReadingSelection();
		if (snapshot) updateReadingSelection(snapshot);
	};

	const openTab = (tab: ReaderTab, selection?: ReadingSelection) => {
		if (activeTab === tab) {
			closePanel();
			return;
		}
		const opening = activeTab === null;
		if (opening && document.activeElement instanceof HTMLElement) {
			panelTriggerRef.current = document.activeElement;
		}
		setActiveTab(tab);
		if (tab === "comments") prepareCommentDraft(selection);
		requestAnimationFrame(() => {
			if (tab === "comments") {
				panelRef.current
					?.querySelector<HTMLTextAreaElement>("textarea")
					?.focus();
			} else if (opening) {
				closeButtonRef.current?.focus();
			}
		});
	};

	const openCommentForSelection = (selection: ReadingSelection) => {
		if (editingCommentId) return;
		if (activeTab === null && document.activeElement instanceof HTMLElement) {
			panelTriggerRef.current = document.activeElement;
		}
		setActiveTab("comments");
		prepareCommentDraft(selection);
		requestAnimationFrame(() => {
			panelRef.current?.querySelector<HTMLTextAreaElement>("textarea")?.focus();
		});
	};

	const selectPanelTab = (tab: ReaderTab) => {
		setActiveTab(tab);
		if (tab === "comments") {
			prepareCommentDraft();
			requestAnimationFrame(() => {
				panelRef.current
					?.querySelector<HTMLTextAreaElement>("textarea")
					?.focus();
			});
		}
	};

	useEffect(() => {
		if (!activeTab) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") closePanel();
		};
		document.addEventListener("keydown", onKeyDown);
		return () => document.removeEventListener("keydown", onKeyDown);
	}, [activeTab, closePanel]);

	const dismissResume = () => {
		awaitingResumeRef.current = false;
		setResumeTarget(undefined);
		window.dispatchEvent(new Event("flowbook-reader:measure"));
		const chapterTitle = document.querySelector<HTMLElement>("h1#_top");
		if (chapterTitle) {
			chapterTitle.setAttribute("tabindex", "-1");
			requestAnimationFrame(() => chapterTitle.focus({ preventScroll: true }));
		}
	};

	const goToSavedLocation = async (
		location: ReadingBookmark | ReadingComment | ReadingProgressRecord,
	) => {
		if (!canonicalEntryIds.has(location.entryId)) {
			announce("This saved item belongs to an earlier version of FlowBook");
			return;
		}
		if (
			normalizeReadingPath(location.path) !== normalizeReadingPath(currentPath)
		) {
			navigateAfterSaving(locationHref(location, "location"));
			return;
		}
		awaitingResumeRef.current = false;
		await restoreLocation(location, true);
		setActiveTab(null);
		setResumeTarget(undefined);
		window.dispatchEvent(new Event("flowbook-reader:measure"));
	};

	const toggleBookmark = async (selection?: ReadingSelection) => {
		const snapshot = selection ?? readingSelectionRef.current;
		const location =
			snapshot?.location ??
			readDomLocation() ??
			liveLocationRef.current ??
			currentProgress;
		if (!location || !currentChapter) {
			announce("This reading position is not ready yet");
			return;
		}
		liveLocationRef.current = location;
		setLiveLocation(location);
		if (snapshot) clearReadingSelection();
		const id = snapshot
			? passageBookmarkRecordId(
					editionId,
					currentEntryId,
					location.headingId,
					snapshot.quote,
					location.sectionProgress,
				)
			: bookmarkRecordId(editionId, currentEntryId, location.headingId);
		const existing = readerDataRef.current.bookmarks.find(
			(bookmark) => bookmark.id === id,
		);

		if (existing) {
			updateReaderData((current) => ({
				...current,
				bookmarks: current.bookmarks.filter((bookmark) => bookmark.id !== id),
			}));
			if (!storageWritableRef.current) {
				announce("Bookmark removed for this session");
				return;
			}
			try {
				await trackAnnotationWrite(deleteReadingBookmark(id));
				announce("Bookmark removed");
			} catch {
				markStorageUnavailable();
				announce("Bookmark removed for this session");
			}
			return;
		}

		const bookmark: ReadingBookmark = {
			id,
			editionId,
			entryId: currentEntryId,
			path: normalizeReadingPath(currentPath),
			title: currentTitle,
			headingId: location.headingId,
			headingText: location.headingText,
			scrollY: location.scrollY,
			headingOffset: location.headingOffset,
			sectionProgress: location.sectionProgress,
			percent: location.percent,
			quote: snapshot?.quote,
			createdAt: new Date().toISOString(),
		};
		updateReaderData((current) => ({
			...current,
			bookmarks: [bookmark, ...current.bookmarks],
		}));
		if (!storageWritableRef.current) {
			announce("Bookmark kept for this session");
			return;
		}
		try {
			await trackAnnotationWrite(saveReadingBookmark(bookmark));
			announce(`Bookmarked ${bookmark.headingText}`);
		} catch {
			markStorageUnavailable();
			announce("Bookmark kept for this session");
		}
	};

	const removeBookmark = async (id: string) => {
		updateReaderData((current) => ({
			...current,
			bookmarks: current.bookmarks.filter((bookmark) => bookmark.id !== id),
		}));
		if (!storageWritableRef.current) {
			announce("Bookmark removed for this session");
			return;
		}
		try {
			await trackAnnotationWrite(deleteReadingBookmark(id));
			announce("Bookmark removed");
		} catch {
			markStorageUnavailable();
			announce("Bookmark removed for this session");
		}
	};

	const submitComment = async (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		const body = draftComment.trim();
		const existing = editingCommentId
			? readerDataRef.current.comments.find(
					(comment) => comment.id === editingCommentId,
				)
			: undefined;
		const location =
			existing ??
			draftLocation ??
			readDomLocation() ??
			liveLocationRef.current ??
			currentProgress;
		if (!body || !location || !currentChapter) {
			if (body) announce("This reading position is not ready yet");
			return;
		}
		const now = new Date().toISOString();
		const comment: ReadingComment = {
			id: existing?.id ?? createCommentId(),
			editionId,
			entryId: existing?.entryId ?? currentEntryId,
			path: existing?.path ?? normalizeReadingPath(currentPath),
			title: existing?.title ?? currentTitle,
			headingId: existing?.headingId ?? location.headingId,
			headingText: existing?.headingText ?? location.headingText,
			scrollY: existing?.scrollY ?? location.scrollY,
			headingOffset: existing?.headingOffset ?? location.headingOffset,
			sectionProgress: existing
				? existing.sectionProgress
				: location.sectionProgress,
			percent: existing?.percent ?? location.percent,
			body,
			quote: existing?.quote ?? draftQuote,
			createdAt: existing?.createdAt ?? now,
			updatedAt: now,
		};
		updateReaderData((current) => ({
			...current,
			comments: [
				comment,
				...current.comments.filter((item) => item.id !== comment.id),
			],
		}));
		setDraftComment("");
		setDraftQuote(undefined);
		setDraftLocation(undefined);
		setEditingCommentId(undefined);
		clearReadingSelection();
		if (!storageWritableRef.current) {
			announce("Comment kept for this session");
			return;
		}
		try {
			await trackAnnotationWrite(saveReadingComment(comment));
			announce(existing ? "Comment updated" : "Comment saved");
		} catch {
			markStorageUnavailable();
			announce("Comment kept for this session");
		}
	};

	const editComment = (comment: ReadingComment) => {
		setDraftComment(comment.body);
		setDraftQuote(comment.quote);
		setDraftLocation(undefined);
		setEditingCommentId(comment.id);
		setActiveTab("comments");
		requestAnimationFrame(() => {
			panelRef.current?.querySelector<HTMLTextAreaElement>("textarea")?.focus();
		});
	};

	const removeComment = async (id: string) => {
		updateReaderData((current) => ({
			...current,
			comments: current.comments.filter((comment) => comment.id !== id),
		}));
		if (editingCommentId === id) {
			setEditingCommentId(undefined);
			setDraftComment("");
			setDraftQuote(undefined);
			setDraftLocation(undefined);
		}
		if (!storageWritableRef.current) {
			announce("Comment hidden for this session");
			return;
		}
		try {
			await trackAnnotationWrite(deleteReadingComment(id));
			announce("Comment deleted");
		} catch {
			markStorageUnavailable();
			announce("Comment hidden for this session");
		}
	};

	if (!isReadingPage) {
		if (
			!isLandingPage ||
			!mostRecentProgress ||
			resumeCardDismissed ||
			!portalHost
		) {
			return null;
		}
		return createPortal(
			<aside
				className="flowbook-resume-card"
				aria-label="Continue reading FlowBook"
			>
				<span className="flowbook-resume-card__eyebrow">Continue FlowBook</span>
				<strong>{mostRecentProgress.title}</strong>
				<span>{mostRecentProgress.headingText}</span>
				<a href={locationHref(mostRecentProgress, "resume")}>
					Resume at {percentage(mostRecentProgress.percent)}
				</a>
				<button
					type="button"
					className="flowbook-resume-card__dismiss"
					onClick={() => setResumeCardDismissed(true)}
					aria-label="Dismiss continue reading card"
				>
					×
				</button>
			</aside>,
			portalHost,
		);
	}

	return (
		<>
			<div className="flowbook-page-progress" aria-hidden="true">
				<i style={{ width: percentage(currentProgress?.percent ?? 0) }} />
			</div>

			{portalHost &&
				createPortal(
					<>
						{resumeTarget && (
							<div className="flowbook-resume-toast" aria-live="polite">
								<div>
									<span>Welcome back</span>
									<strong>
										Continue from {percentage(resumeTarget.percent)}
									</strong>
								</div>
								<button
									type="button"
									onClick={() => void goToSavedLocation(resumeTarget)}
								>
									Resume
								</button>
								<button
									type="button"
									className="flowbook-resume-toast__dismiss"
									onClick={dismissResume}
									aria-label="Dismiss resume prompt"
								>
									×
								</button>
							</div>
						)}

						{readingSelection &&
							!resumeTarget &&
							(activeTab === null || activeTab === "comments") && (
								<fieldset
									className="flowbook-selection-tools"
									aria-label="Actions for selected text"
									style={{
										left: readingSelection.left,
										top: readingSelection.top,
									}}
								>
									<span>Selected</span>
									<button
										type="button"
										onPointerDown={(event) => event.preventDefault()}
										onClick={() => void toggleBookmark(readingSelection)}
									>
										<i className="flowbook-bookmark-glyph" />
										Bookmark
									</button>
									<button
										type="button"
										className="flowbook-selection-tools__comment"
										onPointerDown={(event) => event.preventDefault()}
										onClick={() => openCommentForSelection(readingSelection)}
										disabled={Boolean(editingCommentId)}
										aria-label={
											editingCommentId
												? "Finish or cancel editing before adding another note"
												: "Add a note for selected text"
										}
									>
										<i className="flowbook-comment-glyph" />
										{editingCommentId ? "Finish edit" : "Add note"}
									</button>
									<button
										type="button"
										className="flowbook-selection-tools__dismiss"
										onClick={clearReadingSelection}
										aria-label="Dismiss selected text actions"
									>
										×
									</button>
								</fieldset>
							)}

						{!resumeTarget && (
							<nav
								className={`flowbook-reader-dock${activeTab ? " is-panel-open" : ""}`}
								aria-label="Reading tools"
							>
								<button
									type="button"
									className="flowbook-reader-dock__progress"
									onClick={() => openTab("overview")}
									aria-expanded={activeTab === "overview"}
									aria-controls="flowbook-reader-panel"
								>
									<i
										style={
											{
												"--flowbook-current-progress": percentage(
													currentProgress?.percent ?? 0,
												),
											} as React.CSSProperties
										}
									>
										{Math.round((currentProgress?.percent ?? 0) * 100)}%
									</i>
									<span>Progress</span>
								</button>
								<button
									type="button"
									className={currentBookmark ? "is-active" : undefined}
									onPointerDown={preserveCurrentSelection}
									onClick={() => void toggleBookmark()}
									aria-pressed={Boolean(currentBookmark)}
									aria-label={
										currentBookmark
											? "Remove section bookmark"
											: "Bookmark this section"
									}
								>
									<i className="flowbook-bookmark-glyph" />
									<span>{currentBookmark ? "Saved" : "Bookmark"}</span>
								</button>
								<button
									type="button"
									onPointerDown={preserveCurrentSelection}
									onClick={() => openTab("comments")}
									aria-expanded={activeTab === "comments"}
									aria-controls="flowbook-reader-panel"
								>
									<i className="flowbook-comment-glyph" />
									<span>Comment</span>
									{canonicalComments.length > 0 && (
										<b>{canonicalComments.length}</b>
									)}
								</button>
							</nav>
						)}

						{activeTab && !resumeTarget && (
							<dialog
								id="flowbook-reader-panel"
								className="flowbook-reader-panel"
								ref={panelRef}
								open
								aria-modal="false"
								aria-labelledby="flowbook-reader-title"
							>
								<header>
									<div>
										<span>Private reading space</span>
										<h2 id="flowbook-reader-title">Your FlowBook</h2>
									</div>
									<button
										ref={closeButtonRef}
										type="button"
										onClick={closePanel}
										aria-label="Close reading tools"
									>
										×
									</button>
								</header>

								<nav aria-label="Reading tool sections">
									{(["overview", "bookmarks", "comments"] as ReaderTab[]).map(
										(tab) => (
											<button
												key={tab}
												type="button"
												aria-pressed={activeTab === tab}
												aria-controls="flowbook-reader-panel-body"
												onClick={() => selectPanelTab(tab)}
											>
												{tab === "overview"
													? "Overview"
													: tab === "bookmarks"
														? `Bookmarks ${canonicalBookmarks.length}`
														: `Comments ${canonicalComments.length}`}
											</button>
										),
									)}
								</nav>

								<div
									id="flowbook-reader-panel-body"
									className="flowbook-reader-panel__body"
								>
									{activeTab === "overview" && (
										<section
											className="flowbook-reader-overview"
											aria-label="Reading overview"
										>
											<div className="flowbook-reader-overview__score">
												<strong>{percentage(summary.overallPercent)}</strong>
												<span>of the current edition explored</span>
												<i>
													<b
														style={{
															width: percentage(summary.overallPercent),
														}}
													/>
												</i>
											</div>
											<div className="flowbook-reader-stats">
												<div>
													<strong>{summary.completedChapters}</strong>
													<span>chapters read</span>
												</div>
												<div>
													<strong>{canonicalBookmarks.length}</strong>
													<span>bookmarks</span>
												</div>
												<div>
													<strong>{canonicalComments.length}</strong>
													<span>comments</span>
												</div>
											</div>
											{currentProgress && (
												<button
													type="button"
													className="flowbook-current-location"
													onClick={() =>
														void goToSavedLocation(currentProgress)
													}
												>
													<span>
														Current chapter ·{" "}
														{percentage(currentProgress.percent)}
													</span>
													<strong>{currentProgress.headingText}</strong>
												</button>
											)}
											<p
												className={storageAvailable ? undefined : "is-warning"}
											>
												<i />
												{storageAvailable
													? "Progress and annotations are saved privately in this browser."
													: "This browser blocked local saving. Reading still works, but changes may not survive a reload."}
											</p>
											{archivedAnnotationCount > 0 && (
												<p className="is-warning">
													<i />
													{archivedAnnotationCount} saved{" "}
													{archivedAnnotationCount === 1
														? "item belongs"
														: "items belong"}{" "}
													to an older book structure and{" "}
													{archivedAnnotationCount === 1 ? "is" : "are"} kept
													locally.
												</p>
											)}
										</section>
									)}

									{activeTab === "bookmarks" && (
										<section
											className="flowbook-saved-list"
											aria-label="Saved bookmarks"
										>
											<button
												type="button"
												className="flowbook-save-current"
												onClick={() => void toggleBookmark()}
											>
												<i className="flowbook-bookmark-glyph" />
												<span>
													<strong>
														{currentBookmark
															? "Remove this bookmark"
															: "Bookmark this section"}
													</strong>
													<small>
														{activeLocation?.headingText ??
															"Current reading position"}
													</small>
												</span>
											</button>
											{canonicalBookmarks.length === 0 ? (
												<div className="flowbook-empty-state">
													<strong>No bookmarks yet</strong>
													<span>
														Save a section and it will appear here on your next
														visit.
													</span>
												</div>
											) : (
												canonicalBookmarks.map((bookmark) => (
													<article
														key={bookmark.id}
														data-flowbook-annotation-id={bookmark.id}
														className={
															activeAnnotationId === bookmark.id
																? "is-targeted"
																: undefined
														}
													>
														<button
															type="button"
															onClick={() => void goToSavedLocation(bookmark)}
														>
															<span>{bookmark.title}</span>
															<strong>{bookmark.headingText}</strong>
															{bookmark.quote && (
																<span className="flowbook-saved-quote">
																	“{bookmark.quote}”
																</span>
															)}
															<small>
																{percentage(bookmark.percent)} ·{" "}
																{formatSavedDate(bookmark.createdAt)}
															</small>
														</button>
														<button
															type="button"
															onClick={() => void removeBookmark(bookmark.id)}
															aria-label={`Remove bookmark for ${bookmark.headingText}`}
														>
															×
														</button>
													</article>
												))
											)}
										</section>
									)}

									{activeTab === "comments" && (
										<section
											className="flowbook-comments"
											aria-label="Private comments"
										>
											<form onSubmit={(event) => void submitComment(event)}>
												<label htmlFor="flowbook-comment-draft">
													{editingCommentId
														? "Edit comment"
														: "Comment on this section"}
													<span>
														{draftLocation?.headingText ??
															activeLocation?.headingText ??
															"Current position"}
													</span>
												</label>
												{draftQuote && <blockquote>“{draftQuote}”</blockquote>}
												<textarea
													id="flowbook-comment-draft"
													value={draftComment}
													onPointerDown={clearReadingSelection}
													onChange={(event) =>
														setDraftComment(event.target.value)
													}
													placeholder="Write a private thought, question, or reminder…"
													maxLength={2000}
													rows={4}
												/>
												<div>
													<small>
														{draftComment.length}/2000 · only on this device
													</small>
													{editingCommentId && (
														<button
															type="button"
															onClick={() => {
																setEditingCommentId(undefined);
																setDraftComment("");
																setDraftQuote(undefined);
																setDraftLocation(undefined);
															}}
														>
															Cancel
														</button>
													)}
													<button type="submit" disabled={!draftComment.trim()}>
														{editingCommentId ? "Update" : "Save comment"}
													</button>
												</div>
											</form>

											{canonicalComments.length === 0 ? (
												<div className="flowbook-empty-state">
													<strong>Your margin is clear</strong>
													<span>
														Select a passage or leave a thought for this
														section.
													</span>
												</div>
											) : (
												<div className="flowbook-comment-list">
													{canonicalComments.map((comment) => (
														<article
															key={comment.id}
															data-flowbook-annotation-id={comment.id}
															className={
																activeAnnotationId === comment.id
																	? "is-targeted"
																	: undefined
															}
														>
															<button
																type="button"
																onClick={() => void goToSavedLocation(comment)}
															>
																<span>{comment.title}</span>
																<strong>{comment.headingText}</strong>
															</button>
															{comment.quote && (
																<blockquote>“{comment.quote}”</blockquote>
															)}
															<p>{comment.body}</p>
															<footer>
																<small>
																	{formatSavedDate(comment.updatedAt)}
																</small>
																<button
																	type="button"
																	onClick={() => editComment(comment)}
																>
																	Edit
																</button>
																<button
																	type="button"
																	onClick={() => void removeComment(comment.id)}
																>
																	Delete
																</button>
															</footer>
														</article>
													))}
												</div>
											)}
										</section>
									)}
								</div>
							</dialog>
						)}

						<div
							className={`flowbook-reader-announcement${announcement ? " is-visible" : ""}`}
							aria-live="polite"
							aria-atomic="true"
						>
							{announcement}
						</div>
					</>,
					portalHost,
				)}
		</>
	);
}
