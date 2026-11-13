import type { useTranslation } from "@flow-like/locales";

type Translate = ReturnType<typeof useTranslation>["t"];

export function failureLabel(t: Translate, errorCode: string | null) {
	switch (errorCode) {
		case null:
			return t("botFailureUnknown", "No error code");
		case "BOT_TURN_MODEL_ERROR":
			return t("botFailureModel", "Model error");
		case "BOT_TURN_TIMEOUT":
			return t("botFailureTimeout", "Timed out");
		case "BOT_TURN_QUOTA":
			return t("botFailureQuota", "Budget or quota reached");
		case "BOT_TURN_SETUP":
			return t("botFailureSetup", "Bot setup incomplete");
		case "BOT_TURN_NO_MODEL":
			return t("botFailureNoModel", "No language model");
		case "BOT_TURN_REPLY_REFUSED":
			return t("botFailureReplyRefused", "Reply refused");
		case "BOT_TURN_UNREADABLE":
			return t("botFailureUnreadable", "Message unreadable");
		case "BOT_TURN_EXPIRED":
			return t("botFailureExpired", "Expired before it ran");
		default:
			return errorCode;
	}
}
