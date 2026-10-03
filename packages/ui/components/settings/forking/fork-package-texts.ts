import { useTranslation } from "@flow-like/locales";

/**
 * What a paid row says before and while its checkout is open. `inCopy` is the
 * wording for a copy that already exists and gets the package added to it.
 */
export function usePaidDescription(pending: boolean, inCopy: boolean): string {
	const { t } = useTranslation("settings");
	if (inCopy && pending)
		return t(
			"forkCopyPackageAwaitingPayment",
			"Finish checkout in your browser. Once the payment is confirmed, the package is added to your copy.",
		);
	if (inCopy)
		return t(
			"forkCopyPackagePaid",
			"You don't own this package. Buy it to add it to your copy.",
		);
	if (pending)
		return t(
			"forkPackageAwaitingPayment",
			"Finish checkout in your browser. Once the payment is confirmed, the package comes with your fork. If you fork before that, it's left out.",
		);
	return t(
		"forkPackagePaid",
		"You don't own this package. Buy it before forking to keep it in your copy.",
	);
}

/** What a request-access row says before and after the request was sent. */
export function useRequestDescription(
	requested: boolean,
	inCopy: boolean,
): string {
	const { t } = useTranslation("settings");
	if (inCopy && requested)
		return t(
			"forkCopyPackageRequestPending",
			"Request sent. Once the author approves it, the package is added to your copy.",
		);
	if (inCopy)
		return t(
			"forkCopyPackageRequestAccess",
			"Its author decides who can use it. Request access to add it to your copy.",
		);
	if (requested)
		return t(
			"forkPackageRequestPending",
			"Request sent. Until the author approves it, the package is left out of your fork. You can add it later.",
		);
	return t(
		"forkPackageRequestAccess",
		"Its author decides who can use it. Request access before forking to keep it in your copy.",
	);
}
