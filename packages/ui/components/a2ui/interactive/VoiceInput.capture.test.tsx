import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { useSpeechRecognition } from "../../voice/use-speech-recognition";
import { useVoiceRecorder } from "../../voice/use-voice-recorder";
import type { BoundValue, VoiceInputComponent } from "../types";

const resolveBoundValue = (value: BoundValue) => {
	if ("literalString" in value) return value.literalString;
	if ("literalNumber" in value) return value.literalNumber;
	if ("literalBool" in value) return value.literalBool;
	if ("literalJson" in value) return JSON.parse(value.literalJson);
	if ("literalOptions" in value) return value.literalOptions;
	return undefined;
};

const triggerEvent = mock(async () => {});

mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({
		helperState: {
			fileToUrl: mock(async () => "temporary://voice"),
		},
	}),
}));

mock.module("../ActionHandler", () => ({
	useActionContext: () => ({}),
	useComponentEventTrigger: () => triggerEvent,
	useIsComponentTriggering: () => false,
	useOnAction: () => undefined,
}));

mock.module("../DataContext", () => ({
	useData: () => ({
		resolve: resolveBoundValue,
		setByPath: mock(() => {}),
	}),
}));

mock.module("../../voice", () => ({
	AudioPlayback: () => null,
	VOICE_DEFAULT_COLOR: "#7c3aed",
	VOICE_DEFAULT_RECORDING_COLOR: "#ef4444",
	getVoiceVisualizer: () => () => <div data-testid="voice-visualizer" />,
	useSpeakerActivity: () => undefined,
	useSpeechRecognition,
	useVoiceRecorder,
}));

afterAll(() => mock.restore());

class FakeMediaRecorder {
	static isTypeSupported = mock(() => true);
	state: "inactive" | "recording" = "inactive";
	mimeType = "audio/webm";
	ondataavailable: ((event: { data: Blob }) => void) | null = null;
	onstart: (() => void) | null = null;
	onstop: (() => void) | null = null;

	start() {
		this.state = "recording";
		this.onstart?.();
	}

	stop() {
		this.state = "inactive";
		this.onstop?.();
	}
}

class FakeSpeechRecognition {
	static instances: FakeSpeechRecognition[] = [];
	continuous = true;
	interimResults = true;
	lang = "";
	onresult: ((event: never) => void) | null = null;
	onerror: ((event: { error: string }) => void) | null = null;
	onend: (() => void) | null = null;

	constructor() {
		FakeSpeechRecognition.instances.push(this);
	}

	start() {}
	stop() {}
	abort() {}

	fail(error: string) {
		this.onerror?.({ error });
	}

	finishWithoutSpeech() {
		this.onend?.();
	}
}

let browserWindow: Window;
let root: Root | null;
const voiceInputTestModule = "./VoiceInput.tsx?voice-input-capture-test";

const setBrowserProperty = (
	target: object,
	key: PropertyKey,
	value: unknown,
) => {
	Object.defineProperty(target, key, {
		configurable: true,
		writable: true,
		value,
	});
};

const voiceComponent = (
	mode: "record" | "stt" = "record",
	invoke: "manual" | "hold" = "manual",
	disabled = false,
): VoiceInputComponent => ({
	type: "voiceInput",
	value: { path: "$.voice" },
	mode: { literalString: mode },
	invoke: { literalString: invoke },
	variant: { literalString: "conservative" },
	...(disabled ? { disabled: { literalBool: true } } : {}),
});

async function updateVoiceInput(
	mode: "record" | "stt" = "record",
	invoke: "manual" | "hold" = "manual",
	disabled = false,
) {
	const { A2UIVoiceInput } = await import(voiceInputTestModule);
	await act(async () => {
		root?.render(
			<A2UIVoiceInput
				component={voiceComponent(mode, invoke, disabled)}
				componentId="voice-input"
				surfaceId="test-surface"
			/>,
		);
	});
}

async function renderVoiceInput(
	mode: "record" | "stt" = "record",
	invoke: "manual" | "hold" = "manual",
) {
	const container = browserWindow.document.createElement("div");
	browserWindow.document.body.append(container);
	root = createRoot(container as unknown as Element);
	await updateVoiceInput(mode, invoke);
}

async function clickRecorder() {
	const button = browserWindow.document.querySelector("button");
	expect(button).not.toBeNull();
	await act(async () => {
		button?.dispatchEvent(
			new browserWindow.MouseEvent("click", { bubbles: true }),
		);
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function holdAndReleaseRecorder() {
	const button = browserWindow.document.querySelector("button");
	expect(button).not.toBeNull();
	await act(async () => {
		for (const type of ["pointerdown", "pointerup"]) {
			const event = new browserWindow.Event(type, { bubbles: true });
			Object.defineProperty(event, "pointerId", { value: 1 });
			button?.dispatchEvent(event);
		}
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

beforeEach(() => {
	browserWindow = new Window({ url: "https://app.flow-like.test" });
	root = null;
	triggerEvent.mockClear();
	FakeMediaRecorder.isTypeSupported.mockClear();
	FakeSpeechRecognition.instances = [];
	Object.assign(browserWindow, { SyntaxError, TypeError });

	Object.assign(globalThis, {
		Blob: browserWindow.Blob,
		document: browserWindow.document,
		DOMException: browserWindow.DOMException,
		Element: browserWindow.Element,
		Event: browserWindow.Event,
		File: browserWindow.File,
		HTMLElement: browserWindow.HTMLElement,
		MouseEvent: browserWindow.MouseEvent,
		Node: browserWindow.Node,
		navigator: browserWindow.navigator,
		requestAnimationFrame:
			browserWindow.requestAnimationFrame.bind(browserWindow),
		cancelAnimationFrame:
			browserWindow.cancelAnimationFrame.bind(browserWindow),
		window: browserWindow,
		IS_REACT_ACT_ENVIRONMENT: true,
	});
});

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	browserWindow.close();
});

describe("A2UI VoiceInput browser capture", () => {
	test("shows a permission error and remains retryable when the browser blocks the microphone", async () => {
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		const getUserMedia = mock(async () => {
			throw new browserWindow.DOMException(
				"Permission denied",
				"NotAllowedError",
			);
		});
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia,
		});
		const originalConsoleError = console.error;
		console.error = mock(() => {});

		try {
			await renderVoiceInput();
			await clickRecorder();

			const alert = browserWindow.document.querySelector('[role="alert"]');
			expect(alert?.textContent).toContain("Microphone access was blocked");
			expect(browserWindow.document.body.textContent).not.toContain(
				"Starting…",
			);
			expect(
				browserWindow.document
					.querySelector("button")
					?.hasAttribute("disabled"),
			).toBe(false);
			expect(getUserMedia).toHaveBeenCalledTimes(1);

			await clickRecorder();
			expect(getUserMedia).toHaveBeenCalledTimes(2);
		} finally {
			console.error = originalConsoleError;
		}
	});

	test("explains that an insecure browser context cannot record", async () => {
		setBrowserProperty(browserWindow, "isSecureContext", false);
		setBrowserProperty(browserWindow.navigator, "mediaDevices", undefined);
		setBrowserProperty(browserWindow, "MediaRecorder", undefined);

		await renderVoiceInput();

		const alert = browserWindow.document.querySelector('[role="alert"]');
		expect(alert?.textContent).toBe(
			"Voice recording requires HTTPS or localhost.",
		);
		expect(
			browserWindow.document.querySelector("button")?.hasAttribute("disabled"),
		).toBe(true);
	});

	test("keeps STT retryable when no speech was detected", async () => {
		setBrowserProperty(
			browserWindow,
			"SpeechRecognition",
			FakeSpeechRecognition,
		);
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		const getUserMedia = mock(async () => {
			throw new Error("audio recording should not be used");
		});
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia,
		});

		await renderVoiceInput("stt");
		await clickRecorder();
		expect(FakeSpeechRecognition.instances).toHaveLength(1);

		await act(async () => {
			FakeSpeechRecognition.instances[0].finishWithoutSpeech();
		});
		expect(
			browserWindow.document.querySelector('[role="alert"]')?.textContent,
		).toBe("No speech was detected. Try again.");

		await clickRecorder();
		expect(FakeSpeechRecognition.instances).toHaveLength(2);
		expect(getUserMedia).not.toHaveBeenCalled();
	});

	test("falls back to audio recording when browser speech recognition fails", async () => {
		setBrowserProperty(
			browserWindow,
			"SpeechRecognition",
			FakeSpeechRecognition,
		);
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		const stopTrack = mock(() => {});
		const getUserMedia = mock(async () => ({
			getTracks: () => [{ stop: stopTrack }],
		}));
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia,
		});
		const originalConsoleError = console.error;
		console.error = mock(() => {});

		try {
			await renderVoiceInput("stt");
			await clickRecorder();
			await act(async () => {
				FakeSpeechRecognition.instances[0].fail("network");
			});

			expect(
				browserWindow.document.querySelector('[role="alert"]')?.textContent,
			).toContain("Tap again to record audio instead");

			await clickRecorder();
			expect(getUserMedia).toHaveBeenCalledTimes(1);
		} finally {
			console.error = originalConsoleError;
		}
	});

	test("explains when a first hold grants permission after the user releases", async () => {
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		let resolveMicrophone: ((stream: MediaStream) => void) | undefined;
		const microphone = new Promise<MediaStream>((resolve) => {
			resolveMicrophone = resolve;
		});
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia: mock(() => microphone),
		});

		await renderVoiceInput("record", "hold");
		await holdAndReleaseRecorder();
		await act(async () => {
			resolveMicrophone?.({
				getTracks: () => [{ stop: mock(() => {}) }],
			} as unknown as MediaStream);
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(
			browserWindow.document.querySelector('[role="alert"]')?.textContent,
		).toBe("Microphone is ready. Hold again to record.");
	});

	test("preserves the real permission error when denial follows a released hold", async () => {
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		let rejectMicrophone: ((error: unknown) => void) | undefined;
		const microphone = new Promise<MediaStream>((_resolve, reject) => {
			rejectMicrophone = reject;
		});
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia: mock(() => microphone),
		});
		const originalConsoleError = console.error;
		console.error = mock(() => {});

		try {
			await renderVoiceInput("record", "hold");
			await holdAndReleaseRecorder();
			await act(async () => {
				rejectMicrophone?.(
					new browserWindow.DOMException(
						"Permission denied",
						"NotAllowedError",
					),
				);
				await new Promise((resolve) => setTimeout(resolve, 0));
			});

			expect(
				browserWindow.document.querySelector('[role="alert"]')?.textContent,
			).toContain("Microphone access was blocked");
		} finally {
			console.error = originalConsoleError;
		}
	});

	test("does not restore hold setup feedback after capture is cancelled", async () => {
		setBrowserProperty(browserWindow, "MediaRecorder", FakeMediaRecorder);
		setBrowserProperty(globalThis, "MediaRecorder", FakeMediaRecorder);
		let resolveMicrophone: ((stream: MediaStream) => void) | undefined;
		const microphone = new Promise<MediaStream>((resolve) => {
			resolveMicrophone = resolve;
		});
		setBrowserProperty(browserWindow.navigator, "mediaDevices", {
			getUserMedia: mock(() => microphone),
		});

		await renderVoiceInput("record", "hold");
		await holdAndReleaseRecorder();
		await updateVoiceInput("record", "hold", true);
		await act(async () => {
			resolveMicrophone?.({
				getTracks: () => [{ stop: mock(() => {}) }],
			} as unknown as MediaStream);
			await new Promise((resolve) => setTimeout(resolve, 0));
		});

		expect(browserWindow.document.querySelector('[role="alert"]')).toBeNull();
	});
});
