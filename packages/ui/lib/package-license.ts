import type { AppPackage, PackageSummary } from "./schema/wasm";

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

/** `stale` is a pin flagged by a server that predates project licensing. */
export type PackagePinState = "active" | "lapsed" | "expired" | "stale";

export interface LicenseTimeLeft {
	remainingMs: number;
	/** Whole days left; 0 once less than a day remains. */
	days: number;
	/** Hours left, rounded up so the last hour still reads "1 hour". */
	hours: number;
}

type PinLicenseFields = Pick<AppPackage, "stale" | "license">;

export function licenseExpiresAt(pkg: PinLicenseFields): number | undefined {
	const license = pkg.license;
	if (!license) return undefined;
	const expires = license.expiresAt
		? Date.parse(license.expiresAt)
		: Number.NaN;
	if (!Number.isNaN(expires)) return expires;
	const lapsed = license.lapsedAt ? Date.parse(license.lapsedAt) : Number.NaN;
	return Number.isNaN(lapsed) ? undefined : lapsed + license.graceDays * DAY_MS;
}

/** A lapsed pin whose grace ran out since the last fetch already counts as expired. */
export function packagePinState(
	pkg: PinLicenseFields,
	now: number = Date.now(),
): PackagePinState {
	const status = pkg.license?.status;
	if (!status) return pkg.stale ? "stale" : "active";
	if (status !== "lapsed") return status;
	const expires = licenseExpiresAt(pkg);
	return expires !== undefined && expires <= now ? "expired" : "lapsed";
}

export function isExpiredPin(pkg: Pick<AppPackage, "license">): boolean {
	return pkg.license?.status === "expired";
}

export function licenseTimeLeft(
	expiresAt: number | undefined,
	now: number,
): LicenseTimeLeft | undefined {
	if (expiresAt === undefined) return undefined;
	const remainingMs = Math.max(0, expiresAt - now);
	return {
		remainingMs,
		days: Math.floor(remainingMs / DAY_MS),
		hours: Math.ceil(remainingMs / HOUR_MS),
	};
}

/** Paid, private and request-access packages need a holder before a project may pin them. */
export function packageNeedsLicense(
	pkg: Pick<PackageSummary, "price" | "visibility">,
): boolean {
	return (
		pkg.price > 0 ||
		pkg.visibility === "private" ||
		pkg.visibility === "public_request_access"
	);
}

export function formatPackagePrice(cents: number): string {
	return `€${(cents / 100).toFixed(2)}`;
}

export function storePackageHref(packageId: string): string {
	return `/store/packages?id=${encodeURIComponent(packageId)}`;
}
