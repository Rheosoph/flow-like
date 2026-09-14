import { afterEach, describe, expect, test } from "bun:test";
import { uploadToSignedUrl } from "./signed-upload";

const originalXhr = globalThis.XMLHttpRequest;

class UploadRequest {
	static last: UploadRequest;
	upload = new EventTarget();
	events = new EventTarget();
	status = 204;
	statusText = "No Content";
	method = "";
	headers = new Map<string, string>();
	body?: XMLHttpRequestBodyInit;
	constructor() {
		UploadRequest.last = this;
	}
	addEventListener(type: string, callback: EventListener) {
		this.events.addEventListener(type, callback);
	}
	open(method: string) {
		this.method = method;
	}
	setRequestHeader(key: string, value: string) {
		this.headers.set(key, value);
	}
	send(body: XMLHttpRequestBodyInit) {
		this.body = body;
		queueMicrotask(() => this.events.dispatchEvent(new Event("load")));
	}
	abort() {
		this.events.dispatchEvent(new Event("abort"));
	}
}

afterEach(() => {
	globalThis.XMLHttpRequest = originalXhr;
});

describe("signed uploads", () => {
	test("S3 policy uploads use multipart POST with the file last", async () => {
		globalThis.XMLHttpRequest =
			UploadRequest as unknown as typeof XMLHttpRequest;
		const file = new File(["data"], "data.parquet");
		await uploadToSignedUrl(
			"https://bucket.s3.eu-central-1.amazonaws.com/",
			file,
			{
				fields: {
					key: "apps/app/upload/data.parquet",
					policy: "signed-policy",
					"x-amz-signature": "signature",
				},
			},
		);
		const request = UploadRequest.last;
		expect(request.method).toBe("POST");
		expect(request.headers.has("Content-Type")).toBe(false);
		expect(request.body).toBeInstanceOf(FormData);
		const form = request.body as FormData;
		expect([...form.keys()]).toEqual([
			"key",
			"policy",
			"x-amz-signature",
			"file",
		]);
		expect(form.get("policy")).toBe("signed-policy");
		expect((form.get("file") as File).size).toBe(4);
	});

	test("existing Azure signed URLs keep PUT and their required blob header", async () => {
		globalThis.XMLHttpRequest =
			UploadRequest as unknown as typeof XMLHttpRequest;
		const file = new Blob(["data"], { type: "text/plain" });
		await uploadToSignedUrl(
			"https://account.blob.core.windows.net/container/file?sig=test",
			file,
		);
		expect(UploadRequest.last.method).toBe("PUT");
		expect(UploadRequest.last.headers.get("x-ms-blob-type")).toBe("BlockBlob");
		expect(UploadRequest.last.body).toBe(file);
	});
});
