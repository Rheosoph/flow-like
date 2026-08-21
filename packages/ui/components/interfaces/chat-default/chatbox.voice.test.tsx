import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createRef, forwardRef } from "react";
import { createRoot } from "react-dom/client";

// Keep this focused on ChatBox and the shared voice hooks. The production UI barrel
// pulls in Radix portals and unrelated browser integrations that happy-dom does not
// need in order to exercise recording, submission, or transcription.
mock.module("../../../lib", () => ({
	cn: (...classes: unknown[]) => classes.filter(Boolean).join(" "),
	humanFileSize: (bytes: number) => `${bytes} B`,
}));

mock.module("../../ui", () => ({
	Button: forwardRef<
		HTMLButtonElement,
		React.ButtonHTMLAttributes<HTMLButtonElement> & {
			size?: string;
			variant?: string;
		}
	>(({ size: _size, variant: _variant, ...props }, ref) => (
		<button ref={ref} {...props} />
	)),
	Popover: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
	PopoverContent: ({ children }: { children?: React.ReactNode }) => (
		<div>{children}</div>
	),
	PopoverTrigger: ({ children }: { children?: React.ReactNode }) => (
		<>{children}</>
	),
	Textarea: forwardRef<
		HTMLTextAreaElement,
		React.ComponentPropsWithoutRef<"textarea">
	>((props, ref) => <textarea ref={ref} {...props} />),
}));

mock.module("./chatbox/file-dialog", () => ({
	FileManagerDialog: () => null,
}));

afterAll(() => mock.restore());

type RecorderDataEvent = { data: Blob };
type SpeechResult = ArrayLike<{ transcript: string }> & { isFinal: boolean };
const chatBoxTestModule = "./chatbox.tsx?chatbox-voice-capture-test";

interface VoiceBrowser {
	window: Window;
	recorders: FakeMediaRecorder[];
	recognitions: FakeSpeechRecognition[];
}

class FakeMediaRecorder {
	static reportedMime = "audio/webm";
	static instances: FakeMediaRecorder[] = [];
	static isTypeSupported = () => true;

	state = "inactive";
	mimeType: string;
	requestedMime?: string;
	ondataavailable: ((event: RecorderDataEvent) => void) | null = null;
	onstart: (() => void) | null = null;
	onstop: (() => void) | null = null;

	constructor(_stream: unknown, options?: { mimeType?: string }) {
		this.requestedMime = options?.mimeType;
		this.mimeType = FakeMediaRecorder.reportedMime;
		FakeMediaRecorder.instances.push(this);
	}

	start(_timeslice?: number) {
		this.state = "recording";
		this.onstart?.();
	}

	stop() {
		if (this.state !== "recording") return;
		this.state = "inactive";
		this.ondataavailable?.({
			data: new Blob(["encoded audio"], { type: this.mimeType }),
		});
		this.onstop?.();
	}
}

class FakeSpeechRecognition {
	static instances: FakeSpeechRecognition[] = [];

	continuous = false;
	interimResults = false;
	lang = "";
	onresult:
		| ((event: {
				resultIndex: number;
				results: ArrayLike<SpeechResult>;
		  }) => void)
		| null = null;
	onerror: ((error: unknown) => void) | null = null;
	onend: (() => void) | null = null;

	constructor() {
		FakeSpeechRecognition.instances.push(this);
	}

	start() {}

	stop() {
		this.onend?.();
	}

	fail(error: unknown = new Error("speech unavailable")) {
		this.onerror?.(error);
	}

	emit(results: SpeechResult[], resultIndex = 0) {
		this.onresult?.({ resultIndex, results });
	}
}

function speechResult(transcript: string, isFinal: boolean): SpeechResult {
	return Object.assign([{ transcript }], { isFinal });
}

function installVoiceBrowser({
	reportedMime = "audio/webm",
	speechRecognition = false,
}: {
	reportedMime?: string;
	speechRecognition?: boolean;
} = {}): VoiceBrowser {
	const window = new Window();
	FakeMediaRecorder.reportedMime = reportedMime;
	FakeMediaRecorder.instances = [];
	FakeSpeechRecognition.instances = [];

	const stream = {
		getTracks: () => [{ stop: () => {} }],
	};
	Object.defineProperty(window.navigator, "mediaDevices", {
		configurable: true,
		value: {
			getUserMedia: async () => stream,
		},
	});

	class FakeAudioContext {
		resume() {
			return Promise.resolve();
		}

		close() {
			return Promise.resolve();
		}

		createMediaStreamSource() {
			return { connect: () => {} };
		}

		createAnalyser() {
			return { fftSize: 0 };
		}
	}

	Object.assign(window, {
		MediaRecorder: FakeMediaRecorder,
		// happy-dom 20 does not currently install these realm constructors under Bun,
		// but its selector engine uses them even for valid selectors.
		SyntaxError,
		TypeError,
		...(speechRecognition ? { SpeechRecognition: FakeSpeechRecognition } : {}),
	});
	Object.assign(globalThis, {
		AudioContext: FakeAudioContext,
		Blob: window.Blob,
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		File: window.File,
		HTMLElement: window.HTMLElement,
		MediaRecorder: FakeMediaRecorder,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		window,
		IS_REACT_ACT_ENVIRONMENT: true,
	});

	return {
		window,
		recorders: FakeMediaRecorder.instances,
		recognitions: FakeSpeechRecognition.instances,
	};
}

async function renderChatBox(
	window: Window,
	overrides: Partial<
		React.ComponentProps<typeof import("./chatbox")["ChatBox"]>
	> = {},
) {
	const { ChatBox } = await import(chatBoxTestModule);
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container);
	const chatBoxRef = createRef<import("./chatbox").ChatBoxRef>();

	await act(async () => {
		root.render(
			<ChatBox
				ref={chatBoxRef}
				onSendMessage={async () => {}}
				fileUpload={false}
				audioInput
				availableTools={[]}
				defaultActiveTools={[]}
				{...overrides}
			/>,
		);
	});

	return {
		container,
		ref: chatBoxRef,
		close: async () => {
			await act(async () => root.unmount());
			window.close();
		},
	};
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
	const element = Array.from(
		container.querySelectorAll<HTMLButtonElement>("button"),
	).find((candidate) => candidate.getAttribute("aria-label") === label);
	expect(element).toBeDefined();
	return element as HTMLButtonElement;
}

function hasButton(container: HTMLElement, label: string): boolean {
	return Array.from(container.querySelectorAll("button")).some(
		(candidate) => candidate.getAttribute("aria-label") === label,
	);
}

async function click(element: HTMLButtonElement) {
	await act(async () => {
		element.click();
		await Promise.resolve();
		await Promise.resolve();
	});
}

describe("ChatBox voice regressions", () => {
	test("submits an audio-only recording using the recorder-reported MP4 MIME and extension", async () => {
		const browser = installVoiceBrowser({
			reportedMime: "audio/mp4;codecs=mp4a.40.2",
		});
		const sends: unknown[][] = [];
		const view = await renderChatBox(browser.window, {
			voiceMode: "record",
			onSendMessage: async (...args) => {
				sends.push(args);
			},
		});

		try {
			await click(button(view.container, "Start audio recording"));
			expect(browser.recorders).toHaveLength(1);
			expect(browser.recorders[0].requestedMime).toBe("audio/webm");

			await act(async () => {
				button(view.container, "Stop audio recording").click();
				// ChatBox keeps a short tail after the stop click. Complete the fake
				// encoder directly so the test does not spend 700 ms on that timer.
				browser.recorders[0].stop();
				await Promise.resolve();
			});

			expect(view.container.textContent).toContain("Audio Recording");
			await click(button(view.container, "Send message"));

			expect(sends).toHaveLength(1);
			expect(sends[0][0]).toBe("");
			expect(sends[0][1]).toEqual([]);
			expect(sends[0][2]).toEqual([]);
			const audio = sends[0][3] as File;
			expect(audio).toBeInstanceOf(browser.window.File);
			expect(audio.type).toBe("audio/mp4;codecs=mp4a.40.2");
			expect(audio.name).toMatch(/^voice-\d+\.mp4$/);
		} finally {
			await view.close();
		}
	});

	test("falls back to recording when STT is selected but Web Speech is unavailable", async () => {
		const browser = installVoiceBrowser();
		const view = await renderChatBox(browser.window, { voiceMode: "stt" });

		try {
			expect(
				hasButton(view.container, "Start voice transcription"),
			).toBeFalse();
			await click(button(view.container, "Start audio recording"));
			expect(browser.recorders).toHaveLength(1);
			expect(hasButton(view.container, "Stop audio recording")).toBeTrue();

			await act(async () => {
				browser.recorders[0].stop();
				await Promise.resolve();
			});
		} finally {
			await view.close();
		}
	});

	test("creates only the latest recording after a rapid stop and restart during mic permission", async () => {
		const browser = installVoiceBrowser();
		let resolveMicrophone!: (stream: MediaStream) => void;
		const microphone = new Promise<MediaStream>((resolve) => {
			resolveMicrophone = resolve;
		});
		Object.defineProperty(browser.window.navigator, "mediaDevices", {
			configurable: true,
			value: { getUserMedia: () => microphone },
		});
		const view = await renderChatBox(browser.window, { voiceMode: "record" });

		try {
			await click(button(view.container, "Start audio recording"));
			await click(button(view.container, "Stop audio recording"));
			await click(button(view.container, "Start audio recording"));

			await act(async () => {
				resolveMicrophone({
					getTracks: () => [{ stop: () => {} }],
				} as unknown as MediaStream);
				await Promise.resolve();
				await Promise.resolve();
			});

			expect(browser.recorders).toHaveLength(1);
			expect(browser.recorders[0].state).toBe("recording");
		} finally {
			await view.close();
		}
	});

	test("does not submit text while an audio recording is still finishing", async () => {
		const browser = installVoiceBrowser();
		const sends: unknown[][] = [];
		const view = await renderChatBox(browser.window, {
			voiceMode: "record",
			onSendMessage: async (...args) => {
				sends.push(args);
			},
		});

		try {
			await act(async () => view.ref.current?.setInput("send with the audio"));
			await click(button(view.container, "Start audio recording"));
			await click(button(view.container, "Stop audio recording"));
			const send = button(view.container, "Send message");
			expect(send.disabled).toBeTrue();
			await click(send);
			expect(sends).toHaveLength(0);

			await act(async () => {
				browser.recorders[0].stop();
				await Promise.resolve();
			});
			expect(button(view.container, "Send message").disabled).toBeFalse();
		} finally {
			await view.close();
		}
	});

	test("falls back to the recording control after speech recognition fails", async () => {
		const browser = installVoiceBrowser({ speechRecognition: true });
		const view = await renderChatBox(browser.window, { voiceMode: "stt" });
		const originalConsoleError = console.error;
		console.error = mock(() => {});

		try {
			await click(button(view.container, "Start voice transcription"));
			await act(async () => {
				browser.recognitions[0].fail();
			});

			expect(view.container.textContent).toContain(
				"Speech recognition is unavailable",
			);
			expect(
				hasButton(view.container, "Start voice transcription"),
			).toBeFalse();
			await click(button(view.container, "Start audio recording"));
			expect(browser.recorders).toHaveLength(1);
		} finally {
			console.error = originalConsoleError;
			await view.close();
		}
	});

	test("replaces supported STT interim text instead of appending each revision", async () => {
		const browser = installVoiceBrowser({ speechRecognition: true });
		const view = await renderChatBox(browser.window, { voiceMode: "stt" });

		try {
			await click(button(view.container, "Start voice transcription"));
			expect(browser.recognitions).toHaveLength(1);
			const recognition = browser.recognitions[0];
			const textarea = view.container.querySelector("textarea");
			expect(textarea).not.toBeNull();

			await act(async () => {
				recognition.emit([speechResult("hel", false)]);
			});
			expect(textarea?.value).toBe("hel");

			await act(async () => {
				recognition.emit([speechResult("hello", false)]);
			});
			expect(textarea?.value).toBe("hello");
			expect(textarea?.value).not.toContain("hel hello");

			await act(async () => {
				recognition.emit([
					speechResult("hello ", true),
					speechResult("wor", false),
				]);
			});
			expect(textarea?.value).toBe("hello wor");

			await act(async () => {
				recognition.emit(
					[speechResult("hello ", true), speechResult("world", false)],
					1,
				);
			});
			expect(textarea?.value).toBe("hello world");
		} finally {
			await view.close();
		}
	});
});
