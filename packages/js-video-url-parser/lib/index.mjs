// src/index.ts
var MAX_URL_LENGTH = 2048;
var YOUTUBE_ID = /^[A-Za-z0-9_-]{11}$/;
var DAILYMOTION_ID = /^[A-Za-z0-9]+$/;
var VIMEO_ID = /^\d{1,32}$/;
var YOUKU_ID = /^[A-Za-z0-9=_-]{1,128}$/;
var COUB_ID = /^[A-Za-z0-9_-]{1,128}$/;
function normalizeHost(hostname) {
	return hostname.toLowerCase().replace(/\.$/, "");
}
function isHost(hostname, domain) {
	return hostname === domain || hostname.endsWith(`.${domain}`);
}
function segments(url) {
	return url.pathname.split("/").filter(Boolean);
}
function validId(value, pattern) {
	if (typeof value !== "string") return;
	return pattern.test(value) ? value : undefined;
}
function videoInfo(provider, id) {
	return { provider, id, mediaType: "video" };
}
function parseYoutube(url, host, path) {
	let id;
	if (host === "youtu.be") {
		id = validId(path[0], YOUTUBE_ID);
	} else if (
		isHost(host, "youtube.com") ||
		isHost(host, "youtube-nocookie.com")
	) {
		id = validId(url.searchParams.get("v") ?? undefined, YOUTUBE_ID);
		if (!id) {
			const [prefix, candidate] = path;
			if (["embed", "shorts", "live", "v", "vi", "videos"].includes(prefix)) {
				id = validId(candidate, YOUTUBE_ID);
			}
		}
	}
	return id ? videoInfo("youtube", id) : undefined;
}
function parseVimeo(host, path) {
	if (!isHost(host, "vimeo.com")) return;
	const candidate =
		host === "player.vimeo.com" && path[0] === "video"
			? path[1]
			: [...path].reverse().find((segment) => VIMEO_ID.test(segment));
	const id = validId(candidate, VIMEO_ID);
	return id ? videoInfo("vimeo", id) : undefined;
}
function parseDailymotion(host, path) {
	let candidate;
	if (host === "dai.ly") {
		candidate = path[0];
	} else if (isHost(host, "dailymotion.com")) {
		const videoIndex = path.indexOf("video");
		candidate = videoIndex === -1 ? undefined : path[videoIndex + 1];
	}
	const id = validId(candidate?.split("_")[0], DAILYMOTION_ID);
	return id ? videoInfo("dailymotion", id) : undefined;
}
function urlPath(path) {
	return `/${path.join("/")}`;
}
function parseYouku(host, path) {
	if (!isHost(host, "youku.com")) return;
	let candidate;
	if (host === "player.youku.com" && path[0] === "embed") {
		candidate = path[1];
	} else {
		const match = urlPath(path).match(
			/(?:^|\/)v_show\/id_([A-Za-z0-9=_-]{1,128})(?:\.html)?$/,
		);
		candidate = match?.[1];
	}
	const id = validId(candidate, YOUKU_ID);
	return id ? videoInfo("youku", id) : undefined;
}
function parseCoub(host, path) {
	if (!isHost(host, "coub.com")) return;
	const [prefix, candidate] = path;
	const id = ["view", "embed"].includes(prefix)
		? validId(candidate, COUB_ID)
		: undefined;
	return id ? videoInfo("coub", id) : undefined;
}
function parse(input) {
	if (typeof input !== "string" || input.length > MAX_URL_LENGTH) return;
	let url;
	try {
		url = new URL(input);
	} catch {
		return;
	}
	if (url.protocol !== "http:" && url.protocol !== "https:") return;
	const host = normalizeHost(url.hostname);
	const path = segments(url);
	return (
		parseYoutube(url, host, path) ??
		parseVimeo(host, path) ??
		parseDailymotion(host, path) ??
		parseYouku(host, path) ??
		parseCoub(host, path)
	);
}
var parser = { parse };
var src_default = parser;
export { parse, src_default as default };
