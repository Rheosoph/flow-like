import { asArray } from "../../lib/response-shape";
import type { IBlockedPackage, IForkReport } from "../../lib/schema/app/fork";
import { paymentMoney } from "../payments/types";

/** A package a fork left behind, in the shape FlowPilot reports to the user. */
export interface LeftOutPackage {
	package_id: string;
	reason: string;
	/**
	 * What the user can do about it. Absent when this app offers no way, or
	 * when the preview that would say so was not available.
	 */
	obtain?: "buy" | "request_access" | "wait_for_approval";
	price?: string;
}

const LEFT_OUT_LIMIT = 20;

const LEFT_OUT_PACKAGES_NOTE =
	"The fork was created WITHOUT the packages in left_out_packages. Flows and page widgets built on them do not work in the new app until the package is added. Tell the user which packages are missing; `reason` says why each was left out. `obtain`, where present, says what they can do: `buy` it in the store for `price`, `request_access` from its author, or `wait_for_approval` because they already asked and the author has not answered. After that they add the package on the new app's Packages page. Where `obtain` is absent, go by `reason` and do not promise a way to get the package. Do not describe the fork as a complete copy.";

function obtainable(
	pkg: IBlockedPackage | undefined,
	purchasingAllowed: boolean,
): Pick<LeftOutPackage, "obtain" | "price"> {
	if (pkg?.block === "paid" && purchasingAllowed)
		return { obtain: "buy", price: paymentMoney(pkg.price, "eur", "en") };
	if (pkg?.block === "request_access")
		return {
			obtain: pkg.request_pending ? "wait_for_approval" : "request_access",
		};
	return {};
}

/**
 * The packages a finished fork skipped. The report says that they were
 * skipped; the preview taken before the fork says how each can be obtained.
 * `purchasingAllowed` is false on builds that may not sell anything.
 */
export function leftOutPackages(
	skipped: IForkReport["skipped"] | null | undefined,
	blocked: readonly IBlockedPackage[] | null | undefined,
	purchasingAllowed: boolean,
): LeftOutPackage[] {
	const blockedById = new Map(
		asArray(blocked).map((pkg) => [pkg.package_id, pkg]),
	);
	return asArray(skipped)
		.filter((item) => item.kind === "Package")
		.map((item) => ({
			package_id: item.source_id,
			reason: item.reason,
			...obtainable(blockedById.get(item.source_id), purchasingAllowed),
		}));
}

/**
 * What a fork tool result adds when the fork skipped packages. They get their
 * own bounded list, apart from `skipped`, which a damaged source app can fill
 * with other entries before a package is reached.
 */
export function leftOutPackagesResult(
	skipped: IForkReport["skipped"] | null | undefined,
	blocked: readonly IBlockedPackage[] | null | undefined,
	purchasingAllowed: boolean,
): Record<string, unknown> {
	const leftOut = leftOutPackages(skipped, blocked, purchasingAllowed);
	if (leftOut.length === 0) return {};
	return {
		left_out_packages: leftOut.slice(0, LEFT_OUT_LIMIT),
		left_out_packages_total: leftOut.length,
		note: LEFT_OUT_PACKAGES_NOTE,
	};
}
