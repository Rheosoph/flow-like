"use client";

import { useTranslation } from "@flow-like/locales";
import {
	KeyRound,
	LogIn,
	type LucideIcon,
	RefreshCw,
	ServerOff,
	Unplug,
	UserX,
} from "lucide-react";
import { type ComponentType, useEffect, useRef, useState } from "react";
import type {
	DevicesRoute,
	HubErrorCode,
} from "../../../../lib/device-management/model/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { InlineResult } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import { useRouteLink } from "../routing/use-devices-route";

/** SPEC §3.12 whole-area states, first match wins: account, token, then the hub. */
export type AreaGateState =
	| { kind: "loading" }
	| { kind: "signed_out" }
	| { kind: "session_expired" }
	| { kind: "profile_error" }
	| { kind: "token" }
	| { kind: "checking" }
	| { kind: "off" }
	| { kind: "unreachable"; code?: HubErrorCode };

export interface AreaGateProps {
	state: AreaGateState;
	/** Host of the connected hub ("api.flow-like.com"); unknown until the profile is loaded. */
	hub?: string;
	onSignIn?(): void;
	/** Tries again; a returned promise resolves once the attempt has an answer. */
	onRetry?(): unknown;
}

interface GateBodyProps extends Omit<AreaGateProps, "hub"> {
	/** The hub's host, or "this hub". */
	hub: string;
}

const HUB_STATUS: DevicesRoute = { screen: "hub" };

type HubCopy = (t: DevicesT, hub: string) => string;

const refused: HubCopy = (t, hub) =>
	t("devices:shell.gate.unreachable.refused", "{{hub}} refused the request.", {
		hub,
	});

const UNREACHABLE_TITLE = {
	network: (t, hub) =>
		t("devices:shell.gate.unreachable.network", "Can't reach {{hub}}.", {
			hub,
		}),
	timeout: (t, hub) =>
		t(
			"devices:shell.gate.unreachable.timeout",
			"Can't reach {{hub}}: it didn't answer in time.",
			{ hub },
		),
	server_error: (t, hub) =>
		t(
			"devices:shell.gate.unreachable.serverError",
			"{{hub}} answered with a server error.",
			{ hub },
		),
	rate_limited: (t, hub) =>
		t(
			"devices:shell.gate.unreachable.rateLimited",
			"{{hub}} is limiting requests right now.",
			{ hub },
		),
	invalid_response: (t, hub) =>
		t(
			"devices:shell.gate.unreachable.invalidResponse",
			"{{hub}} sent an answer this app can't read.",
			{ hub },
		),
	not_found: (t, hub) =>
		t(
			"devices:shell.gate.unreachable.notFound",
			"{{hub}} doesn't say whether it supports devices.",
			{ hub },
		),
	unauthorized: refused,
	forbidden: refused,
	token_restricted: refused,
} satisfies Record<HubErrorCode, HubCopy>;

/** Retry with a visible result: "Still unreachable at 14:02:10" (IA §6.7 S03). */
function RetryGate({
	icon,
	title,
	text,
	stillText,
	onRetry,
}: Readonly<{
	icon: LucideIcon;
	title: string;
	text: string;
	stillText(time: string): string;
	onRetry?: AreaGateProps["onRetry"];
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const [busy, setBusy] = useState(false);
	const [failedAt, setFailedAt] = useState<number | null>(null);
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);

	const retry = async () => {
		if (busy) return;
		setBusy(true);
		try {
			await onRetry?.();
		} catch {
			// The gate reports the outcome itself: it stays, with the time of this attempt.
		}
		if (!mounted.current) return;
		setBusy(false);
		setFailedAt(Date.now() / 1000);
	};

	return (
		<StateView
			kind="gate"
			icon={icon}
			title={title}
			text={text}
			actions={
				<>
					<DvButton icon={RefreshCw} busy={busy} onClick={() => void retry()}>
						{t("shell.gate.retry", "Retry")}
					</DvButton>
					{failedAt !== null && !busy ? (
						<InlineResult tone="warning">
							{stillText(time.clock(failedAt))}
						</InlineResult>
					) : null}
				</>
			}
		/>
	);
}

function LoadingGate() {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="loading"
			rows={4}
			title={t("shell.gate.loading", "Loading your account…")}
		/>
	);
}

function SignedOutGate({ onSignIn }: Readonly<GateBodyProps>) {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="gate"
			icon={LogIn}
			title={t("shell.gate.signIn.title", "Sign in to manage devices")}
			text={t(
				"shell.gate.signIn.text",
				"Devices, their keys and who can reach them belong to your Flow-Like account.",
			)}
			actions={
				<DvButton variant="primary" icon={LogIn} onClick={onSignIn}>
					{t("shell.gate.signIn.action", "Sign in")}
				</DvButton>
			}
		/>
	);
}

function SessionExpiredGate({ onSignIn }: Readonly<GateBodyProps>) {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="gate"
			icon={LogIn}
			title={t("shell.gate.expired.title", "Your sign-in has expired.")}
			text={t(
				"shell.gate.expired.text",
				"Sign in again to see and manage your devices.",
			)}
			actions={
				<DvButton variant="primary" icon={LogIn} onClick={onSignIn}>
					{t("shell.gate.expired.action", "Sign in again")}
				</DvButton>
			}
		/>
	);
}

function TokenGate({ onSignIn }: Readonly<GateBodyProps>) {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="gate"
			icon={KeyRound}
			title={t(
				"shell.gate.token.title",
				"Your access token can't manage devices.",
			)}
			text={t(
				"shell.gate.token.text",
				"Use a token with full permissions. Tokens limited to some apps or to reading can't list or manage devices.",
			)}
			actions={
				<DvButton onClick={onSignIn}>
					{t("shell.gate.token.action", "Sign in with full permissions")}
				</DvButton>
			}
		/>
	);
}

function ProfileErrorGate({ onRetry }: Readonly<GateBodyProps>) {
	const { t } = useTranslation("devices");
	return (
		<RetryGate
			icon={UserX}
			title={t(
				"shell.gate.profile.title",
				"Your account profile couldn't be loaded.",
			)}
			text={t(
				"shell.gate.profile.text",
				"Devices are listed per account, hub and profile, so nothing can be shown without it.",
			)}
			stillText={(time) =>
				t("shell.gate.profile.still", "Still couldn't load it at {{time}}", {
					time,
				})
			}
			onRetry={onRetry}
		/>
	);
}

function CheckingGate({ hub }: Readonly<GateBodyProps>) {
	const { t } = useTranslation("devices");
	const label = t("shell.gate.checking", "Checking {{hub}}…", { hub });
	return (
		<>
			<p className="text-ui text-muted-foreground">{label}</p>
			<StateView kind="loading" rows={4} title={label} />
		</>
	);
}

function OffGate({ hub }: Readonly<GateBodyProps>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	return (
		<StateView
			kind="gate"
			icon={ServerOff}
			title={t("shell.gate.off.title", "Devices aren't enabled on {{hub}}.", {
				hub,
			})}
			text={t(
				"shell.gate.off.text",
				"Ask the hub operator to turn on device support. Apps, flows and everything else in Flow-Like keep working.",
			)}
			actions={
				<DvButton asChild>
					<a {...link(HUB_STATUS)}>
						{t("shell.gate.off.action", "Open hub status")}
					</a>
				</DvButton>
			}
		/>
	);
}

function UnreachableGate({ state, hub, onRetry }: Readonly<GateBodyProps>) {
	const { t } = useTranslation("devices");
	const code = state.kind === "unreachable" ? state.code : undefined;
	return (
		<RetryGate
			icon={Unplug}
			title={UNREACHABLE_TITLE[code ?? "network"](t, hub)}
			text={t(
				"shell.gate.unreachable.text",
				"The app keeps retrying. Your devices keep running; nothing on them depends on this computer reaching the hub.",
			)}
			stillText={(time) =>
				t("shell.gate.unreachable.still", "Still unreachable at {{time}}", {
					time,
				})
			}
			onRetry={onRetry}
		/>
	);
}

const GATES: Record<AreaGateState["kind"], ComponentType<GateBodyProps>> = {
	loading: LoadingGate,
	signed_out: SignedOutGate,
	session_expired: SessionExpiredGate,
	token: TokenGate,
	profile_error: ProfileErrorGate,
	checking: CheckingGate,
	off: OffGate,
	unreachable: UnreachableGate,
};

/** Replaces the page content while a whole-area condition fails; top bar and status bar stay. */
export function AreaGate({ hub, ...props }: Readonly<AreaGateProps>) {
	const { t } = useTranslation("devices");
	const Body = GATES[props.state.kind];
	return (
		<div data-area-gate={props.state.kind} className="flex flex-col gap-3">
			<Body {...props} hub={hub ?? t("shell.gate.thisHub", "this hub")} />
		</div>
	);
}
