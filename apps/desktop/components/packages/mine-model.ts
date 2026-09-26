import type { WorkspaceTab } from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-href";
import { isMaintainer } from "@flow-like/flow-like-ui/lib/permission/wasm-package-permission";
import type { DeveloperProject } from "@flow-like/flow-like-ui/lib/schema/developer";
import {
	PackageStatus,
	type PackageSummary,
} from "@flow-like/flow-like-ui/lib/schema/wasm";

export type MineState =
	| "local-only"
	| "unpublished-changes"
	| "in-review"
	| "live"
	| "not-on-this-machine"
	| "disabled";

export type MineFilter = "all" | MineState | "issues";

export type MineSort = "attention" | "name";

export const MINE_FILTERS: readonly MineFilter[] = [
	"all",
	"unpublished-changes",
	"in-review",
	"live",
	"local-only",
	"not-on-this-machine",
	"disabled",
	"issues",
];

/** Every template ships an id under this prefix. */
const PLACEHOLDER_ID_PREFIX = "com.example.";

export function isPlaceholderId(id: string | null | undefined): boolean {
	return !!id && id.trim().toLowerCase().startsWith(PLACEHOLDER_ID_PREFIX);
}

export interface MineManifest {
	id: string;
	name?: string;
	version?: string;
	description?: string;
	keywords?: string[];
}

export interface MineLintCounts {
	errors: number;
	warnings: number;
}

export interface MineProjectInput {
	project: DeveloperProject;
	manifest?: MineManifest | null;
	lint?: MineLintCounts | null;
	stale?: boolean;
}

/** `null` when the registry could not be asked (signed out, offline). */
export interface MineRegistryInput {
	/** `access=maintainer` results. */
	maintained: readonly PackageSummary[];
	/** `ids=` lookup of local manifest ids missing from `maintained`. */
	lookedUp: readonly PackageSummary[];
}

export type MineIssue =
	| { kind: "placeholder-id" }
	| { kind: "id-taken" }
	| { kind: "lint"; errors: number }
	| { kind: "stale" };

export type MineIssueTone = "error" | "warning";

export function mineIssueTone(issue: MineIssue): MineIssueTone {
	return issue.kind === "id-taken" || issue.kind === "lint"
		? "error"
		: "warning";
}

export interface MineCheckout {
	projectId: string;
	path: string;
	name: string;
	language: string;
	version: string | null;
	newest: boolean;
	matchesLive: boolean;
	lintErrors: number;
	lintWarnings: number;
	inspected: boolean;
	stale: boolean;
}

export type MineAction =
	| { kind: "choose-id"; path: string }
	| { kind: "rename-id"; path: string }
	| { kind: "fix-errors"; path: string; errors: number }
	| { kind: "reload"; path: string }
	| { kind: "publish"; path: string; version: string | null; first: boolean }
	| { kind: "view-review"; packageId: string }
	| { kind: "view-releases"; packageId: string }
	| { kind: "open"; packageId: string }
	| { kind: "link-folder"; packageId: string };

/** The workspace tab an action opens; null when it runs in place or leaves the workspace. */
export function mineActionTab(action: MineAction): WorkspaceTab | null {
	switch (action.kind) {
		case "choose-id":
		case "rename-id":
			return "manifest";
		case "fix-errors":
			return "nodes";
		case "view-review":
		case "view-releases":
			return "releases";
		case "open":
			return "overview";
		default:
			return null;
	}
}

/**
 * The Mine action the workspace header offers for the open checkout, or null.
 * Only the newest checkout gets it (the action is derived from that one), never
 * on the tab it would open, and on Releases only "Publish", since that tab
 * shows the same slot as its "Next release".
 */
export function workspaceHeaderAction(
	entry: MineEntry,
	checkoutPath: string,
	activeTab: WorkspaceTab,
): MineAction | null {
	const action = entry.primaryAction;
	if (action.kind === "open" || action.kind === "link-folder") return null;
	const newest = entry.checkouts[0];
	if (!newest || !sameCheckoutPath(newest.path, checkoutPath)) return null;
	if (mineActionTab(action) === activeTab) return null;
	if (activeTab === "releases" && action.kind !== "publish") return null;
	return action;
}

export interface MineOptions {
	/** List disabled packages that have no checkout here; a checkout always shows its package as disabled. */
	includeDisabled?: boolean;
}

export interface MineEntry {
	key: string;
	packageId: string | null;
	name: string;
	description: string;
	keywords: string[];
	language: string | null;
	state: MineState;
	localVersion: string | null;
	liveVersion: string | null;
	registry: PackageSummary | null;
	checkouts: MineCheckout[];
	issues: MineIssue[];
	idTaken: boolean;
	primaryAction: MineAction;
	searchText: string;
}

export interface MineModel {
	entries: MineEntry[];
	counts: Record<MineFilter, number>;
}

interface ParsedVersion {
	core: [number, number, number];
	pre: string[];
}

const VERSION_PATTERN =
	/^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/;
const NUMERIC = /^\d+$/;

function parseVersion(version: string): ParsedVersion | null {
	const match = VERSION_PATTERN.exec(version.trim());
	if (!match) return null;
	return {
		core: [Number(match[1]), Number(match[2] ?? 0), Number(match[3] ?? 0)],
		pre: match[4] ? match[4].split(".") : [],
	};
}

function comparePrerelease(a: string[], b: string[]): number {
	if (a.length === 0 || b.length === 0) return b.length - a.length;
	for (let i = 0; i < Math.max(a.length, b.length); i++) {
		if (i >= a.length) return -1;
		if (i >= b.length) return 1;
		const left = a[i];
		const right = b[i];
		if (left === right) continue;
		const leftNumeric = NUMERIC.test(left);
		const rightNumeric = NUMERIC.test(right);
		if (leftNumeric && rightNumeric) return Number(left) - Number(right);
		if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1;
		return left < right ? -1 : 1;
	}
	return 0;
}

/** Semver precedence; `null` when either side is not a version. */
export function compareVersions(a: string, b: string): number | null {
	const left = parseVersion(a);
	const right = parseVersion(b);
	if (!left || !right) return null;
	for (let i = 0; i < 3; i++) {
		const diff = left.core[i] - right.core[i];
		if (diff !== 0) return Math.sign(diff);
	}
	return Math.sign(comparePrerelease(left.pre, right.pre));
}

function isNewer(version: string | null, than: string | null): boolean {
	if (!version || !than) return false;
	return (compareVersions(version, than) ?? 0) > 0;
}

function sameVersion(a: string | null, b: string | null): boolean {
	if (!a || !b) return false;
	return compareVersions(a, b) === 0;
}

function byVersionDesc(a: MineProjectInput, b: MineProjectInput): number {
	const left = a.manifest?.version ?? null;
	const right = b.manifest?.version ?? null;
	if (left === right) return 0;
	if (!left) return 1;
	if (!right) return -1;
	return -(compareVersions(left, right) ?? 0);
}

function canMaintain(pkg: PackageSummary): boolean {
	return (
		pkg.viewerPermission === undefined || isMaintainer(pkg.viewerPermission)
	);
}

function manifestId(input: MineProjectInput): string | null {
	const id = input.manifest?.id?.trim();
	return id ? id : null;
}

function groupKey(input: MineProjectInput): string {
	return manifestId(input) ?? `project:${input.project.id}`;
}

function registryName(pkg: PackageSummary): string {
	return pkg.metadata?.name ?? pkg.name;
}

function registryDescription(pkg: PackageSummary): string {
	return pkg.metadata?.description ?? pkg.description;
}

function toCheckout(
	input: MineProjectInput,
	index: number,
	total: number,
	liveVersion: string | null,
): MineCheckout {
	const version = input.manifest?.version ?? null;
	return {
		projectId: input.project.id,
		path: input.project.path,
		name: input.manifest?.name ?? input.project.name,
		language: input.project.language,
		version,
		newest: total > 1 && index === 0,
		matchesLive: sameVersion(version, liveVersion),
		lintErrors: input.lint?.errors ?? 0,
		lintWarnings: input.lint?.warnings ?? 0,
		inspected: Boolean(input.lint),
		stale: Boolean(input.stale),
	};
}

function isDisabled(pkg: PackageSummary): boolean {
	return pkg.status === PackageStatus.Disabled;
}

function localState(
	registry: PackageSummary | null,
	localVersion: string | null,
): MineState {
	if (!registry) return "local-only";
	if (isDisabled(registry)) return "disabled";
	if (registry.status === PackageStatus.PendingReview) return "in-review";
	if (isNewer(localVersion, registry.latestVersion))
		return "unpublished-changes";
	return "live";
}

/** A placeholder id is flagged before id-taken: the fix is the same, and the id was never meant to be kept. */
function localIssues(
	placeholder: boolean,
	idTaken: boolean,
	newest: MineCheckout,
	checkouts: readonly MineCheckout[],
): MineIssue[] {
	const issues: MineIssue[] = [];
	if (placeholder) issues.push({ kind: "placeholder-id" });
	else if (idTaken) issues.push({ kind: "id-taken" });
	if (newest.lintErrors > 0)
		issues.push({ kind: "lint", errors: newest.lintErrors });
	if (checkouts.some((checkout) => checkout.stale))
		issues.push({ kind: "stale" });
	return issues;
}

function localAction(
	state: MineState,
	issues: readonly MineIssue[],
	newest: MineCheckout,
	packageId: string | null,
): MineAction {
	if (packageId && state === "disabled")
		return { kind: "view-releases", packageId };
	if (issues.some((issue) => issue.kind === "placeholder-id"))
		return { kind: "choose-id", path: newest.path };
	if (issues.some((issue) => issue.kind === "id-taken"))
		return { kind: "rename-id", path: newest.path };
	const lint = issues.find((issue) => issue.kind === "lint");
	if (lint?.kind === "lint")
		return { kind: "fix-errors", path: newest.path, errors: lint.errors };
	if (issues.some((issue) => issue.kind === "stale"))
		return { kind: "reload", path: newest.path };
	if (packageId && state === "in-review")
		return { kind: "view-review", packageId };
	if (packageId && state === "live") return { kind: "open", packageId };
	return {
		kind: "publish",
		path: newest.path,
		version: newest.version,
		first: state === "local-only",
	};
}

function localEntry(
	key: string,
	inputs: MineProjectInput[],
	maintained: ReadonlyMap<string, PackageSummary>,
	foreign: ReadonlyMap<string, PackageSummary>,
): MineEntry {
	const ordered = [...inputs].sort(byVersionDesc);
	const head = ordered[0];
	const packageId = manifestId(head);
	const registry = packageId ? (maintained.get(packageId) ?? null) : null;
	const idTaken = Boolean(packageId && !registry && foreign.has(packageId));
	const placeholder = !registry && isPlaceholderId(packageId);
	const liveVersion = registry?.latestVersion ?? null;
	const checkouts = ordered.map((input, index) =>
		toCheckout(input, index, ordered.length, liveVersion),
	);
	const newest = checkouts[0];
	const localVersion = newest.version;
	const state = localState(registry, localVersion);
	const issues = localIssues(placeholder, idTaken, newest, checkouts);
	const name = registry ? registryName(registry) : newest.name;

	return {
		key,
		packageId,
		name,
		description: registry
			? registryDescription(registry)
			: (head.manifest?.description ?? ""),
		keywords: registry?.keywords ?? head.manifest?.keywords ?? [],
		language: newest.language,
		state,
		localVersion,
		liveVersion,
		registry,
		checkouts,
		issues,
		idTaken,
		primaryAction: localAction(state, issues, newest, packageId),
		searchText: [
			name,
			packageId ?? "",
			...checkouts.map((checkout) => checkout.path),
		].join(" "),
	};
}

function remoteEntry(pkg: PackageSummary): MineEntry {
	const name = registryName(pkg);
	const disabled = isDisabled(pkg);
	return {
		key: pkg.id,
		packageId: pkg.id,
		name,
		description: registryDescription(pkg),
		keywords: pkg.keywords ?? [],
		language: null,
		state: disabled ? "disabled" : "not-on-this-machine",
		localVersion: null,
		liveVersion: pkg.latestVersion,
		registry: pkg,
		checkouts: [],
		issues: [],
		idTaken: false,
		primaryAction: disabled
			? { kind: "view-releases", packageId: pkg.id }
			: { kind: "link-folder", packageId: pkg.id },
		searchText: `${name} ${pkg.id}`,
	};
}

function splitRegistry(
	registry: MineRegistryInput | null,
	localIds: ReadonlySet<string>,
) {
	const maintained = new Map<string, PackageSummary>();
	const foreign = new Map<string, PackageSummary>();
	for (const pkg of registry?.maintained ?? []) {
		if (canMaintain(pkg)) maintained.set(pkg.id, pkg);
	}
	for (const pkg of registry?.lookedUp ?? []) {
		if (!localIds.has(pkg.id) || maintained.has(pkg.id)) continue;
		if (
			pkg.viewerPermission !== undefined &&
			isMaintainer(pkg.viewerPermission)
		) {
			maintained.set(pkg.id, pkg);
		} else foreign.set(pkg.id, pkg);
	}
	return { maintained, foreign };
}

export function matchesMineFilter(
	entry: MineEntry,
	filter: MineFilter,
): boolean {
	if (filter === "all") return true;
	if (filter === "issues") return entry.issues.length > 0;
	return entry.state === filter;
}

export function countMine(
	entries: readonly MineEntry[],
): Record<MineFilter, number> {
	const counts = Object.fromEntries(
		MINE_FILTERS.map((filter) => [filter, 0]),
	) as Record<MineFilter, number>;
	for (const entry of entries) {
		for (const filter of MINE_FILTERS) {
			if (matchesMineFilter(entry, filter)) counts[filter] += 1;
		}
	}
	return counts;
}

const STATE_ATTENTION: readonly MineState[] = [
	"unpublished-changes",
	"in-review",
	"live",
	"not-on-this-machine",
	"local-only",
	"disabled",
];

function attentionRank(entry: MineEntry): number {
	if (entry.issues.some((issue) => mineIssueTone(issue) === "error")) return 0;
	if (entry.issues.length > 0) return 1;
	return 2 + STATE_ATTENTION.indexOf(entry.state);
}

function byName(a: MineEntry, b: MineEntry): number {
	return (
		a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) ||
		a.key.localeCompare(b.key)
	);
}

export function sortMine(
	entries: readonly MineEntry[],
	sort: MineSort,
): MineEntry[] {
	return [...entries].sort((a, b) =>
		sort === "name"
			? byName(a, b)
			: attentionRank(a) - attentionRank(b) || byName(a, b),
	);
}

export function deriveMine(
	projects: readonly MineProjectInput[],
	registry: MineRegistryInput | null,
	{ includeDisabled = false }: MineOptions = {},
): MineModel {
	const groups = new Map<string, MineProjectInput[]>();
	for (const input of projects) {
		const key = groupKey(input);
		groups.set(key, [...(groups.get(key) ?? []), input]);
	}

	const localIds = new Set(
		projects.flatMap((input) => {
			const id = manifestId(input);
			return id ? [id] : [];
		}),
	);
	const { maintained, foreign } = splitRegistry(registry, localIds);

	const entries = [
		...[...groups].map(([key, inputs]) =>
			localEntry(key, inputs, maintained, foreign),
		),
		...[...maintained.values()]
			.filter(
				(pkg) => !localIds.has(pkg.id) && (includeDisabled || !isDisabled(pkg)),
			)
			.map(remoteEntry),
	];

	return {
		entries: sortMine(entries, "attention"),
		counts: countMine(entries),
	};
}

/** Local manifest ids the maintainer listing did not cover. */
export function idsToLookUp(
	projects: readonly MineProjectInput[],
	maintained: readonly PackageSummary[],
	limit = 100,
): string[] {
	const known = new Set(maintained.map((pkg) => pkg.id));
	const ids = new Set<string>();
	for (const input of projects) {
		const id = manifestId(input);
		if (id && !known.has(id)) ids.add(id);
	}
	return [...ids].sort().slice(0, limit);
}

function trimPath(path: string): string {
	return path.replace(/[\\/]+$/, "");
}

export function sameCheckoutPath(a: string, b: string): boolean {
	return trimPath(a) === trimPath(b);
}

/** The entry a workspace link points at: the checkout when `project` is set, otherwise the package id. */
export function findWorkspaceEntry(
	entries: readonly MineEntry[],
	{ id, project }: { id?: string | null; project?: string | null },
): MineEntry | undefined {
	if (project) {
		return entries.find((entry) =>
			entry.checkouts.some((checkout) =>
				sameCheckoutPath(checkout.path, project),
			),
		);
	}
	if (id) return entries.find((entry) => entry.packageId === id);
	return undefined;
}

/** What the card shows for a package that exists only on this machine. */
export function localSummary(entry: MineEntry): PackageSummary {
	return (
		entry.registry ?? {
			id: entry.packageId ?? entry.key,
			name: entry.name,
			description: entry.description,
			latestVersion: entry.localVersion ?? "",
			downloadCount: 0,
			status: PackageStatus.Active,
			keywords: entry.keywords,
			verified: false,
			price: 0,
			visibility: "public",
		}
	);
}
