import {
	type CanaryRun,
	type CanarySetup,
	canaryExpectation,
	canaryUrl,
} from "./canary";
import {
	DASH_TYPE,
	HLS_TYPE,
	type LoadResult,
	type MediaObservation,
	ObjectUrls,
	SMOOTH_TYPE,
	dashManifest,
	dataUrl,
	describeMedia,
	fetchLoads,
	hlsMasterPlaylist,
	hlsMediaPlaylist,
	imageLoads,
	playMedia,
	smoothManifest,
	vttDocument,
	wavBytes,
} from "./media";
import { directiveSources, servedPolicy, sourceCovers } from "./policy";
import {
	type ProbeCheck,
	type ProbeChecks,
	check,
	delay,
	describeError,
} from "./report";

const MEDIA_WAIT_MS = 4_000;
const ARTWORK_PLAY_MS = 6_000;
const LOAD_WAIT_MS = 4_000;
const NETWORK_DIRECTIVES = [
	"connect-src",
	"img-src",
	"media-src",
	"font-src",
	"style-src",
] as const;
const ALLOWED = "allowed (local bytes need no approval)";
const CANARY_SILENT = "no request in the canary log";
const FOREIGN_BLOCKED =
	"blocked: another document's blob: URL never resolves here";

export interface LocalSuiteOptions {
	canary: CanarySetup;
	foreignBlobUrls: readonly string[];
	slot: HTMLElement;
	onCheck(id: string, result: ProbeCheck): void;
}

type StreamFormat = "hlsMaster" | "hlsMedia" | "dash" | "smooth";
type LocalScheme = "data" | "blob";

const STREAMS: Record<
	StreamFormat,
	{ type: string; build: (url: (file: string) => string) => string }
> = {
	hlsMaster: { type: HLS_TYPE, build: hlsMasterPlaylist },
	hlsMedia: { type: HLS_TYPE, build: hlsMediaPlaylist },
	dash: { type: DASH_TYPE, build: dashManifest },
	smooth: { type: SMOOTH_TYPE, build: smoothManifest },
};

const STREAM_ROWS: { id: string; format: StreamFormat; scheme: LocalScheme }[] =
	[
		{ id: "local.hlsMasterData", format: "hlsMaster", scheme: "data" },
		{ id: "local.hlsMasterBlob", format: "hlsMaster", scheme: "blob" },
		{ id: "local.hlsMediaData", format: "hlsMedia", scheme: "data" },
		{ id: "local.hlsMediaBlob", format: "hlsMedia", scheme: "blob" },
		{ id: "local.dashData", format: "dash", scheme: "data" },
		{ id: "local.dashBlob", format: "dash", scheme: "blob" },
		{ id: "local.smoothData", format: "smooth", scheme: "data" },
		{ id: "local.smoothBlob", format: "smooth", scheme: "blob" },
	];

const FOREIGN_ROWS = {
	fetch: "local.foreignBlob.fetch",
	img: "local.foreignBlob.img",
	video: "local.foreignBlob.video",
} as const;

function canaryReview(
	run: CanaryRun,
	rowId: string,
	observed: string,
): ProbeCheck {
	return check(
		"review",
		canaryExpectation(run, rowId),
		`${observed}; read the canary log`,
	);
}

function allowedCheck(result: LoadResult, what: string): ProbeCheck {
	if (result === "loaded") return check("pass", ALLOWED, `${what} loaded`);
	return result === "timeout"
		? check("review", ALLOWED, `${what} did not settle within the wait`)
		: check("fail", ALLOWED, `${what} failed to load`);
}

function mediaAllowedCheck(observation: MediaObservation): ProbeCheck {
	if (observation.loaded) {
		return check(
			"pass",
			ALLOWED,
			`metadata loaded; ${describeMedia(observation)}`,
		);
	}
	return check(
		observation.error ? "fail" : "review",
		ALLOWED,
		describeMedia(observation),
	);
}

/** The canary proves nothing when the served policy already allows its host */
function checkCanaryUndeclared(canary: CanarySetup): ProbeCheck {
	const expected = "canary host not allowed by the served policy";
	if (canary.kind !== "ready") return check("skip", expected, canary.reason);
	const policy = servedPolicy();
	if (policy === null) {
		return check("review", expected, "no single CSP meta to compare against");
	}
	const allowing = NETWORK_DIRECTIVES.filter((directive) =>
		(directiveSources(policy, directive) ?? []).some((source) =>
			sourceCovers(source, canary.run.origin),
		),
	);
	return allowing.length === 0
		? check("pass", expected, `${canary.run.origin} is in no directive`)
		: check(
				"fail",
				expected,
				`${allowing.join(", ")} allow ${canary.run.origin}; pick a canary host no source covers`,
			);
}

interface ArtworkProbe {
	finish(): Promise<ProbeCheck>;
}

/**
 * Starts audible `data:` playback with MediaSession artwork on the canary.
 * Runs synchronously inside the click that started the suite, while the
 * document still has user activation.
 */
function startArtworkProbe(canary: CanarySetup): ArtworkProbe {
	const rowId = "local.mediaSessionArtwork";
	if (canary.kind !== "ready") {
		return { finish: async () => check("skip", CANARY_SILENT, canary.reason) };
	}
	const expected = canaryExpectation(canary.run, rowId);
	if (!("mediaSession" in navigator) || typeof MediaMetadata !== "function") {
		return {
			finish: async () =>
				check("skip", expected, "this engine has no Media Session API"),
		};
	}
	const audio = new Audio(
		dataUrl("audio/wav", wavBytes(ARTWORK_PLAY_MS / 1000 + 1, 440, 12)),
	);
	audio.volume = 0.3;
	navigator.mediaSession.metadata = new MediaMetadata({
		title: "Widget CSP probe",
		artist: "Artwork must not load",
		artwork: [
			{
				src: canaryUrl(canary.run, rowId, "artwork.png"),
				sizes: "512x512",
				type: "image/png",
			},
		],
	});
	const started = Date.now();
	const playing = audio.play().then(
		() => "data: audio played",
		(error: unknown) => `play() rejected: ${describeError(error)}`,
	);
	return {
		async finish() {
			const state = await playing;
			await delay(Math.max(0, ARTWORK_PLAY_MS - (Date.now() - started)));
			audio.pause();
			navigator.mediaSession.metadata = null;
			return canaryReview(
				canary.run,
				rowId,
				`${state} for ${ARTWORK_PLAY_MS / 1000} s with artwork set`,
			);
		},
	};
}

async function streamRow(
	run: CanaryRun,
	row: (typeof STREAM_ROWS)[number],
	slot: HTMLElement,
	objectUrls: ObjectUrls,
): Promise<ProbeCheck> {
	const { type, build } = STREAMS[row.format];
	const manifest = build((file) => canaryUrl(run, row.id, file));
	const src =
		row.scheme === "data"
			? dataUrl(type, manifest)
			: objectUrls.create(type, manifest);
	const support = document.createElement("video").canPlayType(type) || "no";
	const observation = await playMedia(slot, "video", src, MEDIA_WAIT_MS);
	return canaryReview(
		run,
		row.id,
		`canPlayType("${type}") = ${support}; ${describeMedia(observation)}`,
	);
}

async function vttRow(
	run: CanaryRun,
	slot: HTMLElement,
	audioSrc: string,
): Promise<ProbeCheck> {
	const rowId = "local.vttData";
	const track = document.createElement("track");
	const trackEvents: string[] = [];
	track.kind = "subtitles";
	track.srclang = "en";
	track.label = "probe";
	track.default = true;
	track.addEventListener("load", () => trackEvents.push("load"));
	track.addEventListener("error", () => trackEvents.push("error"));
	track.src = dataUrl(
		"text/vtt",
		vttDocument((file) => canaryUrl(run, rowId, file)),
	);
	const observation = await playMedia(
		slot,
		"video",
		audioSrc,
		MEDIA_WAIT_MS,
		(element) => {
			element.append(track);
			track.track.mode = "showing";
		},
	);
	return canaryReview(
		run,
		rowId,
		`data: track readyState ${track.readyState}, track events: ${trackEvents.join(", ") || "none"}; media ${describeMedia(observation)}`,
	);
}

type ForeignResult = { url: string; result: LoadResult };

function foreignVerdict(results: ForeignResult[], via: string): ProbeCheck {
	const observed = results
		.map(({ url, result }) => `${url}: ${result}`)
		.join("; ");
	if (results.some(({ result }) => result === "loaded")) {
		return check(
			"fail",
			FOREIGN_BLOCKED,
			`${via} read another document's blob: ${observed}`,
		);
	}
	return results.some(({ result }) => result === "timeout")
		? check(
				"review",
				FOREIGN_BLOCKED,
				`${observed}; a timeout is neither a load nor a refusal`,
			)
		: check(
				"pass",
				FOREIGN_BLOCKED,
				`${observed}; valid only while each URL's document was still open`,
			);
}

async function foreignBlobRows(
	urls: readonly string[],
	slot: HTMLElement,
): Promise<Record<string, ProbeCheck>> {
	const blobs = urls
		.map((url) => url.trim())
		.filter((url) => url.startsWith("blob:"));
	if (blobs.length === 0) {
		const reason =
			urls.length === 0
				? "set foreignBlobUrls to blob: URLs from the sibling widget and the host page"
				: "foreignBlobUrls holds no blob: URL";
		return Object.fromEntries(
			Object.values(FOREIGN_ROWS).map((id) => [
				id,
				check("skip", FOREIGN_BLOCKED, reason),
			]),
		);
	}
	const fetches = await Promise.all(
		blobs.map(async (url) => ({ url, result: await fetchLoads(url) })),
	);
	const images = await Promise.all(
		blobs.map(async (url) => ({
			url,
			result: await imageLoads(url, LOAD_WAIT_MS),
		})),
	);
	const videos = await Promise.all(
		blobs.map(async (url): Promise<ForeignResult> => {
			const observation = await playMedia(slot, "video", url, MEDIA_WAIT_MS);
			const result: LoadResult = observation.loaded
				? "loaded"
				: observation.error
					? "error"
					: "timeout";
			return { url, result };
		}),
	);
	return {
		[FOREIGN_ROWS.fetch]: foreignVerdict(fetches, "fetch"),
		[FOREIGN_ROWS.img]: foreignVerdict(images, "img"),
		[FOREIGN_ROWS.video]: foreignVerdict(videos, "video"),
	};
}

async function dataFetchResult(): Promise<LoadResult> {
	try {
		const response = await fetch("data:application/json,%7B%22ok%22%3Atrue%7D");
		const value: unknown = await response.json();
		return typeof value === "object" && value !== null && "ok" in value
			? "loaded"
			: "error";
	} catch {
		return "error";
	}
}

/**
 * Local-scheme rows (§14.1 ship gate). Call it synchronously from a click:
 * the MediaSession row starts audio before the first await.
 */
export async function runLocalSuite({
	canary,
	foreignBlobUrls,
	slot,
	onCheck,
}: LocalSuiteOptions): Promise<ProbeChecks> {
	const artwork = startArtworkProbe(canary);
	const checks: ProbeChecks = {};
	const record = (id: string, result: ProbeCheck) => {
		checks[id] = result;
		onCheck(id, result);
	};
	const objectUrls = new ObjectUrls();
	slot.replaceChildren();
	try {
		record("local.canaryUndeclared", checkCanaryUndeclared(canary));
		record(
			"local.fetchData",
			allowedCheck(await dataFetchResult(), "fetch of a data: URL"),
		);
		record(
			"local.fetchBlob",
			allowedCheck(
				await fetchLoads(objectUrls.create("text/plain", "csp-probe own blob")),
				"fetch of this document's blob: URL",
			),
		);
		record(
			"local.imgBlob",
			allowedCheck(
				await imageLoads(
					objectUrls.create(
						"image/svg+xml",
						'<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>',
					),
					LOAD_WAIT_MS,
				),
				"image from this document's blob: URL",
			),
		);

		const clip = wavBytes(0.5);
		const audioData = dataUrl("audio/wav", clip);
		const [mediaData, mediaBlob] = await Promise.all([
			playMedia(slot, "audio", audioData, MEDIA_WAIT_MS),
			playMedia(
				slot,
				"audio",
				objectUrls.create("audio/wav", clip),
				MEDIA_WAIT_MS,
			),
		]);
		record("local.mediaData", mediaAllowedCheck(mediaData));
		record("local.mediaBlob", mediaAllowedCheck(mediaBlob));

		if (canary.kind === "ready") {
			const [streams, vtt] = await Promise.all([
				Promise.all(
					STREAM_ROWS.map((row) =>
						streamRow(canary.run, row, slot, objectUrls),
					),
				),
				vttRow(canary.run, slot, audioData),
			]);
			STREAM_ROWS.forEach((row, index) => {
				const result = streams[index];
				if (result) record(row.id, result);
			});
			record("local.vttData", vtt);
		} else {
			for (const id of [...STREAM_ROWS.map((row) => row.id), "local.vttData"]) {
				record(id, check("skip", CANARY_SILENT, canary.reason));
			}
		}

		record("local.mediaSessionArtwork", await artwork.finish());

		for (const [id, result] of Object.entries(
			await foreignBlobRows(foreignBlobUrls, slot),
		)) {
			record(id, result);
		}
		return checks;
	} finally {
		slot.replaceChildren();
		objectUrls.revokeAll();
	}
}
