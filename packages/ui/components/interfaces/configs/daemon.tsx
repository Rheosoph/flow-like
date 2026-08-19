"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

type DaemonRestartPolicy = "never" | "on_failure" | "always";

type DaemonSink = {
	sink_type?: "daemon";
	restart_policy?: DaemonRestartPolicy;
	min_restart_delay_ms?: number;
	max_restart_delay_ms?: number;
	board_poll_interval_ms?: number;
	log_flush_interval_ms?: number;
	log_batch_size?: number;
	healthy_reset_ms?: number;
	payload?: unknown;
};

const DEFAULTS: Required<Omit<DaemonSink, "payload">> = {
	sink_type: "daemon",
	restart_policy: "on_failure",
	min_restart_delay_ms: 1000,
	max_restart_delay_ms: 30000,
	board_poll_interval_ms: 3000,
	log_flush_interval_ms: 5000,
	log_batch_size: 500,
	healthy_reset_ms: 60000,
};

function numericValue(value: unknown, fallback: number): number {
	const parsed = Number(value);
	return Number.isFinite(parsed) ? parsed : fallback;
}

export function DaemonConfig({
	config,
	onConfigUpdate,
	isEditing,
	section,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const current = {
		...DEFAULTS,
		...(config as DaemonSink),
		sink_type: "daemon",
	};

	const setValue = (key: keyof DaemonSink, value: unknown) => {
		onConfigUpdate?.({
			...(config as DaemonSink),
			sink_type: "daemon",
			[key]: value,
		} as any);
	};

	const numberInput = (
		key: keyof DaemonSink,
		label: string,
		fallback: number,
		min: number,
		step = 100,
	) => (
		<div className="space-y-2">
			<Label htmlFor={`daemon_${key}`}>{label}</Label>
			<Input
				id={`daemon_${key}`}
				type="number"
				min={min}
				step={step}
				disabled={!isEditing}
				value={numericValue(current[key], fallback)}
				onChange={(event) => setValue(key, Number(event.target.value))}
			/>
		</div>
	);

	// The events surface renders one section at a time; anywhere else (and for
	// any section this component doesn't know) it renders whole.
	const shows = (id: string) => !section || section === id;

	return (
		<div className="w-full space-y-4">
			{shows("supervision") && (
				<div className="space-y-2">
					<Label htmlFor="daemon_restart_policy">
						{t("restartPolicy", "Restart Policy")}
					</Label>
					<Select
						value={current.restart_policy}
						onValueChange={(value) =>
							setValue("restart_policy", value as DaemonRestartPolicy)
						}
						disabled={!isEditing}
					>
						<SelectTrigger id="daemon_restart_policy" className="w-full">
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="on_failure">
								{t("onFailure", "On Failure")}
							</SelectItem>
							<SelectItem value="always">{t("always", "Always")}</SelectItem>
							<SelectItem value="never">{t("never", "Never")}</SelectItem>
						</SelectContent>
					</Select>
				</div>
			)}

			{shows("supervision") && (
				<div className="grid gap-4 md:grid-cols-2">
					{numberInput(
						"min_restart_delay_ms",
						t("minRestartDelayMs", "Min Restart Delay (ms)"),
						DEFAULTS.min_restart_delay_ms,
						100,
					)}
					{numberInput(
						"max_restart_delay_ms",
						t("maxRestartDelayMs", "Max Restart Delay (ms)"),
						DEFAULTS.max_restart_delay_ms,
						100,
					)}
					{numberInput(
						"healthy_reset_ms",
						t("healthyResetWindowMs", "Healthy Reset Window (ms)"),
						DEFAULTS.healthy_reset_ms,
						1000,
					)}
				</div>
			)}

			{shows("logging") && (
				<div className="grid gap-4 md:grid-cols-2">
					{numberInput(
						"board_poll_interval_ms",
						t("boardPollIntervalMs", "Board Poll Interval (ms)"),
						DEFAULTS.board_poll_interval_ms,
						500,
					)}
					{numberInput(
						"log_flush_interval_ms",
						t("logFlushIntervalMs", "Log Flush Interval (ms)"),
						DEFAULTS.log_flush_interval_ms,
						500,
					)}
					{numberInput(
						"log_batch_size",
						t("logBatchSize", "Log Batch Size"),
						DEFAULTS.log_batch_size,
						1,
						1,
					)}
				</div>
			)}
		</div>
	);
}
