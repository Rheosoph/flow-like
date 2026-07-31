const MAX_FLOWSCRIPT_SOURCE_CHARS = 60_000;

export interface ReadFlowScriptSourceRequest {
	appId: string;
	boardId: string;
	locator?: string;
	/** The board specialist's own app, carried separately from an explicit cross-app target. */
	scopedAppId?: string;
}

export interface ReadFlowScriptSourceDependencies {
	getProfileAppIds: () => Promise<Set<string>>;
	getFlowScript: (appId: string, boardId: string) => Promise<string>;
}

interface BoundedFlowScriptSource {
	source: string;
	sourceChars: number;
	startOffset: number;
	endOffset: number;
	truncated: boolean;
	locatorMatched: boolean | undefined;
}

function locatorMatchOffset(source: string, locator: string): number {
	const exact = source.indexOf(locator);
	if (exact >= 0) return exact;

	// Scout locators are often descriptive prose followed by one or more stable node/function ids.
	// Prefer the longest identifier-like fragment, which makes those ids win over prose words.
	const candidates = Array.from(
		new Set(locator.match(/[A-Za-z0-9_:@./-]{8,}/g) ?? []),
	).sort((left, right) => right.length - left.length);
	for (const candidate of candidates) {
		const offset = source.indexOf(candidate);
		if (offset >= 0) return offset;
	}
	return -1;
}

/** Keep prior-art reads bounded while centering large sources on the requested locator. */
export function boundFlowScriptSource(
	source: string,
	locator?: string,
): BoundedFlowScriptSource {
	const trimmedLocator = locator?.trim() ?? "";
	const matchOffset = trimmedLocator
		? locatorMatchOffset(source, trimmedLocator)
		: -1;
	const locatorMatched = trimmedLocator ? matchOffset >= 0 : undefined;

	if (source.length <= MAX_FLOWSCRIPT_SOURCE_CHARS) {
		return {
			source,
			sourceChars: source.length,
			startOffset: 0,
			endOffset: source.length,
			truncated: false,
			locatorMatched,
		};
	}

	const desiredStart =
		matchOffset >= 0
			? Math.max(0, matchOffset - Math.floor(MAX_FLOWSCRIPT_SOURCE_CHARS / 3))
			: 0;
	let startOffset = desiredStart;
	if (startOffset > 0) {
		const nextLine = source.indexOf("\n", startOffset);
		if (
			nextLine >= 0 &&
			nextLine - startOffset < Math.floor(MAX_FLOWSCRIPT_SOURCE_CHARS / 4)
		) {
			startOffset = nextLine + 1;
		}
	}
	let endOffset = Math.min(
		source.length,
		startOffset + MAX_FLOWSCRIPT_SOURCE_CHARS,
	);
	if (endOffset < source.length) {
		const previousLine = source.lastIndexOf("\n", endOffset);
		if (previousLine > startOffset) endOffset = previousLine + 1;
	}

	return {
		source: source.slice(startOffset, endOffset),
		sourceChars: source.length,
		startOffset,
		endOffset,
		truncated: startOffset > 0 || endOffset < source.length,
		locatorMatched,
	};
}

/**
 * Read a board's canonical FlowScript after proving the target is either the specialist's own app
 * or an app visible in the user's current profile. The backend remains the authoritative board
 * access check; this frontend guard prevents arbitrary local app-id probing before that request.
 */
export async function readFlowScriptSource(
	request: ReadFlowScriptSourceRequest,
	dependencies: ReadFlowScriptSourceDependencies,
): Promise<Record<string, unknown>> {
	const appId = request.appId.trim();
	const boardId = request.boardId.trim();
	if (!appId || !boardId) {
		return {
			status: "error",
			code: "FLOWSCRIPT_SOURCE_TARGET_REQUIRED",
			message:
				"read_flowscript_source requires an app_id and board_id after scoped defaults are applied.",
		};
	}

	const scopedAppId = request.scopedAppId?.trim();
	const profileAppIds = await dependencies.getProfileAppIds();
	if (appId !== scopedAppId && !profileAppIds.has(appId)) {
		return {
			status: "forbidden",
			code: "FLOWSCRIPT_SOURCE_APP_NOT_ACCESSIBLE",
			app_id: appId,
			board_id: boardId,
			message:
				"The requested source app is not available in the user's current profile.",
		};
	}

	let source: string;
	try {
		source = await dependencies.getFlowScript(appId, boardId);
	} catch {
		return {
			status: "error",
			code: "FLOWSCRIPT_SOURCE_NOT_READABLE",
			app_id: appId,
			board_id: boardId,
			message:
				"The requested board could not be read. Verify the app and board ids and the user's board access.",
		};
	}

	const locator = request.locator?.trim() || undefined;
	const bounded = boundFlowScriptSource(source, locator);
	return {
		status: "ok",
		app_id: appId,
		board_id: boardId,
		...(locator ? { locator } : {}),
		...(bounded.locatorMatched === undefined
			? {}
			: { locator_matched: bounded.locatorMatched }),
		source: bounded.source,
		source_chars: bounded.sourceChars,
		returned_chars: bounded.source.length,
		start_offset: bounded.startOffset,
		end_offset: bounded.endOffset,
		truncated: bounded.truncated,
		...(locator && bounded.locatorMatched === false
			? {
					note: "The locator was not found verbatim or by stable identifier; the bounded board source is returned so the specialist can resolve it without inventing prior art.",
				}
			: {}),
	};
}
