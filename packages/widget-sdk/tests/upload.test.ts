import { describe, expect, test } from "bun:test";
import {
	type FlwEnvelope,
	type InitCapabilities,
	createEnvelope,
} from "../src/protocol";
import {
	type FileUploadPayload,
	MAX_WIDGET_UPLOAD_BYTES,
	type WidgetFile,
	WidgetUploadError,
	createWidgetUploadClient,
	isWidgetFile,
} from "../src/upload";
import { validateSchema } from "../src/validate";

const HANDLE = "flwf_0123456789abcdef";

function setup(capabilities: InitCapabilities = { uploads: true }) {
	const messages: FlwEnvelope[] = [];
	const client = createWidgetUploadClient(
		(type, payload) => {
			messages.push(createEnvelope(type, payload, "nonce", "instance"));
		},
		() => capabilities,
	);
	const reply = (requestId: string, result: Record<string, unknown>) =>
		client.handle(
			createEnvelope(
				"file:result",
				{ requestId, ok: false, ...result },
				"nonce",
				"instance",
			),
		);
	return { client, messages, reply };
}

const handleFor = (name: string): WidgetFile => ({
	$flwFile: HANDLE,
	name,
	size: 5,
	mimeType: "text/plain",
});

describe("widget upload client", () => {
	test("posts the file itself and resolves the host's handle", async () => {
		const { client, messages, reply } = setup();
		const file = new File(["hello"], "notes.png", { type: "image/png" });
		const result = client.uploadFile(file);
		expect(messages[0]?.type).toBe("file:upload");
		const payload = messages[0]?.payload as FileUploadPayload;
		expect(payload.file).toBe(file);
		expect(payload.name).toBe("notes.png");
		expect(payload.mimeType).toBe("image/png");
		reply(payload.requestId, { ok: true, file: handleFor("notes.png") });
		expect(await result).toEqual(handleFor("notes.png"));
		client.dispose();
	});

	test("rejects without the uploads capability and posts nothing", async () => {
		const { client, messages } = setup({});
		await expect(client.uploadFile(new Blob(["x"]))).rejects.toMatchObject({
			code: "not-granted",
		});
		expect(messages.length).toBe(0);
	});

	test("rejects files over the limit before posting", async () => {
		const { client, messages } = setup();
		const large = new Blob([new Uint8Array(MAX_WIDGET_UPLOAD_BYTES + 1)]);
		await expect(client.uploadFile(large)).rejects.toMatchObject({
			code: "too-large",
		});
		expect(messages.length).toBe(0);
	});

	test("caps concurrent uploads", async () => {
		const { client } = setup();
		const inFlight = Array.from({ length: 4 }, () =>
			client.uploadFile(new Blob(["x"])).catch(() => undefined),
		);
		await expect(client.uploadFile(new Blob(["x"]))).rejects.toMatchObject({
			code: "busy",
		});
		client.dispose();
		await Promise.all(inFlight);
	});

	test("maps known host error codes and never trusts anything else", async () => {
		const { client, messages, reply } = setup();
		const signIn = client.uploadFile(new Blob(["x"]));
		const forged = client.uploadFile(new Blob(["x"]));
		const malformed = client.uploadFile(new Blob(["x"]));
		const [a = "", b = "", c = ""] = messages.map(
			(message) => (message.payload as FileUploadPayload).requestId,
		);
		reply(a, { error: "sign-in-required" });
		reply(b, { error: "Internal: s3 key apps/x" });
		reply(c, { ok: true, file: { ...handleFor("x"), path: "apps/x" } });
		await expect(signIn).rejects.toBeInstanceOf(WidgetUploadError);
		await expect(signIn).rejects.toMatchObject({ code: "sign-in-required" });
		await expect(forged).rejects.toMatchObject({ code: "failed" });
		await expect(malformed).rejects.toMatchObject({ code: "failed" });
	});

	test("abort posts a cancel to the host", async () => {
		const { client, messages } = setup();
		const controller = new AbortController();
		const result = client.uploadFile(new Blob(["x"]), {
			signal: controller.signal,
		});
		controller.abort();
		await expect(result).rejects.toThrow(/aborted/);
		expect(messages[1]?.type).toBe("file:cancel");
	});

	test("dispose cancels pending uploads", async () => {
		const { client, messages } = setup();
		const result = client.uploadFile(new Blob(["x"]));
		client.dispose();
		await expect(result).rejects.toMatchObject({ code: "disposed" });
		expect(messages.at(-1)?.type).toBe("file:cancel");
	});
});

describe("widget file handles", () => {
	test("accept only the exact handle shape", () => {
		expect(isWidgetFile(handleFor("a.pdf"))).toBe(true);
		expect(isWidgetFile({ ...handleFor("a.pdf"), $flwFile: "apps/x" })).toBe(
			false,
		);
		expect(isWidgetFile({ ...handleFor("a.pdf"), flowPath: {} })).toBe(false);
		expect(isWidgetFile({ ...handleFor(""), name: "" })).toBe(false);
	});

	test("the file marker validates event payloads against the handle form", () => {
		const schema = {
			type: "object",
			properties: { file: { type: "object", "x-flow-like-type": "file" } },
			required: ["file"],
		};
		expect(validateSchema(schema, { file: handleFor("a.pdf") }).valid).toBe(
			true,
		);
		const forged = validateSchema(schema, {
			file: {
				name: "a.pdf",
				flowPath: {
					path: "apps/x",
					store_ref: "__flow_like_http_request_files",
				},
			},
		});
		expect(forged.valid).toBe(false);
	});
});
