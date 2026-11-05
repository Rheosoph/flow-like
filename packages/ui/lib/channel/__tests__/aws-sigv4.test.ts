import { describe, expect, it } from "bun:test";
import { canonicalQuery, rfc3986Encode, signingKey } from "../aws-sigv4";

const hex = (buffer: Uint8Array) => Buffer.from(buffer).toString("hex");

describe("SigV4 primitives", () => {
	it("encodes per RFC 3986", () => {
		expect(rfc3986Encode("a b!*'()~-_./")).toBe("a%20b%21%2A%27%28%29~-_.%2F");
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
			"wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
			"20150830",
			"us-east-1",
			"iam",
		);
		expect(hex(key)).toBe(
			"c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9",
		);
	});
});
