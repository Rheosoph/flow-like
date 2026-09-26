import { afterEach, describe, expect, test } from "bun:test";
import AdmZip from "adm-zip";
import {
	CompactSign,
	calculateJwkThumbprint,
	exportJWK,
	generateKeyPair,
} from "jose";
import type { IApiState } from "../state/backend-state/api-state";
import type { IProfile } from "../types";
import { prepareDevicePackage } from "./device-management/setup";
import {
	type ReleaseConfig,
	type StandalonePackageInput,
	type StandaloneRelease,
	buildStandalonePackage,
	fetchVerifiedRelease,
	standalonePackageModes,
	validateReleaseConfig,
	validateStandalonePackageSelection,
	verifyReleaseManifest,
} from "./device-package";

const originalFetch = globalThis.fetch;
afterEach(() => {
	globalThis.fetch = originalFetch;
});
const now = Math.floor(Date.now() / 1000);
const bytes = new TextEncoder().encode("verified standalone binary fixture");
async function fixture() {
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

describe("verified standalone release packages", () => {
	test("oversized signed binaries fail before enrollment while retaining signed Linux Docker options", async () => {
		const { config, manifest, sign } = await fixture();
		manifest.artifacts[0].size = 256 * 1024 * 1024 + 1;
		const signed = await sign(manifest);
		const verified = await verifyReleaseManifest(signed, config);
		expect(
			standalonePackageModes(verified.manifest, "x86_64-unknown-linux-gnu"),
		).toEqual({ binary: false, docker: true });
		expect(() =>
			validateStandalonePackageSelection(
				verified.manifest,
				"x86_64-unknown-linux-gnu",
				"docker",
			),
		).not.toThrow();
		let requests = 0;
		const api = {
			fetch: async () => {
				requests++;
				throw new Error("enrollment reached");
			},
		} as unknown as IApiState;
		for (const mode of ["binary", "both"] as const)
			await expect(
				prepareDevicePackage({
					api,
					profile: {} as IProfile,
					scope: {
						account: "owner",
						issuer: "issuer",
						apiOrigin: "https://api.example",
						profileId: "profile",
					},
					name: "Device",
					password: "management password",
					target: "x86_64-unknown-linux-gnu",
					mode,
					release: config,
					verifiedRelease: verified,
				}),
			).rejects.toThrow("256 MiB");
		expect(requests).toBe(0);
		manifest.artifacts[0].target = "aarch64-apple-darwin";
		expect(standalonePackageModes(manifest, "aarch64-apple-darwin")).toEqual({
			binary: false,
			docker: false,
		});
		expect(() =>
			validateStandalonePackageSelection(
				manifest,
				"aarch64-apple-darwin",
				"binary",
			),
		).toThrow("no Docker alternative");
	});
	test("requires configured keys, signature profile, sequence, and fresh metadata", async () => {
		const { config, manifest, signed, sign } = await fixture();
		expect((await verifyReleaseManifest(signed, config, now)).targets).toEqual([
			"x86_64-unknown-linux-gnu",
		]);
		expect(() =>
			validateReleaseConfig({ ...config, publicKeys: [] }),
		).toThrow();
		expect(() =>
			validateReleaseConfig({
				...config,
				publicKeys: [Buffer.alloc(32).toString("base64url")],
			}),
		).toThrow();
		expect(() =>
			validateReleaseConfig({
				...config,
				manifestUrl: "http://releases.example/release",
			}),
		).toThrow();
		await expect(
			verifyReleaseManifest(signed, { ...config, minimumSequence: 5 }, now),
		).rejects.toThrow();
		await expect(
			verifyReleaseManifest(signed, config, manifest.expires_at),
		).rejects.toThrow();
		await expect(
			verifyReleaseManifest(
				await sign(manifest, "flow-like-device-onboarding+jwt"),
				config,
				now,
			),
		).rejects.toThrow();
		await expect(
			verifyReleaseManifest(
				await sign({
					...manifest,
					artifacts: [...manifest.artifacts, ...manifest.artifacts],
				}),
				config,
				now,
			),
		).rejects.toThrow();
		await expect(
			verifyReleaseManifest(
				await sign(
					JSON.stringify(manifest).replace(
						'"sequence":4',
						'"sequence":3,"sequence":4',
					),
				),
				config,
				now,
			),
		).rejects.toThrow("Duplicate");
	});

	test("packages verified target bytes, private enrollment, scripts, and a digest-pinned container", async () => {
		const { signed, config, input } = await fixture();
		const requested: string[] = [];
		globalThis.fetch = (async (
			url: string | URL | Request,
			options?: RequestInit,
		) => {
			requested.push(String(url));
			expect(options?.credentials).toBe("omit");
			expect(options?.redirect).toBe("error");
			expect(options?.referrerPolicy).toBe("no-referrer");
			return new Response(String(url) === config.manifestUrl ? signed : bytes);
		}) as typeof fetch;
		const blob = await buildStandalonePackage(input);
		const archive = new AdmZip(Buffer.from(await blob.arrayBuffer()));
		expect(requested).toEqual([
			config.manifestUrl,
			"https://releases.example/agent",
		]);
		expect(archive.readFile("flow-like-standalone")).toEqual(
			Buffer.from(bytes),
		);
		expect((archive.getEntry("flow-like-standalone")?.attr ?? 0) >>> 16).toBe(
			0o100700,
		);
		expect((archive.getEntry("onboarding.json")?.attr ?? 0) >>> 16).toBe(
			0o100600,
		);
		expect(archive.readAsText("compose.yaml")).toContain(
			`@sha256:${"a".repeat(64)}`,
		);
		expect(archive.readAsText(".gitignore")).toBe("*\n!.gitignore\n");
		expect(archive.readAsText("state/agent.env")).toContain(
			"FLOW_LIKE_DEVICE_ISOLATION_POLICY=compatible",
		);
		expect((archive.getEntry("state/agent.env")?.attr ?? 0) >>> 16).toBe(
			0o100600,
		);
		expect(archive.readAsText(".env")).not.toContain(input.enrollment_token);
		expect(
			archive.getEntries().some((entry) => entry.entryName.includes("vault")),
		).toBe(false);
		expect(
			JSON.parse(archive.readAsText("onboarding.json")).bootstrap_secret,
		).toEqual(input.bootstrap_secret);
	});

	test("Docker mode downloads no binary and cached metadata cannot substitute artifact fields", async () => {
		const { signed, config, input } = await fixture();
		const verified = await verifyReleaseManifest(signed, config);
		verified.manifest.artifacts[0] = {
			...verified.manifest.artifacts[0],
			url: "https://untrusted.example/changed",
			size: 1,
			sha256: "b".repeat(64),
			target: input.target,
		};
		globalThis.fetch = (async () => {
			throw new Error("Docker-only must not download a host binary");
		}) as unknown as typeof fetch;
		const archive = new AdmZip(
			Buffer.from(
				await (
					await buildStandalonePackage({
						...input,
						mode: "docker",
						verifiedRelease: verified,
					})
				).arrayBuffer(),
			),
		);
		expect(archive.getEntry("flow-like-standalone")).toBeNull();
		expect(archive.readAsText("container-start.sh")).toContain(
			"enroll /package",
		);
		expect(JSON.parse(archive.readAsText("platform.json")).target).toBe(
			input.target,
		);
	});

	test("rejects changed binary bytes, truncation, mismatched bootstrap, and canceled requests", async () => {
		const { signed, config, input } = await fixture();
		const verified = await verifyReleaseManifest(signed, config);
		globalThis.fetch = (async () =>
			new Response(new Uint8Array(bytes.length))) as unknown as typeof fetch;
		await expect(
			buildStandalonePackage({ ...input, verifiedRelease: verified }),
		).rejects.toThrow("digest");
		globalThis.fetch = (async () =>
			new Response(new Uint8Array(1))) as unknown as typeof fetch;
		await expect(
			buildStandalonePackage({ ...input, verifiedRelease: verified }),
		).rejects.toThrow("truncated");
		await expect(
			buildStandalonePackage({
				...input,
				mode: "docker",
				verifiedRelease: verified,
				bootstrap_secret: Array(32).fill(1),
			}),
		).rejects.toThrow("bootstrap secret");
		globalThis.fetch = (async () => {
			throw new Error("must not fetch");
		}) as unknown as typeof fetch;
		await expect(
			fetchVerifiedRelease(config, AbortSignal.abort()),
		).rejects.toBeDefined();
	});
});
