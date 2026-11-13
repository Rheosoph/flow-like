import { i18n } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { isMobileDevice, isTauriRuntime } from "./platform";

const OFFER_TOAST_ID = "start-at-login-offer";
const OFFER_TOAST_DURATION_MS = 20_000;

let offerSettled = false;

/** Whether the operating system starts Flow-Like at login; read fresh each time. */
export function getStartAtLogin(): Promise<boolean> {
	return invoke<boolean>("get_start_at_login");
}

/** Answers the operating system's state after the change. */
export function setStartAtLogin(enabled: boolean): Promise<boolean> {
	return invoke<boolean>("set_start_at_login", { enabled });
}

async function acceptOffer(): Promise<void> {
	try {
		if (await setStartAtLogin(true)) {
			toast.success(
				i18n.t("startAtLoginEnabled", "Flow-Like now starts at login"),
			);
		}
	} catch (error) {
		console.warn("[StartAtLogin] Could not turn on start at login:", error);
		toast.error(String(error));
	}
}

async function offer(hostedHere: () => Promise<boolean>): Promise<void> {
	if (offerSettled || !isTauriRuntime() || isMobileDevice()) return;
	if (!(await hostedHere())) return;
	offerSettled = true;
	if (!(await invoke<boolean>("claim_start_at_login_offer"))) return;
	toast.info(
		i18n.t("startAtLoginOfferTitle", "Keep your bots running after a restart?"),
		{
			id: OFFER_TOAST_ID,
			duration: OFFER_TOAST_DURATION_MS,
			description: i18n.t(
				"startAtLoginOfferDescription",
				"Flow-Like can start without a window when you log in, so scheduled tasks of bots on this computer still run after a restart. You can change this in Settings > System.",
			),
			action: {
				label: i18n.t("startAtLoginOfferAction", "Start at login"),
				onClick: () => void acceptOffer(),
			},
		},
	);
}

/**
 * Offers start at login once, the first time a bot hosted on this computer gets a task.
 * `hostedHere` is asked only while the offer is still open.
 */
export function offerStartAtLogin(hostedHere: () => Promise<boolean>): void {
	offer(hostedHere).catch((error) =>
		console.warn("[StartAtLogin] Could not offer start at login:", error),
	);
}
