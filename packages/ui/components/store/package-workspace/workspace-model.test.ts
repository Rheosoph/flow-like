import { describe, expect, test } from "bun:test";
import { PackagePermissionBits } from "../../../lib/permission/wasm-package-permission";
import {
	type PackageMeta,
	PackageStatus,
	type PackageUser,
	type PackageVersion,
	type RegistryEntry,
} from "../../../lib/schema/wasm";
import {
	type WorkspaceAccessInput,
	errorHttpStatus,
	humanizeKey,
	listingHealth,
	liveVersion,
	overviewChecks,
	pendingVersion,
	peopleCounts,
	registryState,
	remoteStatusOf,
	storePreviewSummary,
	workspaceAccess,
	workspaceAuthState,
	workspaceTabs,
} from "./workspace-model";

const { Owner, Maintainer, User, Buyer } = PackagePermissionBits;

function access(
	overrides: Partial<Omit<WorkspaceAccessInput, "remote">> & {
		remote?: Partial<WorkspaceAccessInput["remote"]>;
	} = {},
) {
	const { remote, ...rest } = overrides;
	return workspaceAccess({
		packageId: "simple-math",
		hasLocal: false,
		auth: "signed_in",
		...rest,
		remote: { status: "ok", source: "registry", ...remote },
	});
}

const STORE_HREF = "/store/packages?id=simple-math";

describe("workspaceAccess", () => {
	describe("checkout still resolving", () => {
		test("waits instead of redirecting to someone else's store page", () => {
			expect(access({ localPending: true })).toEqual({ mode: "loading" });
			expect(
				access({ localPending: true, remote: { permission: User } }),
			).toEqual({ mode: "loading" });
		});

		test("waits before deciding local_only vs not_found", () => {
			expect(access({ localPending: true, packageId: undefined })).toEqual({
				mode: "loading",
			});
		});

		test("waits for a signed-out caller too", () => {
			expect(access({ localPending: true, auth: "signed_out" })).toEqual({
				mode: "loading",
			});
		});
	});

	describe("rule 1: no package id", () => {
		test("local checkout → local_only", () => {
			expect(access({ packageId: undefined, hasLocal: true })).toEqual({
				mode: "local_only",
			});
		});

		test("nothing → not_found, even while auth is unknown", () => {
			expect(
				access({
					packageId: "",
					auth: "unknown",
					remote: { status: "idle" },
				}),
			).toEqual({ mode: "not_found" });
		});
	});

	describe("rule 2: loading", () => {
		test("auth unknown → loading, even with a maintainer bit on an unsettled entry", () => {
			for (const status of ["idle", "loading"] as const) {
				expect(
					access({ auth: "unknown", remote: { status, permission: Owner } }),
				).toEqual({ mode: "loading" });
			}
		});

		test("auth unknown without a maintainer bit → loading, never a redirect", () => {
			for (const permission of [undefined, User, Buyer]) {
				expect(access({ auth: "unknown", remote: { permission } })).toEqual({
					mode: "loading",
				});
			}
		});

		test("a renewal keeps the confirmed maintainer answer for this user", () => {
			expect(
				access({ auth: "unknown", remote: { permission: Maintainer } }),
			).toEqual({ mode: "full", isOwner: false, isMaintainer: true });
		});

		test("auth unknown with a checkout → loading", () => {
			expect(
				access({
					auth: "unknown",
					hasLocal: true,
					remote: { status: "error" },
				}),
			).toEqual({ mode: "loading" });
		});

		test("idle (query still disabled) → loading", () => {
			expect(access({ remote: { status: "idle" } })).toEqual({
				mode: "loading",
			});
		});

		test("loading → loading, with or without a checkout", () => {
			expect(access({ remote: { status: "loading" } })).toEqual({
				mode: "loading",
			});
			expect(access({ hasLocal: true, remote: { status: "loading" } })).toEqual(
				{ mode: "loading" },
			);
		});
	});

	describe("signed out: no maintainer bit is possible", () => {
		test("no checkout → store page before anything loads", () => {
			for (const status of ["idle", "loading", "ok"] as const) {
				expect(access({ auth: "signed_out", remote: { status } })).toEqual({
					mode: "redirect",
					href: STORE_HREF,
				});
			}
		});

		test("no checkout + failed profile or registry → store page, not an error", () => {
			expect(
				access({ auth: "signed_out", remote: { status: "error" } }),
			).toEqual({ mode: "redirect", href: STORE_HREF });
			expect(
				access({ auth: "signed_out", remote: { status: "forbidden" } }),
			).toEqual({ mode: "redirect", href: STORE_HREF });
		});

		test("with a checkout: unpublished and unreachable are still told apart", () => {
			const signedOut = (remote: Partial<WorkspaceAccessInput["remote"]>) =>
				access({ auth: "signed_out", hasLocal: true, remote });
			expect(signedOut({ status: "loading" })).toEqual({ mode: "loading" });
			expect(signedOut({ status: "not_found" })).toEqual({
				mode: "local_only",
			});
			expect(signedOut({ status: "error" })).toEqual({
				mode: "local",
				banner: "registry_unavailable",
			});
		});

		test("with a checkout: a published id is never called taken while ownership is unknowable", () => {
			const signedOut = (remote: Partial<WorkspaceAccessInput["remote"]>) =>
				access({ auth: "signed_out", hasLocal: true, remote });
			for (const remote of [
				{ status: "ok" as const },
				{ status: "ok" as const, permission: Owner },
				{ status: "forbidden" as const },
			]) {
				expect(signedOut(remote)).toEqual({
					mode: "local",
					banner: "signed_out",
				});
			}
		});

		test("an anonymous entry never opens the owner shell", () => {
			expect(
				access({
					auth: "signed_out",
					hasLocal: true,
					remote: { permission: Owner },
				}).mode,
			).not.toBe("full");
			expect(
				access({ auth: "signed_out", remote: { permission: Owner } }),
			).toEqual({ mode: "redirect", href: STORE_HREF });
		});
	});

	describe("expired session: never an endless skeleton", () => {
		const expired = (
			overrides: Partial<Omit<WorkspaceAccessInput, "remote" | "auth">> & {
				remote?: Partial<WorkspaceAccessInput["remote"]>;
			} = {},
		) => access({ auth: "expired", ...overrides });

		test("nothing confirmed, no checkout → sign in again, no redirect", () => {
			for (const status of ["idle", "loading", "error"] as const) {
				expect(expired({ remote: { status } })).toEqual({
					mode: "session_expired",
				});
			}
		});

		test("nothing confirmed, with a checkout → local tabs with a sign-in banner", () => {
			for (const status of ["idle", "loading", "error"] as const) {
				expect(expired({ hasLocal: true, remote: { status } })).toEqual({
					mode: "local",
					banner: "session_expired",
				});
			}
		});

		test("a confirmed maintainer keeps the owner shell", () => {
			expect(expired({ remote: { permission: Owner } })).toEqual({
				mode: "full",
				isOwner: true,
				isMaintainer: true,
			});
			expect(
				expired({ hasLocal: true, remote: { permission: Maintainer } }),
			).toEqual({ mode: "full", isOwner: false, isMaintainer: true });
		});

		test("other confirmed answers still follow the table", () => {
			expect(expired({ remote: { status: "not_found" } })).toEqual({
				mode: "not_found",
			});
			expect(
				expired({ hasLocal: true, remote: { status: "not_found" } }),
			).toEqual({ mode: "local_only" });
			expect(expired({ remote: { status: "forbidden" } })).toEqual({
				mode: "not_found",
			});
			expect(expired({ hasLocal: true, remote: { permission: User } })).toEqual(
				{ mode: "local", banner: "id_taken" },
			);
			expect(expired({ remote: { permission: User } })).toEqual({
				mode: "redirect",
				href: STORE_HREF,
			});
		});
	});

	describe("rule 3: installed-package fallback", () => {
		test("source local → not_found", () => {
			expect(
				access({ remote: { source: "local", permission: Owner } }),
			).toEqual({ mode: "not_found" });
		});

		test("source local + checkout → local_only", () => {
			expect(access({ hasLocal: true, remote: { source: "local" } })).toEqual({
				mode: "local_only",
			});
		});

		test("unusable response without a source → not_found", () => {
			expect(
				access({ remote: { source: undefined, permission: Owner } }),
			).toEqual({ mode: "not_found" });
		});
	});

	describe("rule 4: 404", () => {
		test("with a project → local_only", () => {
			expect(
				access({ hasLocal: true, remote: { status: "not_found" } }),
			).toEqual({ mode: "local_only" });
		});

		test("without a project → not_found", () => {
			expect(access({ remote: { status: "not_found" } })).toEqual({
				mode: "not_found",
			});
		});
	});

	describe("rule 5: 403", () => {
		test("without a checkout → not_found", () => {
			expect(access({ remote: { status: "forbidden" } })).toEqual({
				mode: "not_found",
			});
		});

		test("with a checkout → id_taken", () => {
			expect(
				access({ hasLocal: true, remote: { status: "forbidden" } }),
			).toEqual({ mode: "local", banner: "id_taken" });
		});
	});

	describe("rule 6: fetch error never redirects", () => {
		test("without a checkout → error", () => {
			expect(access({ remote: { status: "error" } })).toEqual({
				mode: "error",
			});
		});

		test("with a checkout → registry_unavailable", () => {
			expect(access({ hasLocal: true, remote: { status: "error" } })).toEqual({
				mode: "local",
				banner: "registry_unavailable",
			});
		});
	});

	describe("rule 7: maintainer bit → full", () => {
		test("owner", () => {
			expect(access({ remote: { permission: Owner } })).toEqual({
				mode: "full",
				isOwner: true,
				isMaintainer: true,
			});
		});

		test("maintainer", () => {
			expect(access({ remote: { permission: Maintainer } })).toEqual({
				mode: "full",
				isOwner: false,
				isMaintainer: true,
			});
		});

		test("maintainer who also bought it, with a checkout", () => {
			expect(
				access({ hasLocal: true, remote: { permission: Maintainer | Buyer } }),
			).toEqual({ mode: "full", isOwner: false, isMaintainer: true });
		});
	});

	describe("rule 8: someone else's package + checkout → id_taken", () => {
		test("permission undefined + checkout", () => {
			expect(access({ hasLocal: true })).toEqual({
				mode: "local",
				banner: "id_taken",
			});
		});

		test("user bit + checkout", () => {
			expect(access({ hasLocal: true, remote: { permission: User } })).toEqual({
				mode: "local",
				banner: "id_taken",
			});
		});
	});

	describe("rule 9: no maintainer bit, no checkout → store page", () => {
		test("no access row (permission undefined)", () => {
			expect(access({ remote: { permission: undefined } })).toEqual({
				mode: "redirect",
				href: STORE_HREF,
			});
		});

		test("signed out: the server leaves permission unset", () => {
			expect(access({ auth: "signed_out", remote: {} })).toEqual({
				mode: "redirect",
				href: STORE_HREF,
			});
		});

		test("user bit", () => {
			expect(access({ remote: { permission: User } })).toEqual({
				mode: "redirect",
				href: STORE_HREF,
			});
		});

		test("buyer bit", () => {
			expect(access({ remote: { permission: Buyer } })).toEqual({
				mode: "redirect",
				href: STORE_HREF,
			});
		});

		test("permission 0", () => {
			expect(access({ remote: { permission: 0 } })).toEqual({
				mode: "redirect",
				href: STORE_HREF,
			});
		});

		test("encodes the package id", () => {
			expect(
				access({ packageId: "a&b", remote: { permission: User } }),
			).toEqual({ mode: "redirect", href: "/store/packages?id=a%26b" });
		});
	});

	test("never returns full without a maintainer bit", () => {
		const statuses = [
			"idle",
			"loading",
			"error",
			"forbidden",
			"not_found",
			"ok",
		] as const;
		const permissions = [undefined, 0, User, Buyer, User | Buyer];
		const auths = ["signed_in", "signed_out", "unknown", "expired"] as const;
		for (const status of statuses) {
			for (const source of ["registry", "local", undefined] as const) {
				for (const permission of permissions) {
					for (const hasLocal of [true, false]) {
						for (const localPending of [true, false]) {
							for (const auth of auths) {
								const result = workspaceAccess({
									packageId: "x",
									hasLocal,
									localPending,
									auth,
									remote: { status, source, permission },
								});
								expect(result.mode).not.toBe("full");
							}
						}
					}
				}
			}
		}
	});

	test("full only for a settled registry answer with a maintainer bit, never anonymous", () => {
		const statuses = [
			"idle",
			"loading",
			"error",
			"forbidden",
			"not_found",
			"ok",
		] as const;
		for (const status of statuses) {
			for (const source of ["registry", "local", undefined] as const) {
				for (const auth of [
					"signed_in",
					"signed_out",
					"unknown",
					"expired",
				] as const) {
					for (const hasLocal of [true, false]) {
						const result = workspaceAccess({
							packageId: "x",
							hasLocal,
							auth,
							remote: { status, source, permission: Owner },
						});
						const confirmed =
							status === "ok" && source === "registry" && auth !== "signed_out";
						expect(result.mode === "full").toBe(confirmed);
					}
				}
			}
		}
	});

	test("a maintainer bit never leads to the store page while anything is unsettled", () => {
		for (const status of ["idle", "loading"] as const) {
			for (const auth of ["signed_in", "unknown"] as const) {
				expect(
					access({ auth, remote: { status, permission: Owner } }).mode,
				).toBe("loading");
			}
			expect(
				access({ auth: "expired", remote: { status, permission: Owner } }).mode,
			).toBe("session_expired");
		}
	});
});

describe("workspaceAuthState", () => {
	const user = (expired?: boolean) => ({ user: { expired } });

	test("nothing pushed yet → unknown", () => {
		expect(workspaceAuthState(undefined, user(false))).toBe("unknown");
		expect(workspaceAuthState(undefined, null)).toBe("unknown");
		expect(workspaceAuthState(undefined, { isLoading: true })).toBe("unknown");
	});

	test("settled OIDC without a user answers when the host never pushes", () => {
		expect(
			workspaceAuthState(undefined, { isLoading: false, user: null }),
		).toBe("signed_out");
		expect(
			workspaceAuthState(undefined, { isLoading: false, ...user(false) }),
		).toBe("unknown");
	});

	test("a confirmed session → signed_in, also through a renewal", () => {
		expect(workspaceAuthState(true, user(false))).toBe("signed_in");
		expect(
			workspaceAuthState(true, {
				...user(false),
				isLoading: true,
				activeNavigator: "signinSilent",
			}),
		).toBe("signed_in");
		expect(workspaceAuthState(true, undefined)).toBe("signed_in");
	});

	test("a pushed session whose token has since expired → expired, not a wait", () => {
		expect(workspaceAuthState(true, user(true))).toBe("expired");
		expect(workspaceAuthState(true, { ...user(true), isLoading: false })).toBe(
			"expired",
		);
	});

	test("an expired token being renewed → unknown", () => {
		expect(
			workspaceAuthState(true, {
				...user(true),
				isLoading: true,
				activeNavigator: "signinSilent",
			}),
		).toBe("unknown");
		expect(workspaceAuthState(false, { ...user(true), isLoading: true })).toBe(
			"unknown",
		);
	});

	test("a restored expired user the host reports signed out → expired once settled", () => {
		expect(workspaceAuthState(false, user(true))).toBe("expired");
		expect(workspaceAuthState(false, { ...user(true), isLoading: false })).toBe(
			"expired",
		);
	});

	test("an expired user the host has not pushed yet → unknown", () => {
		expect(workspaceAuthState(undefined, user(true))).toBe("unknown");
	});

	test("a fresh user the host has not pushed as signed in yet → unknown", () => {
		expect(
			workspaceAuthState(false, { isLoading: false, ...user(false) }),
		).toBe("unknown");
	});

	test("OIDC still initialising → unknown", () => {
		expect(workspaceAuthState(false, { isLoading: true, user: null })).toBe(
			"unknown",
		);
		expect(
			workspaceAuthState(false, { activeNavigator: "signinRedirect" }),
		).toBe("unknown");
	});

	test("no stored user and nothing in flight → signed_out", () => {
		expect(workspaceAuthState(false, { isLoading: false, user: null })).toBe(
			"signed_out",
		);
		expect(workspaceAuthState(false, undefined)).toBe("signed_out");
	});
});

describe("remoteStatusOf", () => {
	test("maps query state", () => {
		expect(
			remoteStatusOf({ status: "pending", fetchStatus: "idle", error: null }),
		).toBe("idle");
		expect(
			remoteStatusOf({
				status: "pending",
				fetchStatus: "fetching",
				error: null,
			}),
		).toBe("loading");
		expect(
			remoteStatusOf({ status: "pending", fetchStatus: "paused", error: null }),
		).toBe("loading");
		expect(
			remoteStatusOf({ status: "success", fetchStatus: "idle", error: null }),
		).toBe("ok");
	});

	test("maps errors by HTTP status, coded or not", () => {
		const failed = (error: unknown) =>
			remoteStatusOf({ status: "error", fetchStatus: "idle", error });

		expect(failed({ status: 404 })).toBe("not_found");
		expect(failed({ status: 404, code: "PACKAGE_NOT_FOUND" })).toBe(
			"not_found",
		);
		expect(failed({ status: 403 })).toBe("forbidden");
		expect(failed({ status: 401 })).toBe("error");
		expect(failed({ status: 500 })).toBe("error");
		expect(failed(new Error("Network unavailable"))).toBe("error");
		expect(failed(null)).toBe("error");
	});

	test("a failed refetch keeps a confirmed answer unless the server says 403 or 404", () => {
		const refetchFailed = (error: unknown) =>
			remoteStatusOf({
				status: "error",
				fetchStatus: "idle",
				error,
				hasConfirmedData: true,
			});

		expect(refetchFailed({ status: 500 })).toBe("ok");
		expect(refetchFailed({ status: 401 })).toBe("ok");
		expect(refetchFailed(new Error("Network unavailable"))).toBe("ok");
		expect(refetchFailed({ status: 403 })).toBe("forbidden");
		expect(refetchFailed({ status: 404 })).toBe("not_found");
		expect(
			remoteStatusOf({
				status: "error",
				fetchStatus: "idle",
				error: { status: 500 },
				hasConfirmedData: false,
			}),
		).toBe("error");
	});

	test("errorHttpStatus only reads numeric status", () => {
		expect(errorHttpStatus({ status: "404" })).toBeUndefined();
		expect(errorHttpStatus("404")).toBeUndefined();
		expect(errorHttpStatus({ status: 403 })).toBe(403);
	});
});

describe("workspaceTabs", () => {
	const maintainer = { isOwner: false, isMaintainer: true };

	test("web maintainer: registry tabs only", () => {
		expect(
			workspaceTabs({ hasLocal: false, hasRemote: true, ...maintainer }),
		).toEqual(["overview", "nodes", "listing", "access", "releases"]);
	});

	test("linked checkout + registry: all seven tabs in order", () => {
		expect(
			workspaceTabs({ hasLocal: true, hasRemote: true, ...maintainer }),
		).toEqual([
			"overview",
			"nodes",
			"test",
			"manifest",
			"listing",
			"access",
			"releases",
		]);
	});

	test("desktop remote-only keeps Test and Manifest for Link folder", () => {
		expect(
			workspaceTabs({
				hasLocal: false,
				hasRemote: true,
				canLinkLocal: true,
				...maintainer,
			}),
		).toEqual([
			"overview",
			"nodes",
			"test",
			"manifest",
			"listing",
			"access",
			"releases",
		]);
	});

	test("local only: no registry tabs", () => {
		expect(
			workspaceTabs({
				hasLocal: true,
				hasRemote: false,
				isOwner: false,
				isMaintainer: false,
			}),
		).toEqual(["overview", "nodes", "test", "manifest"]);
	});

	test("registry entry without a maintainer bit: no registry tabs", () => {
		expect(
			workspaceTabs({
				hasLocal: true,
				hasRemote: true,
				isOwner: false,
				isMaintainer: false,
			}),
		).toEqual(["overview", "nodes", "test", "manifest"]);
	});

	test("owner bit alone unlocks the registry tabs", () => {
		expect(
			workspaceTabs({
				hasLocal: false,
				hasRemote: true,
				isOwner: true,
				isMaintainer: false,
			}),
		).toEqual(["overview", "nodes", "listing", "access", "releases"]);
	});
});

function version(
	value: string,
	patch: Partial<PackageVersion> = {},
): PackageVersion {
	return {
		version: value,
		wasmHash: "",
		wasmSize: 0,
		publishedAt: "2026-09-01T00:00:00Z",
		yanked: false,
		...patch,
	};
}

function entry(patch: Partial<RegistryEntry> = {}): RegistryEntry {
	return {
		id: "simple-math",
		manifest: {
			manifestVersion: 1,
			id: "simple-math",
			name: "Simple Math",
			version: "0.3.0",
			description: "Integer math",
			authors: [],
			keywords: ["math"],
			primaryCategory: "DATA_TRANSFORMATION",
			permissions: {} as RegistryEntry["manifest"]["permissions"],
			metadata: {},
		},
		nodes: [],
		versions: [version("0.2.1")],
		status: PackageStatus.Active,
		downloadCount: 212,
		createdAt: "2026-08-01T00:00:00Z",
		updatedAt: "2026-09-01T00:00:00Z",
		source: { type: "remote" },
		verified: false,
		price: 0,
		visibility: "public",
		...patch,
	};
}

function meta(patch: Partial<PackageMeta> = {}): PackageMeta {
	return { id: "simple-math", lang: "en", name: "Simple Math", ...patch };
}

describe("listingHealth", () => {
	test("a bare package misses the thumbnail only", () => {
		expect(listingHealth(entry(), meta())).toEqual({
			missing: ["thumbnail"],
			complete: false,
		});
	});

	test("reports every missing field in order", () => {
		const bare = entry();
		bare.manifest = { ...bare.manifest, description: " ", keywords: [] };
		expect(listingHealth(bare, null).missing).toEqual([
			"thumbnail",
			"description",
			"keywords",
		]);
	});

	test("meta fills the gaps the manifest leaves", () => {
		const bare = entry();
		bare.manifest = { ...bare.manifest, description: "", keywords: [] };
		expect(
			listingHealth(
				bare,
				meta({
					thumbnail: "https://cdn/thumb.png",
					description: "Integer math",
					tags: ["math"],
				}),
			),
		).toEqual({ missing: [], complete: true });
	});
});

describe("overviewChecks", () => {
	test("pending until the host reports", () => {
		expect(overviewChecks({}).map((c) => [c.id, c.status])).toEqual([
			["build", "pending"],
			["lint", "pending"],
		]);
	});

	test("never reports a Tests check", () => {
		expect(overviewChecks({}).map((c) => c.id)).toEqual(["build", "lint"]);
	});

	test("build states", () => {
		const status = (build: Parameters<typeof overviewChecks>[0]["build"]) =>
			overviewChecks({ build })[0].status;
		expect(status({ exists: true, sizeBytes: 38_000 })).toBe("ok");
		expect(status({ exists: true, stale: true })).toBe("warning");
		expect(status({ exists: false })).toBe("error");
		expect(status({ exists: true, failed: true })).toBe("error");
	});

	test("lint states and counts", () => {
		const [, clean] = overviewChecks({ lint: { errors: 0, warnings: 0 } });
		const [, warned] = overviewChecks({ lint: { errors: 0, warnings: 1 } });
		const [, failed] = overviewChecks({ lint: { errors: 2, warnings: 1 } });
		expect(clean.status).toBe("ok");
		expect(warned).toEqual({
			id: "lint",
			status: "warning",
			errors: 0,
			warnings: 1,
		});
		expect(failed.status).toBe("error");
	});
});

describe("versions and state", () => {
	test("live skips pending, rejected, disabled and yanked versions", () => {
		const versions = [
			version("0.4.0", { status: PackageStatus.PendingReview }),
			version("0.3.1", { status: PackageStatus.Rejected }),
			version("0.3.0", { yanked: true }),
			version("0.2.1", { status: PackageStatus.Active }),
		];
		expect(liveVersion(versions)?.version).toBe("0.2.1");
		expect(pendingVersion(versions)?.version).toBe("0.4.0");
	});

	test("registry state", () => {
		expect(registryState(entry())).toBe("live");
		expect(registryState(entry({ status: PackageStatus.PendingReview }))).toBe(
			"in_review",
		);
		expect(
			registryState(
				entry({
					versions: [version("0.1.0", { status: PackageStatus.PendingReview })],
				}),
			),
		).toBe("in_review");
		expect(registryState(entry({ status: PackageStatus.Disabled }))).toBe(
			"disabled",
		);
		expect(registryState(entry({ status: PackageStatus.Rejected }))).toBe(
			"rejected",
		);
	});
});

describe("peopleCounts", () => {
	test("counts each person at their highest role", () => {
		const user = (permission: number): PackageUser => ({
			id: `${permission}`,
			userId: `${permission}`,
			permission,
			grantedAt: "",
		});
		expect(
			peopleCounts([
				user(Owner),
				user(Maintainer | User),
				user(User),
				user(User | Buyer),
				user(Buyer),
			]),
		).toEqual({ owners: 1, maintainers: 1, users: 2, buyers: 1 });
	});
});

describe("storePreviewSummary", () => {
	test("prefers the saved listing over the manifest", () => {
		const summary = storePreviewSummary(
			entry({
				versions: [
					version("0.3.0", { status: PackageStatus.PendingReview }),
					version("0.2.1"),
				],
			}),
			meta({
				name: "Math Helpers",
				description: "Listing text",
				tags: ["numbers"],
				icon: "https://cdn/icon.png",
			}),
			["net.http"],
		);
		expect(summary.name).toBe("Math Helpers");
		expect(summary.description).toBe("Listing text");
		expect(summary.keywords).toEqual(["numbers"]);
		expect(summary.latestVersion).toBe("0.2.1");
		expect(summary.metadata?.icon).toBe("https://cdn/icon.png");
		expect(summary.capabilities).toEqual(["net.http"]);
	});

	test("falls back to the manifest without a listing", () => {
		const summary = storePreviewSummary(entry(), null);
		expect(summary.name).toBe("Simple Math");
		expect(summary.metadata).toBeUndefined();
		expect(summary.keywords).toEqual(["math"]);
	});
});

test("humanizeKey", () => {
	expect(humanizeKey("long_running")).toBe("Long running");
	expect(humanizeKey("minimal")).toBe("Minimal");
	expect(humanizeKey(undefined)).toBeUndefined();
});
