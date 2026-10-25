import { describe, expect, test } from "bun:test";
import {
	type FileResultPayload,
	MAX_WIDGET_UPLOAD_BYTES,
	WIDGET_FILE_HANDLE_PATTERN,
	type WidgetFile,
	createEnvelope,
	isWidgetFile,
} from "@flow-like/widget-sdk";
import type { IHelperState } from "../../state/backend-state/helper-state";
import {
	MAX_WIDGET_UPLOADS_PER_MINUTE,
	type WidgetFileRecord,
	type WidgetUploadFunction,
	type WidgetUploadOutcome,
	type WidgetUploadTarget,
	createMicroWidgetUpload,
	mintWidgetFileHandle,
	resolveWidgetFileHandles,
	uploadWidgetFile,
} from "./micro-widget-upload";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

const FLOW_PATH = {
	path: "tmp/user/sub-1/apps/app-1/upload/a.pdf",
	store_ref: "__flow_like_http_request_files",
};
const SIGNED_URL = "https://bucket.example.com/tmp/a.pdf?X-Amz-Signature=abc";

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

function harness(
	options: {
		enabled?: () => boolean;
		upload?: WidgetUploadFunction;
		now?: () => number;
	} = {},
) {
	const results: FileResultPayload[] = [];
	const uploads: { file: File; signal: AbortSignal }[] = [];
	const controller = createMicroWidgetUpload({
		enabled: options.enabled ?? (() => true),
		upload:
			options.upload ??
			(async (file, signal) => {
				uploads.push({ file, signal });
				return { ok: true, url: SIGNED_URL, flowPath: FLOW_PATH };
			}),
		result: (payload) => results.push(payload),
		now: options.now,
	});
	const send = (
		type: "file:upload" | "file:cancel",
		payload: Record<string, unknown>,
	) => controller.handle(createEnvelope(type, payload as never, "n", "i"));
	const request = (
		requestId: string,
		file: unknown = new File(["hello"], "a.pdf", { type: "application/pdf" }),
		extra: Record<string, unknown> = {},
	) =>
		send("file:upload", {
			requestId,
			file,
			name: "a.pdf",
			mimeType: "application/pdf",
			...extra,
		});
	return { controller, results, uploads, send, request };
}

function oversized(): Blob {
	const blob = new Blob(["x"]);
	Object.defineProperty(blob, "size", { value: MAX_WIDGET_UPLOAD_BYTES + 1 });
	return blob;
}

describe("createMicroWidgetUpload", () => {
	test("an ungranted widget never uploads", async () => {
		const { results, uploads, request } = harness({ enabled: () => false });
		request("a");
		await tick();
		expect(uploads).toHaveLength(0);
		expect(results).toEqual([
			{ requestId: "a", ok: false, error: "not-granted" },
		]);
	});

	test("ignores malformed request ids and non-upload envelopes", async () => {
		const { controller, results, uploads, request } = harness();
		for (const requestId of ["", "a b", "x".repeat(129), "../a"])
			request(requestId);
		controller.handle(createEnvelope("ready", {} as never, "n", "i") as never);
		await tick();
		expect(results).toEqual([]);
		expect(uploads).toEqual([]);
	});

	test("only a Blob is uploaded", async () => {
		const { results, uploads, request } = harness();
		request("a", "aGVsbG8=");
		request("b", { size: 1 });
		await tick();
		expect(uploads).toHaveLength(0);
		expect(results.map((result) => result.error)).toEqual(["failed", "failed"]);
	});

	test("refuses files above the size cap", async () => {
		const { results, uploads, request } = harness();
		request("a", oversized());
		await tick();
		expect(uploads).toHaveLength(0);
		expect(results).toEqual([
			{ requestId: "a", ok: false, error: "too-large" },
		]);
	});

	test("answers with a handle that carries no path or URL", async () => {
		const { results, uploads, request } = harness();
		request("a");
		await tick();
		expect(uploads).toHaveLength(1);
		expect(uploads[0].file.name).toBe("a.pdf");
		expect(uploads[0].file.type).toBe("application/pdf");
		const [result] = results;
		expect(result.ok).toBe(true);
		expect(isWidgetFile(result.file)).toBe(true);
		expect(result.file?.$flwFile).toMatch(/^flwf_[A-Za-z0-9_-]{24}$/);
		expect(result.file).toEqual({
			$flwFile: result.file?.$flwFile as string,
			name: "a.pdf",
			size: 5,
			mimeType: "application/pdf",
		});
		expect(JSON.stringify(result)).not.toContain("tmp/");
		expect(JSON.stringify(result)).not.toContain("https:");
	});

	test("names and types are bounded and a bad type falls back", async () => {
		const { results, uploads, request } = harness();
		request("a", new Blob(["x"]), {
			name: "n".repeat(300),
			mimeType: "text/html\r\nX-Injected: 1",
		});
		request("b", new Blob(["x"], { type: "image/png" }), {
			name: "",
			mimeType: 7,
		});
		await tick();
		expect(uploads.map(({ file }) => [file.name.length, file.type])).toEqual([
			[255, "application/octet-stream"],
			[4, "image/png"],
		]);
		expect(results.map((result) => result.file?.name)).toEqual([
			"n".repeat(255),
			"file",
		]);
	});

	test("at most four uploads run at once", async () => {
		const gates: ReturnType<typeof deferred<WidgetUploadOutcome>>[] = [];
		const { results, request } = harness({
			upload: () => {
				const gate = deferred<WidgetUploadOutcome>();
				gates.push(gate);
				return gate.promise;
			},
		});
		for (const id of ["a", "b", "c", "d", "e"]) request(id);
		expect(gates).toHaveLength(4);
		expect(results).toEqual([{ requestId: "e", ok: false, error: "busy" }]);
		gates[0].resolve({ ok: true, url: SIGNED_URL });
		await tick();
		request("f");
		expect(gates).toHaveLength(5);
	});

	test("a duplicate in-flight request id is ignored", async () => {
		const gate = deferred<WidgetUploadOutcome>();
		let calls = 0;
		const { results, request } = harness({
			upload: () => {
				calls++;
				return gate.promise;
			},
		});
		request("a");
		request("a");
		expect(calls).toBe(1);
		gate.resolve({ ok: true, url: SIGNED_URL });
		await tick();
		expect(results).toHaveLength(1);
	});

	test(`allows ${MAX_WIDGET_UPLOADS_PER_MINUTE} uploads per minute`, async () => {
		let now = 1_000_000;
		const { results, uploads, request } = harness({ now: () => now });
		for (let index = 0; index < MAX_WIDGET_UPLOADS_PER_MINUTE; index++) {
			request(`a${index}`);
			await tick();
		}
		request("over");
		await tick();
		expect(uploads).toHaveLength(MAX_WIDGET_UPLOADS_PER_MINUTE);
		expect(results.at(-1)).toEqual({
			requestId: "over",
			ok: false,
			error: "busy",
		});
		now += 3_000;
		request("later");
		await tick();
		expect(uploads).toHaveLength(MAX_WIDGET_UPLOADS_PER_MINUTE + 1);
	});

	test("a cancelled upload is aborted and its late result ignored", async () => {
		const gate = deferred<WidgetUploadOutcome>();
		let signal: AbortSignal | undefined;
		const { results, request, send } = harness({
			upload: (_file, abort) => {
				signal = abort;
				return gate.promise;
			},
		});
		request("a");
		send("file:cancel", { requestId: "a" });
		expect(signal?.aborted).toBe(true);
		gate.resolve({ ok: true, url: SIGNED_URL, flowPath: FLOW_PATH });
		await tick();
		expect(results).toEqual([]);
	});

	test("failures reach the widget as generic codes only", async () => {
		const outcomes: unknown[] = [
			Promise.reject(new Error("403 Forbidden: user sub-1 is not a member")),
			{ ok: false, error: "sign-in-required" },
			{ ok: false, error: "unavailable" },
			{ ok: false, error: "AccessDenied: s3://bucket/apps/app-1" },
		];
		const { results, request } = harness({
			upload: async () => (await outcomes.shift()) as WidgetUploadOutcome,
		});
		for (const id of ["a", "b", "c", "d"]) request(id);
		await tick();
		expect(results).toEqual([
			{ requestId: "a", ok: false, error: "failed" },
			{ requestId: "b", ok: false, error: "sign-in-required" },
			{ requestId: "c", ok: false, error: "unavailable" },
			{ requestId: "d", ok: false, error: "failed" },
		]);
	});

	test("revoke cancels running uploads with the given code and forgets handles", async () => {
		const gate = deferred<WidgetUploadOutcome>();
		let calls = 0;
		const { controller, results, request } = harness({
			upload: async () => {
				calls++;
				if (calls === 1) return { ok: true, url: SIGNED_URL };
				return gate.promise;
			},
		});
		request("a");
		await tick();
		const handle = results[0].file as WidgetFile;
		expect(controller.resolve({ file: handle }).ok).toBe(true);
		request("b");
		controller.revoke();
		gate.resolve({ ok: true, url: SIGNED_URL });
		await tick();
		expect(results.slice(1)).toEqual([
			{ requestId: "b", ok: false, error: "not-granted" },
		]);
		expect(controller.resolve({ file: handle })).toEqual({ ok: false });
		request("c");
		controller.revoke("failed");
		expect(results.at(-1)).toEqual({
			requestId: "c",
			ok: false,
			error: "failed",
		});
	});

	test("dispose aborts silently and stops answering", async () => {
		const gate = deferred<WidgetUploadOutcome>();
		let signal: AbortSignal | undefined;
		const { controller, results, request } = harness({
			upload: (_file, abort) => {
				signal = abort;
				return gate.promise;
			},
		});
		request("a");
		controller.dispose();
		expect(signal?.aborted).toBe(true);
		gate.resolve({ ok: true, url: SIGNED_URL });
		request("b");
		controller.revoke();
		await tick();
		expect(results).toEqual([]);
	});
});

describe("mintWidgetFileHandle", () => {
	test("mints flwf_ plus 24 random base64url characters", () => {
		const pattern = new RegExp(WIDGET_FILE_HANDLE_PATTERN);
		const handles = new Set(
			Array.from({ length: 200 }, () => mintWidgetFileHandle()),
		);
		expect(handles.size).toBe(200);
		for (const handle of handles) {
			expect(handle).toMatch(/^flwf_[A-Za-z0-9_-]{24}$/);
			expect(pattern.test(handle)).toBe(true);
		}
	});
});

describe("resolveWidgetFileHandles", () => {
	const minted: WidgetFile = {
		$flwFile: "flwf_AAAAAAAAAAAAAAAAAAAAAAAA",
		name: "a.pdf",
		size: 5,
		mimeType: "application/pdf",
	};
	const resolved = {
		name: "a.pdf",
		size: 5,
		type: "application/pdf",
		url: SIGNED_URL,
		flowPath: FLOW_PATH,
	};
	const registry = new Map<string, WidgetFileRecord>([
		[minted.$flwFile, { handle: minted, file: resolved }],
	]);

	test("keeps a payload without handles as it is", () => {
		const payload = { a: [1, { b: "c" }], d: null };
		const result = resolveWidgetFileHandles(payload, registry);
		expect(result).toEqual({ ok: true, payload });
		expect(result.ok && result.payload).toBe(payload);
		expect(resolveWidgetFileHandles("text", registry)).toEqual({
			ok: true,
			payload: "text",
		});
	});

	test("swaps handles at any depth, also inside arrays", () => {
		const payload = {
			file: { ...minted },
			nested: { list: [{ ...minted }, [{ ...minted }], 3] },
			other: "x",
		};
		const result = resolveWidgetFileHandles(payload, registry);
		expect(result).toEqual({
			ok: true,
			payload: {
				file: resolved,
				nested: { list: [resolved, [resolved], 3] },
				other: "x",
			},
		});
		expect(payload.file).toEqual(minted);
		expect(resolveWidgetFileHandles([{ ...minted }], registry)).toEqual({
			ok: true,
			payload: [resolved],
		});
		expect(resolveWidgetFileHandles({ ...minted }, registry)).toEqual({
			ok: true,
			payload: resolved,
		});
	});

	test("the substituted file is a copy of the registered one", () => {
		const result = resolveWidgetFileHandles({ file: { ...minted } }, registry);
		if (!result.ok) throw new Error("expected the payload to resolve");
		const { file } = result.payload as { file: typeof resolved };
		expect(file).toEqual(resolved);
		expect(file).not.toBe(resolved);
		expect(file.flowPath).not.toBe(FLOW_PATH);
	});

	test("an unknown, foreign or altered handle fails the whole payload", () => {
		const forged: unknown[] = [
			{ ...minted, $flwFile: "flwf_BBBBBBBBBBBBBBBBBBBBBBBB" },
			{ ...minted, name: "other.pdf" },
			{ ...minted, size: 6 },
			{ ...minted, mimeType: "text/html" },
			{ ...minted, path: FLOW_PATH.path },
			{ $flwFile: minted.$flwFile },
			{ $flwFile: "../apps/app-1/storage/x" },
			{ $flwFile: { path: "apps/app-1" } },
		];
		for (const value of forged) {
			expect(
				resolveWidgetFileHandles({ ok: { ...minted }, bad: value }, registry),
			).toEqual({ ok: false });
			expect(
				resolveWidgetFileHandles([[{ ...minted }], [value]], registry),
			).toEqual({ ok: false });
		}
	});

	test("a handle minted for another widget instance does not resolve", () => {
		expect(resolveWidgetFileHandles({ file: minted }, new Map())).toEqual({
			ok: false,
		});
	});

	test("an object key named __proto__ stays data", () => {
		const payload = JSON.parse(
			`{"__proto__": {"polluted": true}, "file": ${JSON.stringify(minted)}}`,
		);
		const result = resolveWidgetFileHandles(payload, registry);
		if (!result.ok) throw new Error("expected the payload to resolve");
		const value = result.payload as Record<string, unknown>;
		expect(Object.getPrototypeOf(value)).toBe(Object.prototype);
		expect(Object.hasOwn(value, "__proto__")).toBe(true);
		expect(({} as Record<string, unknown>).polluted).toBeUndefined();
	});

	test("a cyclic payload fails instead of recursing forever", () => {
		const payload: Record<string, unknown> = { file: { ...minted } };
		payload.self = payload;
		expect(resolveWidgetFileHandles(payload, registry)).toEqual({ ok: false });
	});

	test("objects that are not plain records are left alone", () => {
		const date = new Date(0);
		const result = resolveWidgetFileHandles({ at: date }, registry);
		expect(result.ok && (result.payload as { at: Date }).at).toBe(date);
	});
});

describe("uploadWidgetFile", () => {
	const file = new File(["hello"], "a.pdf", { type: "application/pdf" });
	const signal = new AbortController().signal;

	function target(
		helper: Partial<IHelperState>,
		overrides: Partial<WidgetUploadTarget> = {},
	): WidgetUploadTarget {
		return {
			helper: {
				fileToUrl: async () => {
					throw new Error("Method not implemented.");
				},
				...helper,
			},
			appId: "app-1",
			signedIn: true,
			...overrides,
		};
	}

	test("uploads through the bulk path with the app id and execution target", async () => {
		const calls: unknown[] = [];
		const outcome = await uploadWidgetFile(
			file,
			target(
				{
					filesToTemporaryFiles: async (files, options) => {
						calls.push([files, options]);
						return [
							{
								file: files[0],
								uploaded: { url: SIGNED_URL, flowPath: FLOW_PATH },
							},
						];
					},
				},
				{ executionTarget: async () => "remote" },
			),
			signal,
		);
		expect(outcome).toEqual({ ok: true, url: SIGNED_URL, flowPath: FLOW_PATH });
		expect(calls).toEqual([
			[[file], { appId: "app-1", executionTarget: "remote", signal }],
		]);
	});

	test("keeps a desktop asset URL without a FlowPath", async () => {
		const url = "asset://localhost/%2Fcache%2Fchat%2Fx.pdf?filename=a.pdf";
		expect(
			await uploadWidgetFile(
				file,
				target({
					filesToTemporaryFiles: async (files) => [
						{ file: files[0], uploaded: { url } },
					],
				}),
				signal,
			),
		).toEqual({ ok: true, url });
	});

	test("a signed-out web viewer gets a blob: URL, which is refused as sign-in", async () => {
		const url = URL.createObjectURL(new Blob(["x"]));
		expect(
			await uploadWidgetFile(
				file,
				target(
					{
						filesToTemporaryFiles: async (files) => [
							{ file: files[0], uploaded: { url } },
						],
					},
					{ signedIn: false },
				),
				signal,
			),
		).toEqual({ ok: false, error: "sign-in-required" });
	});

	test("a signed-out desktop viewer whose upload is refused is asked to sign in", async () => {
		expect(
			await uploadWidgetFile(
				file,
				target(
					{
						filesToTemporaryFiles: async () => {
							throw new Error("Profile or auth not set");
						},
					},
					{ signedIn: false },
				),
				signal,
			),
		).toEqual({ ok: false, error: "sign-in-required" });
	});

	test("a failed upload of a signed-in viewer is a generic failure", async () => {
		expect(
			await uploadWidgetFile(
				file,
				target({
					filesToTemporaryFiles: async (files) => [
						{ file: files[0], error: "403 Forbidden" },
					],
				}),
				signal,
			),
		).toEqual({ ok: false, error: "failed" });
	});

	test("no app or no temporary storage is unavailable", async () => {
		const bulk = target({
			filesToTemporaryFiles: async () => {
				throw new Error("must not upload");
			},
		});
		expect(
			await uploadWidgetFile(file, { ...bulk, appId: undefined }, signal),
		).toEqual({ ok: false, error: "unavailable" });
		expect(await uploadWidgetFile(file, target({}), signal)).toEqual({
			ok: false,
			error: "unavailable",
		});
	});

	test("a hosted page without the bulk path is unavailable, signed in or not", async () => {
		const hosted = target({
			fileToTemporaryFile: async () => {
				throw new Error("Method not implemented.");
			},
		});
		expect(await uploadWidgetFile(file, hosted, signal)).toEqual({
			ok: false,
			error: "unavailable",
		});
		expect(
			await uploadWidgetFile(file, { ...hosted, signedIn: false }, signal),
		).toEqual({ ok: false, error: "unavailable" });
	});

	test("the single-file fallback is used like a FileInput", async () => {
		const calls: unknown[] = [];
		expect(
			await uploadWidgetFile(
				file,
				target(
					{
						fileToTemporaryFile: async (...args) => {
							calls.push(args.slice(1));
							return { url: SIGNED_URL, flowPath: FLOW_PATH };
						},
					},
					{ executionTarget: async () => "local" },
				),
				signal,
			),
		).toEqual({ ok: true, url: SIGNED_URL, flowPath: FLOW_PATH });
		expect(calls).toEqual([[false, "app-1", "local"]]);
	});

	test("a result without FlowPath or a readable URL is refused", async () => {
		const result = (url: string) =>
			uploadWidgetFile(
				file,
				target({
					filesToTemporaryFiles: async (files) => [
						{ file: files[0], uploaded: { url } },
					],
				}),
				signal,
			);
		expect(await result("")).toEqual({ ok: false, error: "unavailable" });
		expect(await result("data:application/pdf;base64,aGVsbG8=")).toEqual({
			ok: false,
			error: "sign-in-required",
		});
		expect(await result("javascript:alert(1)")).toEqual({
			ok: false,
			error: "unavailable",
		});
	});

	test("a FlowPath result keeps only a readable URL and drops a malformed FlowPath", async () => {
		const result = (uploaded: Record<string, unknown>) =>
			uploadWidgetFile(
				file,
				target({
					filesToTemporaryFiles: async (files) => [
						{ file: files[0], uploaded: uploaded as never },
					],
				}),
				signal,
			);
		expect(await result({ url: "", flowPath: FLOW_PATH })).toEqual({
			ok: true,
			url: "",
			flowPath: FLOW_PATH,
		});
		expect(
			await result({ url: SIGNED_URL, flowPath: { path: "", store_ref: "x" } }),
		).toEqual({ ok: true, url: SIGNED_URL });
	});
});
