import { describe, expect, it } from "bun:test";
import { createHash, createHmac } from "node:crypto";
import {
	canonicalQuery,
	canonicalUri,
	rfc3986Encode,
	signAwsRequest,
	signingKey,
} from "../aws-sigv4";

const CREDENTIALS = {
	accessKeyId: "AKIDEXAMPLE",
	secretAccessKey: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
};
const SIGNING_DATE = new Date("2015-08-30T12:36:00Z");

const hex = (buffer: Uint8Array) => Buffer.from(buffer).toString("hex");

/** Independent reference built on node:crypto, used to cross-check the WebCrypto signer. */
function referenceSignature(
	secret: string,
	scope: { dateStamp: string; region: string; service: string },
	stringToSign: string,
): string {
	const hmac = (key: Buffer | string, data: string) =>
		createHmac("sha256", key).update(data).digest();
	const kDate = hmac(`AWS4${secret}`, scope.dateStamp);
	const kRegion = hmac(kDate, scope.region);
	const kService = hmac(kRegion, scope.service);
	const kSigning = hmac(kService, "aws4_request");
	return createHmac("sha256", kSigning).update(stringToSign).digest("hex");
}

describe("SigV4 primitives", () => {
	it("encodes per RFC 3986", () => {
		expect(rfc3986Encode("a b!*'()~-_./")).toBe("a%20b%21%2A%27%28%29~-_.%2F");
	});

	it("encodes path segments exactly once", () => {
		expect(canonicalUri("/connections/run%2Fabc/messages")).toBe(
			"/connections/run%2Fabc/messages",
		);
		expect(canonicalUri("/a b/c*d")).toBe("/a%20b/c%2Ad");
		expect(canonicalUri("")).toBe("/");
	});

	it("sorts and encodes query parameters", () => {
		expect(
			canonicalQuery(
				new URLSearchParams(
					"topic=runs/run-1/inbox&confirmation=true&timeout=10&a=2&a=1",
				),
			),
		).toBe("a=1&a=2&confirmation=true&timeout=10&topic=runs%2Frun-1%2Finbox");
	});

	it("derives the documented IAM example signing key", async () => {
		// https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html
		const key = await signingKey(
			CREDENTIALS.secretAccessKey,
			"20150830",
			"us-east-1",
			"iam",
		);
		expect(hex(key)).toBe(
			"c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9",
		);
	});
});

describe("signAwsRequest", () => {
	it("reproduces the AWS 'get-vanilla' test-suite vector", async () => {
		const signed = await signAwsRequest(
			{
				method: "GET",
				url: "https://example.amazonaws.com/",
				service: "service",
				region: "us-east-1",
				date: SIGNING_DATE,
			},
			CREDENTIALS,
		);

		expect(signed.canonicalRequest).toBe(
			[
				"GET",
				"/",
				"",
				"host:example.amazonaws.com",
				"x-amz-date:20150830T123600Z",
				"",
				"host;x-amz-date",
				"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
			].join("\n"),
		);
		expect(signed.stringToSign).toBe(
			[
				"AWS4-HMAC-SHA256",
				"20150830T123600Z",
				"20150830/us-east-1/service/aws4_request",
				"bb579772317eb040ac9ed261061d46c1f17a8133879d6129b6e1c25292927e63",
			].join("\n"),
		);
		expect(signed.signature).toBe(
			"5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
		);
		expect(signed.headers).toEqual({
			"x-amz-date": "20150830T123600Z",
			authorization:
				"AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
		});
	});

	it("signs an IoT direct message like the reference implementation", async () => {
		const body = JSON.stringify({
			channel_id: "run-1",
			request_id: "req-1",
			kind: "reply",
			value: { ok: true },
		});
		const bodyHash = createHash("sha256").update(body).digest("hex");
		const url =
			"https://abc-ats.iot.eu-central-1.amazonaws.com/connections/run-1/messages?topic=runs%2Frun-1%2Finbox&confirmation=true&timeout=10";

		const signed = await signAwsRequest(
			{
				method: "POST",
				url,
				headers: {
					"Content-Type": "application/json",
					"x-amz-content-sha256": bodyHash,
				},
				body,
				service: "iotdata",
				region: "eu-central-1",
				date: SIGNING_DATE,
			},
			{ ...CREDENTIALS, sessionToken: "SESSION/TOKEN==" },
		);

		expect(signed.canonicalRequest).toBe(
			[
				"POST",
				"/connections/run-1/messages",
				"confirmation=true&timeout=10&topic=runs%2Frun-1%2Finbox",
				"content-type:application/json",
				"host:abc-ats.iot.eu-central-1.amazonaws.com",
				`x-amz-content-sha256:${bodyHash}`,
				"x-amz-date:20150830T123600Z",
				"x-amz-security-token:SESSION/TOKEN==",
				"",
				"content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
				bodyHash,
			].join("\n"),
		);
		expect(signed.signedHeaders).toBe(
			"content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
		);
		const canonicalHash = createHash("sha256")
			.update(signed.canonicalRequest)
			.digest("hex");
		expect(signed.stringToSign).toBe(
			[
				"AWS4-HMAC-SHA256",
				"20150830T123600Z",
				"20150830/eu-central-1/iotdata/aws4_request",
				canonicalHash,
			].join("\n"),
		);
		expect(signed.signature).toBe(
			referenceSignature(
				CREDENTIALS.secretAccessKey,
				{ dateStamp: "20150830", region: "eu-central-1", service: "iotdata" },
				signed.stringToSign,
			),
		);
		expect(signed.headers["x-amz-security-token"]).toBe("SESSION/TOKEN==");
		expect(signed.headers.host).toBeUndefined();
		expect(signed.headers.authorization).toBe(
			`AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/eu-central-1/iotdata/aws4_request, SignedHeaders=${signed.signedHeaders}, Signature=${signed.signature}`,
		);
	});
});
