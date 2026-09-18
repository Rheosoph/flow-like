import { describe, expect, test } from "bun:test";
import {
	createEnvelope,
	type MediaResultPayload,
	type WidgetMediaState,
} from "@flow-like/widget-sdk";
import {
	createMicroWidgetMedia,
	isPublicMediaUrl,
	publicWidgetProps,
	readPublicMediaGrants,
} from "./micro-widget-media";
class FakeAudio extends EventTarget {
	src = "";
	crossOrigin = "";
	preload = "";
	paused = false;
	deny = false;
	async play() {
		if (this.deny) throw new Error("provider-secret");
		this.paused = false;
	}
	pause() {
		this.paused = true;
	}
	load() {}
	removeAttribute(name: string) {
		if (name === "src") this.src = "";
	}
}
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const grant = { id: "radio-1", url: "https://broadcast.example.org/live.mp3" };
describe("public audio authority", () => {
	test("private/credential URLs cannot become grants", () => {
		for (const url of [
			"http://radio.example.org/",
			"https://user:secret@radio.example.org/",
			"https://127.0.0.1/",
			"https://0x7f000001/",
			"https://10.0.0.1/",
			"https://192.168.1.1/",
			"https://[::1]/",
			"https://a.local/",
			"https://a.local./",
			"https://localhost./",
			"https://a.internal/",
			"https://radio.example.org/?access_token=secret",
			"https://radio.example.org/?X-Amz-Credential=secret",
			"https://radio.example.org/?api_key=secret",
		]) {
			expect(isPublicMediaUrl(url)).toBe(false);
			expect(readPublicMediaGrants([{ ...grant, url }])).toEqual([]);
		}
		expect(
			readPublicMediaGrants([
				grant,
				{ ...grant, headers: { Authorization: "secret" } },
			]),
		).toEqual([grant]);
	});
	test("host-only grants never cross props", () => {
		expect(
			publicWidgetProps({
				publicMediaGrants: [grant],
				data: 1,
			}),
		).toEqual({ data: 1 });
	});
	test("plays approved id without publishing URL and revokes on grant removal", async () => {
		const audio = new FakeAudio();
		const results: MediaResultPayload[] = [];
		const states: WidgetMediaState[] = [];
		let grants = [grant];
		const media = createMicroWidgetMedia({
			grants: () => grants,
			result: (r) => results.push(r),
			state: (s) => states.push(s),
			audio: () => audio as unknown as HTMLAudioElement,
		});
		media.handle(
			createEnvelope(
				"media:play",
				{ requestId: "one", mediaId: grant.id },
				"n",
				"i",
			),
		);
		await tick();
		expect(audio.src).toBe(grant.url);
		expect(audio.crossOrigin).toBe("anonymous");
		expect(results).toEqual([{ requestId: "one", ok: true }]);
		expect(JSON.stringify(states)).not.toContain(grant.url);
		media.pause();
		expect(states.at(-1)?.state).toBe("paused");
		await media.resume();
		expect(states.at(-1)?.state).toBe("playing");
		grants = [];
		media.revoke();
		expect(audio.src).toBe("");
		expect(states.at(-1)?.state).toBe("idle");
		media.dispose();
	});
	test("unknown ids never construct audio; browser denial is recoverable in host", async () => {
		const audio = new FakeAudio();
		audio.deny = true;
		let creates = 0;
		const states: WidgetMediaState[] = [];
		const results: MediaResultPayload[] = [];
		const media = createMicroWidgetMedia({
			grants: () => [grant],
			state: (s) => states.push(s),
			result: (r) => results.push(r),
			audio: () => {
				creates++;
				return audio as unknown as HTMLAudioElement;
			},
		});
		media.handle(
			createEnvelope(
				"media:play",
				{ requestId: "bad", mediaId: "https://attacker.example" },
				"n",
				"i",
			),
		);
		expect(creates).toBe(0);
		media.handle(
			createEnvelope(
				"media:play",
				{ requestId: "good", mediaId: grant.id },
				"n",
				"i",
			),
		);
		await tick();
		expect(results.at(-1)).toEqual({ requestId: "good", ok: false });
		expect(JSON.stringify(states)).not.toContain("provider-secret");
		audio.deny = false;
		await media.resume();
		expect(states.at(-1)?.state).toBe("playing");
		media.dispose();
		expect(audio.src).toBe("");
	});
});
