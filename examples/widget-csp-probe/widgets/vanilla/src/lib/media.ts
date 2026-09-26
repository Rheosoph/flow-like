const SAMPLE_RATE = 8_000;

/** 8-bit mono PCM WAV; `amplitude` 0 is silence, 127 is full scale */
export function wavBytes(
	seconds: number,
	toneHz = 0,
	amplitude = 0,
): Uint8Array<ArrayBuffer> {
	const samples = Math.round(seconds * SAMPLE_RATE);
	const bytes = new Uint8Array(44 + samples);
	const view = new DataView(bytes.buffer);
	const ascii = (offset: number, text: string) => {
		for (let index = 0; index < text.length; index += 1) {
			view.setUint8(offset + index, text.charCodeAt(index));
		}
	};
	ascii(0, "RIFF");
	view.setUint32(4, 36 + samples, true);
	ascii(8, "WAVEfmt ");
	view.setUint32(16, 16, true);
	view.setUint16(20, 1, true);
	view.setUint16(22, 1, true);
	view.setUint32(24, SAMPLE_RATE, true);
	view.setUint32(28, SAMPLE_RATE, true);
	view.setUint16(32, 1, true);
	view.setUint16(34, 8, true);
	ascii(36, "data");
	view.setUint32(40, samples, true);
	for (let index = 0; index < samples; index += 1) {
		const wave = Math.sin((2 * Math.PI * toneHz * index) / SAMPLE_RATE);
		bytes[44 + index] = 128 + Math.round(amplitude * wave);
	}
	return bytes;
}

function base64(bytes: Uint8Array): string {
	let binary = "";
	const chunk = 0x8000;
	for (let offset = 0; offset < bytes.length; offset += chunk) {
		binary += String.fromCharCode(...bytes.subarray(offset, offset + chunk));
	}
	return btoa(binary);
}

export function dataUrl(type: string, content: string | Uint8Array): string {
	const bytes =
		typeof content === "string" ? new TextEncoder().encode(content) : content;
	return `data:${type};base64,${base64(bytes)}`;
}

/** Object URLs this document created; revoke them once a run is done */
export class ObjectUrls {
	private readonly urls: string[] = [];

	create(type: string, content: string | Uint8Array<ArrayBuffer>): string {
		const url = URL.createObjectURL(new Blob([content], { type }));
		this.urls.push(url);
		return url;
	}

	revokeAll(): void {
		for (const url of this.urls.splice(0)) URL.revokeObjectURL(url);
	}
}

function xml(value: string): string {
	return value
		.replaceAll("&", "&amp;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;")
		.replaceAll('"', "&quot;");
}

export const HLS_TYPE = "application/vnd.apple.mpegurl";
export const DASH_TYPE = "application/dash+xml";
export const SMOOTH_TYPE = "application/vnd.ms-sstr+xml";

/** Master playlist: every rendition and variant URI is absolute */
export function hlsMasterPlaylist(url: (file: string) => string): string {
	return [
		"#EXTM3U",
		"#EXT-X-VERSION:6",
		`#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aud",NAME="probe",DEFAULT=YES,AUTOSELECT=YES,URI="${url("rendition-audio.m3u8")}"`,
		`#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="subs",NAME="probe",LANGUAGE="en",DEFAULT=NO,AUTOSELECT=NO,URI="${url("rendition-subtitles.m3u8")}"`,
		'#EXT-X-STREAM-INF:BANDWIDTH=200000,CODECS="avc1.42e00a,mp4a.40.2",RESOLUTION=320x180,AUDIO="aud",SUBTITLES="subs"',
		url("variant.m3u8"),
		"",
	].join("\n");
}

/** Media playlist: EXT-X-KEY, EXT-X-MAP and segment URIs are absolute */
export function hlsMediaPlaylist(url: (file: string) => string): string {
	return [
		"#EXTM3U",
		"#EXT-X-VERSION:6",
		"#EXT-X-TARGETDURATION:4",
		"#EXT-X-MEDIA-SEQUENCE:0",
		"#EXT-X-PLAYLIST-TYPE:VOD",
		`#EXT-X-KEY:METHOD=AES-128,URI="${url("key.bin")}",IV=0x00000000000000000000000000000001`,
		`#EXT-X-MAP:URI="${url("init.mp4")}"`,
		"#EXTINF:4.0,",
		url("segment0.m4s"),
		"#EXTINF:4.0,",
		url("segment1.m4s"),
		"#EXT-X-ENDLIST",
		"",
	].join("\n");
}

/** MPD whose only media location is an absolute BaseURL */
export function dashManifest(url: (file: string) => string): string {
	return `<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011" mediaPresentationDuration="PT8S" minBufferTime="PT2S">
  <BaseURL>${xml(url("dash/"))}</BaseURL>
  <Period>
    <AdaptationSet mimeType="video/mp4" segmentAlignment="true">
      <Representation id="v0" bandwidth="200000" codecs="avc1.42e00a" width="320" height="180">
        <BaseURL>video.mp4</BaseURL>
        <SegmentBase indexRange="0-999"><Initialization range="0-499"/></SegmentBase>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
`;
}

/** Smooth Streaming manifest whose fragment template is absolute */
export function smoothManifest(url: (file: string) => string): string {
	const template = `${url("smooth")}/QualityLevels({bitrate})/Fragments(video={start time})`;
	return `<?xml version="1.0" encoding="UTF-8"?>
<SmoothStreamingMedia MajorVersion="2" MinorVersion="0" Duration="80000000">
  <StreamIndex Type="video" Name="video" Chunks="2" QualityLevels="1" MaxWidth="320" MaxHeight="180" Url="${xml(template)}">
    <QualityLevel Index="0" Bitrate="200000" FourCC="H264" MaxWidth="320" MaxHeight="180" CodecPrivateData="00000001674D401FE8802802DD80B501010140000003004000000C83C60C448000000168EBEF20"/>
    <c t="0" d="40000000"/>
    <c d="40000000"/>
  </StreamIndex>
</SmoothStreamingMedia>
`;
}

/** WebVTT whose STYLE block names an absolute image */
export function vttDocument(url: (file: string) => string): string {
	return `WEBVTT

STYLE
::cue {
  background-image: url("${url("vtt-style.png")}");
}

00:00.000 --> 00:05.000
CSP probe caption
`;
}

export interface MediaObservation {
	events: string[];
	error: string | null;
	/** `readyState` reached HAVE_METADATA */
	loaded: boolean;
}

const MEDIA_EVENTS = [
	"loadstart",
	"loadedmetadata",
	"loadeddata",
	"canplay",
	"playing",
	"stalled",
	"waiting",
	"abort",
	"emptied",
	"error",
] as const;

function mediaError(element: HTMLMediaElement): string | null {
	const error = element.error;
	if (!error) return null;
	return `MediaError ${error.code}${error.message ? ` (${error.message})` : ""}`;
}

/** Records media events until an error or until `ms` elapsed */
function observeMedia(
	element: HTMLMediaElement,
	ms: number,
): Promise<MediaObservation> {
	return new Promise((resolve) => {
		const events: string[] = [];
		let done = false;
		const listener = (event: Event) => {
			if (!events.includes(event.type)) events.push(event.type);
			if (event.type === "error") finish();
		};
		const timer = setTimeout(finish, ms);
		for (const name of MEDIA_EVENTS) element.addEventListener(name, listener);
		function finish() {
			if (done) return;
			done = true;
			clearTimeout(timer);
			for (const name of MEDIA_EVENTS) {
				element.removeEventListener(name, listener);
			}
			resolve({
				events,
				error: mediaError(element),
				loaded: element.readyState >= HTMLMediaElement.HAVE_METADATA,
			});
		}
	});
}

export function describeMedia(observation: MediaObservation): string {
	const events =
		observation.events.length > 0 ? observation.events.join(", ") : "none";
	return `events: ${events}${observation.error ? `; ${observation.error}` : ""}`;
}

export type LoadResult = "loaded" | "error" | "timeout";

export function imageLoads(url: string, ms: number): Promise<LoadResult> {
	return new Promise((resolve) => {
		const image = new Image();
		const timer = setTimeout(() => finish("timeout"), ms);
		function finish(result: LoadResult) {
			clearTimeout(timer);
			image.onload = null;
			image.onerror = null;
			resolve(result);
		}
		image.onload = () => finish("loaded");
		image.onerror = () => finish("error");
		image.src = url;
	});
}

export async function fetchLoads(url: string): Promise<LoadResult> {
	try {
		await (await fetch(url, { cache: "no-store" })).arrayBuffer();
		return "loaded";
	} catch {
		return "error";
	}
}

/** A media element in `slot` with `src`, observed for `ms` */
export async function playMedia(
	slot: HTMLElement,
	tag: "audio" | "video",
	src: string,
	ms: number,
	prepare?: (element: HTMLMediaElement) => void,
): Promise<MediaObservation> {
	const element = document.createElement(tag);
	element.muted = true;
	element.preload = "auto";
	element.setAttribute("playsinline", "");
	prepare?.(element);
	slot.append(element);
	const observation = observeMedia(element, ms);
	element.src = src;
	element.load();
	element.play().catch(() => undefined);
	const result = await observation;
	element.pause();
	return result;
}
