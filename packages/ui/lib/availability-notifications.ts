import {
	type UserAvailability,
	shouldPresentNotification,
} from "./availability";

/** Account preferences also govern device-only Focus while that account is signed in. */
export function shouldPresentUserNotification(
	account: UserAvailability | undefined,
	device: UserAvailability | undefined,
	priority: "routine" | "mention" | "call",
	now = Date.now(),
): boolean {
	return (
		shouldPresentNotification(account, priority, now) &&
		shouldPresentNotification(
			device
				? { ...device, preferences: account?.preferences ?? device.preferences }
				: undefined,
			priority,
			now,
		)
	);
}

export async function waitForAvailabilityToSettle(
	isLoading: () => boolean,
	timeoutMs = 5_000,
	pollMs = 50,
): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	while (isLoading() && Date.now() < deadline) {
		await new Promise((resolve) => setTimeout(resolve, pollMs));
	}
}
