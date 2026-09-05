"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2, Mail } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import type { IUserInfo } from "../../state/backend-state/user-state";
import { Alert, AlertDescription } from "../ui/alert";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { accountError, invalidateAccountIdentity } from "./account-model";

export interface ChangeEmailDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	updateEmail: (
		email: string,
	) => Promise<{ needsVerification: boolean; destination?: string }>;
	verifyEmail: (code: string) => Promise<void>;
	resendCode: () => Promise<unknown>;
}

export default function ChangeEmailDialog({
	open,
	onOpenChange,
	updateEmail,
	verifyEmail,
	resendCode,
}: ChangeEmailDialogProps) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const client = useQueryClient();
	const info = useInvoke(backend.userState.getInfo, backend.userState, []);
	const [email, setEmail] = useState("");
	const [code, setCode] = useState("");
	const [destination, setDestination] = useState("");
	const [step, setStep] = useState<"email" | "verification">("email");
	const [pending, setPending] = useState<"submit" | "resend" | null>(null);
	const busy = useRef(false);
	const [error, setError] = useState("");
	const [notice, setNotice] = useState("");
	const [resendAfter, setResendAfter] = useState(0);
	useEffect(() => {
		if (resendAfter <= 0) return;
		const timer = setTimeout(
			() => setResendAfter((remaining) => remaining - 1),
			1000,
		);
		return () => clearTimeout(timer);
	}, [resendAfter]);

	function close() {
		if (busy.current) return;
		setError("");
		setNotice("");
		// Keep a pending verification so reopening can resume it.
		if (step === "email") {
			setEmail("");
			setCode("");
		}
		onOpenChange(false);
	}

	async function complete() {
		client.setQueryData<IUserInfo>(
			[backend.userState.getInfo.name],
			(current) => (current ? { ...current, email: email.trim() } : current),
		);
		setStep("email");
		setEmail("");
		setCode("");
		setDestination("");
		setResendAfter(0);
		await invalidateAccountIdentity(client);
		toast.success(t("accountEmailUpdated", "Email address updated."));
		onOpenChange(false);
	}

	async function submit(event: React.FormEvent) {
		event.preventDefault();
		if (busy.current) return;
		const next = email.trim();
		if (
			step === "email" &&
			(!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(next) ||
				next.toLowerCase() === info.data?.email?.trim().toLowerCase())
		) {
			setError(
				t(
					"accountEmailDifferent",
					"Enter a valid email address different from your current address.",
				),
			);
			return;
		}
		if (step === "verification" && !code.trim()) {
			setError(t("accountCodeRequired", "Enter the confirmation code."));
			return;
		}
		busy.current = true;
		setPending("submit");
		setError("");
		setNotice("");
		try {
			if (step === "verification") {
				await verifyEmail(code.trim());
				await complete();
			} else {
				const result = await updateEmail(next);
				setEmail(next);
				if (result.needsVerification) {
					setStep("verification");
					setDestination(result.destination || next);
					setCode("");
					setResendAfter(30);
				} else await complete();
			}
		} catch (cause) {
			setError(
				accountError(
					cause,
					t(
						"accountEmailFailed",
						"The email change could not be completed. Try again.",
					),
				),
			);
		} finally {
			busy.current = false;
			setPending(null);
		}
	}

	async function resend() {
		if (busy.current || resendAfter > 0) return;
		busy.current = true;
		setPending("resend");
		setError("");
		setNotice("");
		try {
			await resendCode();
			setResendAfter(30);
			setNotice(
				t("accountCodeResent", "A new confirmation code has been sent."),
			);
		} catch (cause) {
			setError(
				accountError(
					cause,
					t(
						"accountCodeResendFailed",
						"A new code could not be sent. Try again.",
					),
				),
			);
		} finally {
			busy.current = false;
			setPending(null);
		}
	}

	return (
		<Dialog
			open={open}
			onOpenChange={(value) => {
				if (!value) close();
			}}
		>
			<DialogContent className="sm:max-w-md" showCloseButton={!pending}>
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Mail className="size-5" />
						{t("changeEmailAddress", "Change email address")}
					</DialogTitle>
					<DialogDescription>
						{step === "email"
							? t(
									"accountEmailHelp",
									"Enter an address you can access. You may need to verify it before the change is complete.",
								)
							: t(
									"accountVerifyEmailHelp",
									"Enter the confirmation code sent to {{email}}.",
									{ email: destination },
								)}
					</DialogDescription>
				</DialogHeader>
				<form
					className="space-y-4"
					onSubmit={submit}
					aria-busy={Boolean(pending)}
				>
					{error && (
						<Alert variant="destructive" role="alert">
							<AlertDescription>{error}</AlertDescription>
						</Alert>
					)}
					{notice && (
						<p aria-live="polite" className="text-sm text-muted-foreground">
							{notice}
						</p>
					)}
					{step === "email" ? (
						<>
							<div className="space-y-2">
								<Label htmlFor="account-current-email">
									{t("currentEmail", "Current email")}
								</Label>
								<Input
									id="account-current-email"
									value={info.data?.email ?? ""}
									readOnly
									className="bg-muted/40"
								/>
							</div>
							<div className="space-y-2">
								<Label htmlFor="account-new-email">
									{t("newEmailAddress", "New email address")}
								</Label>
								<Input
									id="account-new-email"
									name="email"
									type="email"
									autoComplete="email"
									required
									disabled={Boolean(pending)}
									value={email}
									onChange={(event) => {
										setEmail(event.target.value);
										setError("");
									}}
								/>
							</div>
						</>
					) : (
						<div className="space-y-2">
							<Label htmlFor="account-email-code">
								{t("confirmationCode", "Confirmation code")}
							</Label>
							<Input
								id="account-email-code"
								name="code"
								autoComplete="one-time-code"
								inputMode="numeric"
								required
								disabled={Boolean(pending)}
								value={code}
								onChange={(event) => {
									setCode(event.target.value);
									setError("");
								}}
							/>
						</div>
					)}
					{step === "verification" && (
						<div className="flex flex-wrap gap-2">
							<Button
								type="button"
								variant="ghost"
								size="sm"
								disabled={Boolean(pending) || resendAfter > 0}
								onClick={resend}
							>
								{pending === "resend" && (
									<Loader2 className="size-4 animate-spin" />
								)}
								{resendAfter > 0
									? t("accountResendIn", "Resend in {{seconds}}s", {
											seconds: resendAfter,
										})
									: t("resend", "Resend code")}
							</Button>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								disabled={Boolean(pending)}
								onClick={() => {
									setStep("email");
									setCode("");
									setError("");
									setNotice("");
								}}
							>
								{t("accountUseDifferentEmail", "Use a different address")}
							</Button>
						</div>
					)}
					<div className="flex flex-col-reverse gap-2 pt-2 sm:flex-row sm:justify-end">
						<Button
							type="button"
							variant="outline"
							onClick={close}
							disabled={Boolean(pending)}
						>
							{step === "verification"
								? t("accountVerifyLater", "Verify later")
								: t("cancel", "Cancel")}
						</Button>
						<Button type="submit" disabled={Boolean(pending) || !info.data}>
							{pending === "submit" && (
								<Loader2 className="size-4 animate-spin" />
							)}
							{step === "email"
								? t("continue", "Continue")
								: t("verifyChange", "Verify and change")}
						</Button>
					</div>
				</form>
			</DialogContent>
		</Dialog>
	);
}
