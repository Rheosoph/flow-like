// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, test, vi } from "vitest";

const { setDraft } = vi.hoisted(() => ({ setDraft: vi.fn() }));
vi.mock("@flow-like/flow-like-ui/state/global-chat/global-chat-store", () => ({
	useGlobalChatStore: { getState: () => ({ setDraft }) },
}));
import { NativeVoiceInput } from "../../components/native-voice-input";

let root: Root | undefined;
let host: HTMLDivElement;
afterEach(async () => {
	if (root) await act(async () => root!.unmount());
	root = undefined;
	host?.remove();
	vi.unstubAllGlobals();
	vi.clearAllMocks();
	delete (window as unknown as { SpeechRecognition?: unknown })
		.SpeechRecognition;
});
async function render() {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
	const onClose = vi.fn();
	await act(async () =>
		root!.render(
			createElement(NativeVoiceInput, { scope: "account-a", onClose }),
		),
	);
	return onClose;
}
const button = (text: string) =>
	Array.from(host.querySelectorAll("button")).find(
		(item) => item.textContent?.trim() === text,
	)!;

describe("native voice entry", () => {
	test("requires explicit capture, stops on blur, and sends a reviewed transcript in its original scope", async () => {
		let recognition: Recognition;
		class Recognition {
			start = vi.fn();
			stop = vi.fn(() => this.onend?.());
			abort = vi.fn();
			onend: (() => void) | null = null;
			onresult: ((event: unknown) => void) | null = null;
			constructor() {
				recognition = this;
			}
		}
		(window as unknown as { SpeechRecognition: unknown }).SpeechRecognition =
			Recognition;
		const close = await render();
		expect(recognition!).toBeUndefined();
		await act(async () => button("Start dictation").click());
		expect(recognition!.start).toHaveBeenCalledOnce();
		await act(async () =>
			recognition!.onresult?.({
				resultIndex: 0,
				results: [
					Object.assign([{ transcript: "Translate this" }], { isFinal: true }),
				],
			}),
		);
		expect((host.querySelector("textarea") as HTMLTextAreaElement).value).toBe(
			"Translate this",
		);
		expect(setDraft).not.toHaveBeenCalled();
		await act(async () => window.dispatchEvent(new Event("blur")));
		expect(recognition!.abort).toHaveBeenCalledOnce();
		expect(recognition!.onresult).toBeNull();
		await act(async () => button("Send").click());
		expect(setDraft).toHaveBeenCalledWith({
			prompt: "Translate this",
			nativeScope: "account-a",
		});
		expect(close).toHaveBeenCalledOnce();
	});
	test("shows keyboard dictation fallback when recognition is unavailable", async () => {
		await render();
		expect(button("Start dictation").disabled).toBe(true);
		expect(host.textContent).toContain("keyboard's dictation button");
		expect(
			(host.querySelector("textarea") as HTMLTextAreaElement).disabled,
		).toBe(false);
	});
});
