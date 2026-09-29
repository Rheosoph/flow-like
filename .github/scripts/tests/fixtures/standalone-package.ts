import {
	CompactSign,
	calculateJwkThumbprint,
	exportJWK,
	generateKeyPair,
} from "jose";
import type {
	ReleaseConfig,
	StandalonePackageInput,
	StandaloneRelease,
} from "../../../../packages/ui/lib/device-package";
export async function standalonePackageFixture(
	bytes = new TextEncoder().encode("verified standalone binary fixture"),
	now = Math.floor(Date.now() / 1000),
) {
	const releaseKey = await generateKeyPair("EdDSA", { extractable: true });
	const publicKey = await exportJWK(releaseKey.publicKey);
	const fingerprint = await calculateJwkThumbprint(publicKey);
	const config: ReleaseConfig = {
		manifestUrl: "https://releases.example/release.jws",
		publicKeys: [String(publicKey.x)],
		minimumSequence: 4,
	};
	const digest = [
		...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
	]
		.map((byte) => byte.toString(16).padStart(2, "0"))
		.join("");
	const manifest: StandaloneRelease = {
		version: 1,
		state_schema_version: 4,
		sequence: 4,
		release_version: "1.2.3",
		issued_at: now - 1,
		expires_at: now + 3600,
		artifacts: [
			{
				target: "x86_64-unknown-linux-gnu",
				url: "https://releases.example/agent",
				size: bytes.length,
				sha256: digest,
			},
		],
		container: {
			image: `ghcr.io/example/agent@sha256:${"a".repeat(64)}`,
			platforms: ["linux/amd64"],
		},
	};
	const sign = (value: unknown, type = "flow-like-standalone-release+jws") =>
		new CompactSign(
			new TextEncoder().encode(
				typeof value === "string" ? value : JSON.stringify(value),
			),
		)
			.setProtectedHeader({ alg: "EdDSA", typ: type, kid: fingerprint })
			.sign(releaseKey.privateKey);
	const signed = await sign(manifest);
	const controller = await generateKeyPair("EdDSA", { extractable: true });
	const controllerJwk = await exportJWK(controller.publicKey);
	const bootstrap = await generateKeyPair("EdDSA", { extractable: true });
	const bootstrapJwk = await exportJWK(bootstrap.privateKey);
	const owner = await generateKeyPair("EdDSA", { extractable: true });
	const ownerJwk = await exportJWK(owner.publicKey);
	const publicPart = (key: JsonWebKey) => ({
		kty: "OKP",
		crv: "Ed25519",
		x: key.x,
	});
	const onboarding = {
		version: 1,
		enrollment_id: "enrollment",
		device_id: "device",
		owner_id: "owner",
		name: "Packaged device",
		api_base_url: "https://api.example/api/v1",
		bootstrap_key: publicPart(bootstrapJwk),
		controller_key: publicPart(controllerJwk),
		owner_invitation_key: publicPart(ownerJwk),
		issued_at: now - 1,
		expires_at: now + 3600,
	};
	const onboardingJws = await new CompactSign(
		new TextEncoder().encode(JSON.stringify(onboarding)),
	)
		.setProtectedHeader({
			alg: "EdDSA",
			typ: "flow-like-device-onboarding+jwt",
			kid: await calculateJwkThumbprint(controllerJwk),
		})
		.sign(controller.privateKey);
	const input: StandalonePackageInput = {
		manifest: onboarding,
		manifest_jws: onboardingJws,
		enrollment_token: "short-lived-enrollment-token",
		bootstrap_secret: [...Buffer.from(String(bootstrapJwk.d), "base64url")],
		target: "x86_64-unknown-linux-gnu",
		mode: "both",
		release: config,
	};
	return { config, manifest, sign, signed, input };
}
