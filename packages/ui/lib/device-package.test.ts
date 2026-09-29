import { afterEach, describe, expect, test } from "bun:test";
import AdmZip from "adm-zip";
import { standalonePackageFixture } from "../../../.github/scripts/tests/fixtures/standalone-package";
import {
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
const fixture = () => standalonePackageFixture(bytes, now);

describe("verified standalone release packages", () => {
	test("oversized signed binaries retain native bootstrap and signed Docker options", async () => {
		const { config, manifest, sign } = await fixture();
		manifest.artifacts[0].size = 256 * 1024 * 1024 + 1;
		const verified = await verifyReleaseManifest(await sign(manifest), config);
		expect(
			standalonePackageModes(verified.manifest, "x86_64-unknown-linux-gnu"),
		).toEqual({ binary: true, docker: true });
		for (const mode of ["binary", "docker", "both"] as const)
			expect(() =>
				validateStandalonePackageSelection(
					verified.manifest,
					"x86_64-unknown-linux-gnu",
					mode,
				),
			).not.toThrow();
		manifest.artifacts[0].target = "aarch64-apple-darwin";
		expect(standalonePackageModes(manifest, "aarch64-apple-darwin")).toEqual({
			binary: true,
			docker: false,
		});
		expect(() =>
			validateStandalonePackageSelection(
				manifest,
				"aarch64-apple-darwin",
				"binary",
			),
		).not.toThrow();
		manifest.artifacts[0].size = 2 * 1024 ** 3 + 1;
		await expect(
			verifyReleaseManifest(await sign(manifest), config),
		).rejects.toThrow("size");
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
