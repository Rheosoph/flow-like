import { afterEach, describe, expect, test } from "bun:test";
import AdmZip from "adm-zip";
import { standalonePackageFixture } from "../../../.github/scripts/tests/fixtures/standalone-package";
import {
	MAX_RELEASE_LIFETIME_S,
	RELEASE_NOT_YET_VALID_TOLERANCE_S,
	type ReleaseCheck,
	ReleaseVerificationError,
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

const DAY = 86_400;

/** The rejection of a verification, as the screens receive it. */
async function refusal(run: Promise<unknown>): Promise<unknown> {
	try {
		await run;
	} catch (error) {
		return error;
	}
	throw new Error("The verification was expected to fail");
}

function expectCheck(error: unknown, check: ReleaseCheck, message: string) {
	expect(error).toBeInstanceOf(ReleaseVerificationError);
	const failed = error as ReleaseVerificationError;
	expect(failed.check).toBe(check);
	expect(failed.message).toBe(message);
	return failed;
}

describe("release validity", () => {
	test("the cap is five years and the clock tolerance five minutes", () => {
		expect(MAX_RELEASE_LIFETIME_S).toBe(157_680_000);
		expect(RELEASE_NOT_YET_VALID_TOLERANCE_S).toBe(300);
	});

	test("a list valid for 365 days verifies; the cap holds to the second", async () => {
		const { config, manifest, sign } = await fixture();
		const lasting = (seconds: number) =>
			sign({ ...manifest, issued_at: now, expires_at: now + seconds });
		const year = await verifyReleaseManifest(
			await lasting(365 * DAY),
			config,
			now,
		);
		expect(year.manifest.expires_at - year.manifest.issued_at).toBe(365 * DAY);
		const atCap = await verifyReleaseManifest(
			await lasting(MAX_RELEASE_LIFETIME_S),
			config,
			now,
		);
		expect(atCap.manifest.expires_at).toBe(now + MAX_RELEASE_LIFETIME_S);
		const over = expectCheck(
			await refusal(
				verifyReleaseManifest(
					await lasting(MAX_RELEASE_LIFETIME_S + 1),
					config,
					now,
				),
			),
			"lifetime",
			"The release manifest is valid for longer than this app accepts.",
		);
		expect(over.facts?.expires_at).toBe(now + MAX_RELEASE_LIFETIME_S + 1);
	});

	test("a list issued up to 300 s ahead of this clock verifies; expiry has no tolerance", async () => {
		const { config, manifest, signed, sign } = await fixture();
		const issuedIn = (seconds: number) =>
			sign({
				...manifest,
				issued_at: now + seconds,
				expires_at: now + seconds + 30 * DAY,
			});
		for (const ahead of [299, 300])
			expect(
				(await verifyReleaseManifest(await issuedIn(ahead), config, now))
					.manifest.issued_at,
			).toBe(now + ahead);
		expectCheck(
			await refusal(verifyReleaseManifest(await issuedIn(301), config, now)),
			"not_yet_valid",
			"The release manifest is not valid yet.",
		);
		expect(
			(await verifyReleaseManifest(signed, config, manifest.expires_at - 1))
				.manifest.sequence,
		).toBe(4);
		expectCheck(
			await refusal(verifyReleaseManifest(signed, config, manifest.expires_at)),
			"expired",
			"The release manifest has expired.",
		);
	});

	test("each failed check names itself, with facts only once the signature verified", async () => {
		const { config, manifest, signed, sign } = await fixture();
		const facts = {
			release_version: "1.2.3",
			sequence: 4,
			issued_at: manifest.issued_at,
			expires_at: manifest.expires_at,
		};
		const other = await fixture();

		const unpinned = expectCheck(
			await refusal(verifyReleaseManifest(signed, other.config, now)),
			"signature",
			"The package signature does not match a configured release key.",
		);
		expect(unpinned.facts).toBeUndefined();
		const [head, body] = signed.split(".");
		const forged = expectCheck(
			await refusal(
				verifyReleaseManifest(`${head}.${body}.${"A".repeat(86)}`, config, now),
			),
			"signature",
			"Release signature verification failed.",
		);
		expect(forged.facts).toBeUndefined();

		expect(
			expectCheck(
				await refusal(
					verifyReleaseManifest(signed, { ...config, minimumSequence: 5 }, now),
				),
				"sequence",
				"The release sequence is older than the configured minimum.",
			).facts,
		).toEqual(facts);
		expect(
			expectCheck(
				await refusal(
					verifyReleaseManifest(signed, config, manifest.expires_at),
				),
				"expired",
				"The release manifest has expired.",
			).facts,
		).toEqual(facts);

		const wrongProfile = expectCheck(
			await refusal(
				verifyReleaseManifest(
					await sign(manifest, "flow-like-device-onboarding+jwt"),
					config,
					now,
				),
			),
			"invalid",
			"Package signature profile mismatch.",
		);
		expect(wrongProfile.facts).toBeUndefined();
		const malformed = expectCheck(
			await refusal(
				verifyReleaseManifest(
					await sign({ ...manifest, expires_at: manifest.issued_at }),
					config,
					now,
				),
			),
			"invalid",
			"Invalid release validity dates.",
		);
		expect(malformed.facts).toBeUndefined();
		expectCheck(
			await refusal(verifyReleaseManifest("not a release list", config, now)),
			"invalid",
			"Invalid package signature.",
		);
	});

	test("the first failing check is the reason: minimum, then lifetime, then the clock", async () => {
		const { config, manifest, signed, sign } = await fixture();
		expectCheck(
			await refusal(
				verifyReleaseManifest(
					signed,
					{ ...config, minimumSequence: 5 },
					manifest.expires_at,
				),
			),
			"sequence",
			"The release sequence is older than the configured minimum.",
		);
		const tooLong = await sign({
			...manifest,
			issued_at: now,
			expires_at: now + MAX_RELEASE_LIFETIME_S + 1,
		});
		for (const clock of [now - 301, now + MAX_RELEASE_LIFETIME_S + 1])
			expectCheck(
				await refusal(verifyReleaseManifest(tooLong, config, clock)),
				"lifetime",
				"The release manifest is valid for longer than this app accepts.",
			);
	});

	test("a list that didn't arrive, or release settings that can't be read, are not failed checks", async () => {
		const { config, signed } = await fixture();
		globalThis.fetch = (async () =>
			new Response("Not Found", { status: 404 })) as unknown as typeof fetch;
		expect(await refusal(fetchVerifiedRelease(config))).not.toBeInstanceOf(
			ReleaseVerificationError,
		);
		expect(
			await refusal(
				verifyReleaseManifest(signed, { ...config, publicKeys: [] }, now),
			),
		).not.toBeInstanceOf(ReleaseVerificationError);
		globalThis.fetch = (async () =>
			new Response(signed)) as unknown as typeof fetch;
		expect((await fetchVerifiedRelease(config)).manifest.sequence).toBe(4);
	});
});
