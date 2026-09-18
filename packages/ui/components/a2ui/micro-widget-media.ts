import type {
	FlwEnvelope,
	MediaPlayPayload,
	MediaResultPayload,
	WidgetMediaState,
	WidgetPublicMediaGrant,
} from "@flow-like/widget-sdk";
const ID = /^[a-zA-Z0-9_-]{1,128}$/;
export function isPublicMediaUrl(value: unknown): value is string {
	if (typeof value !== "string" || value.length > 4096) return false;
	try {
		const url = new URL(value);
		if (
			url.protocol !== "https:" ||
			url.username ||
			url.password ||
			url.hash ||
			(url.port && url.port !== "443")
		)
			return false;
		const host = url.hostname.toLowerCase().replace(/\.+$/, "");
		if (
			!host.includes(".") ||
			host.includes(":") ||
			["localhost", "local", "internal", "test"].some(
				(suffix) => host === suffix || host.endsWith(`.${suffix}`),
			)
		)
			return false;
		if (/^\d+\.\d+\.\d+\.\d+$/.test(host)) {
			const [a, b] = host.split(".").map(Number);
			if (
				a === 0 ||
				a === 10 ||
				a === 127 ||
				a >= 224 ||
				(a === 169 && b === 254) ||
				(a === 172 && b >= 16 && b <= 31) ||
				(a === 192 && b === 168) ||
				(a === 100 && b >= 64 && b <= 127) ||
				(a === 198 && (b === 18 || b === 19))
			)
				return false;
		}
		for (const key of url.searchParams.keys()) {
			const normalized = key.toLowerCase().replace(/[_-]/g, "");
			if (
				/token|signature|credential/.test(normalized) ||
				/^(auth|authorization|jwt|accesskey|apikey|key|session|sessionid|password|secret)$/.test(
					normalized,
				)
			)
				return false;
		}
		return true;
	} catch {
		return false;
	}
}
export function readPublicMediaGrants(
	value: unknown,
): WidgetPublicMediaGrant[] {
	if (!Array.isArray(value) || value.length > 1000) return [];
	const seen = new Set<string>();
	return value.flatMap((row) => {
		if (
			!row ||
			typeof row !== "object" ||
			typeof row.id !== "string" ||
			!ID.test(row.id) ||
			!isPublicMediaUrl(row.url) ||
			Object.keys(row).some((key) => key !== "id" && key !== "url") ||
			seen.has(row.id)
		)
			return [];
		seen.add(row.id);
		return [{ id: row.id, url: row.url }];
	});
}
export const PUBLIC_MEDIA_GRANTS_PROP = "publicMediaGrants";
export function publicWidgetProps(
	props: Record<string, unknown>,
): Record<string, unknown> {
	const { [PUBLIC_MEDIA_GRANTS_PROP]: _mediaGrants, ...publicProps } = props;
	return publicProps;
}
/** The node vets broadcaster URLs. Only approved IDs cross the iframe boundary. */
export function createMicroWidgetMedia(options: {
	grants: () => WidgetPublicMediaGrant[];
	result: (value: MediaResultPayload) => void;
	state: (value: WidgetMediaState) => void;
	audio?: () => HTMLAudioElement;
}) {
	let current:
		| { id: string; url: string; audio: HTMLAudioElement; generation: number }
		| undefined;
	let generation = 0;
	let disposed = false;
	const pending = new Set<string>();
	const finish = (requestId: string, ok: boolean) => {
		if (pending.delete(requestId)) options.result({ requestId, ok });
	};
	const stop = () => {
		generation++;
		if (current) {
			current.audio.pause();
			current.audio.removeAttribute("src");
			current.audio.load();
			current = undefined;
		}
		for (const id of pending) finish(id, false);
		options.state({ state: "idle" });
	};
	async function resume(requestId?: string) {
		const active = current;
		if (!active) return;
		options.state({ id: active.id, state: "loading" });
		try {
			await active.audio.play();
			if (current !== active || disposed) return;
			options.state({ id: active.id, state: "playing" });
			if (requestId) finish(requestId, true);
		} catch {
			if (current !== active || disposed) return;
			options.state({
				id: active.id,
				state: "error",
				error: "Playback unavailable; press Play in the host control",
			});
			if (requestId) finish(requestId, false);
		}
	}
	return {
		handle(envelope: FlwEnvelope) {
			if (disposed) return;
			if (envelope.type === "media:stop") {
				stop();
				return;
			}
			if (envelope.type === "media:pause") {
				if (current) {
					current.audio.pause();
					options.state({ id: current.id, state: "paused" });
				}
				return;
			}
			if (envelope.type !== "media:play") return;
			const request = envelope.payload as MediaPlayPayload;
			if (
				!request ||
				typeof request.requestId !== "string" ||
				!ID.test(request.requestId) ||
				pending.has(request.requestId)
			)
				return;
			const grant = options.grants().find((row) => row.id === request.mediaId);
			if (!grant || !isPublicMediaUrl(grant.url)) {
				options.result({ requestId: request.requestId, ok: false });
				return;
			}
			stop();
			pending.add(request.requestId);
			const audio = options.audio?.() ?? new Audio();
			audio.crossOrigin = "anonymous";
			audio.preload = "none";
			const active = {
				id: grant.id,
				url: grant.url,
				audio,
				generation: ++generation,
			};
			current = active;
			audio.addEventListener("error", () => {
				if (current !== active) return;
				options.state({
					id: active.id,
					state: "error",
					error: "Broadcaster stream unavailable",
				});
				finish(request.requestId, false);
			});
			audio.addEventListener("ended", () => {
				if (current === active) stop();
			});
			audio.src = grant.url;
			void resume(request.requestId);
		},
		resume: () => resume(),
		pause() {
			if (current) {
				current.audio.pause();
				options.state({ id: current.id, state: "paused" });
			}
		},
		stop,
		revoke() {
			if (
				current &&
				!options
					.grants()
					.some((row) => row.id === current!.id && row.url === current!.url)
			)
				stop();
		},
		dispose() {
			disposed = true;
			stop();
		},
	};
}
