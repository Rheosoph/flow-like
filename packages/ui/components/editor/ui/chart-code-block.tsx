"use client";

import { useTranslation } from "@flow-like/locales";
import { type ComponentType, useEffect, useMemo, useState } from "react";
import { cn } from "../../../lib/utils";
import { type ChartInput, parseChartData } from "./chart-data-parser";

type ChartPreviewComponent = ComponentType<{
	input: ChartInput;
	height?: number;
}>;

const CHART_COMPONENTS = {
	nivo: () => import("./chart-nivo-preview"),
	plotly: () => import("./chart-plotly-preview"),
} as const;

const TOGGLE_CLASS =
	"absolute top-2 left-2 z-10 rounded-md border border-border bg-background/95 px-2 py-1 text-xs text-muted-foreground hover:text-foreground";

interface ChartCodeBlockProps {
	/** Raw content from code block */
	content: string;
	/** Language identifier (nivo or plotly) */
	language: "nivo" | "plotly";
	/** Optional CSS class name */
	className?: string;
}

function ChartLoadingFallback() {
	const { t } = useTranslation("common");
	return (
		<div className="flex items-center justify-center h-75 bg-muted/20 rounded-md animate-pulse">
			<span className="text-muted-foreground text-sm">{t('loadingChart', 'Loading chart...')}</span>
		</div>
	);
}

function ChartErrorFallback({ error }: { error: string }) {
	return (
		<div className="flex items-center justify-center h-50 bg-destructive/10 rounded-md p-4">
			<span className="text-destructive text-sm">{error}</span>
		</div>
	);
}

function ChartModuleFallback({
	error,
	onRetry,
}: {
	error: string;
	onRetry: () => void;
}) {
	const { t } = useTranslation("common");
	return (
		<div className="flex h-50 flex-col items-center justify-center gap-3 rounded-md bg-destructive/10 p-4">
			<span className="text-center text-sm text-destructive">{error}</span>
			<button
				type="button"
				onClick={onRetry}
				className="rounded bg-background/80 px-3 py-1 text-xs text-foreground"
			>
				{t('retryChartLoad', 'Retry chart load')}
			</button>
		</div>
	);
}

/**
 * ChartCodeBlock renders ```nivo``` or ```plotly``` code blocks as interactive charts.
 *
 * Supports two modes:
 * 1. **CSV Mode**: Simple CSV data with optional config header
 *    ```nivo
 *    type: bar
 *    ---
 *    label,value
 *    Jan,20
 *    Feb,14
 *    Mar,25
 *    ```
 *
 * 2. **JSON Mode**: Full Plotly/Nivo JSON configuration
 *    ```plotly
 *    {
 *      "data": [{"x": [1,2,3], "y": [4,5,6], "type": "scatter"}],
 *      "layout": {"title": "My Chart"}
 *    }
 *    ```
 */
export function ChartCodeBlock({
	content,
	language,
	className,
}: ChartCodeBlockProps) {
	const { t } = useTranslation("common");
	const [showSource, setShowSource] = useState(false);
	const [ChartComponent, setChartComponent] =
		useState<ChartPreviewComponent | null>(null);
	const [moduleError, setModuleError] = useState<string | null>(null);
	const [isModuleLoading, setIsModuleLoading] = useState(false);
	const [retryKey, setRetryKey] = useState(0);

	const chartInput = useMemo<ChartInput | null>(() => {
		try {
			return parseChartData(content, language);
		} catch (e) {
			return null;
		}
	}, [content, language]);

	useEffect(() => {
		let cancelled = false;

		setIsModuleLoading(true);
		setModuleError(null);
		setChartComponent(null);

		CHART_COMPONENTS[language]()
			.then((mod) => {
				if (cancelled) return;
				const component = (mod.default ?? null) as ChartPreviewComponent | null;
				if (!component) {
					setModuleError(
						t('chartPreviewModuleLoadedWithoutADefaultExport', 'Chart preview module loaded without a default export.'),
					);
					return;
				}
				setChartComponent(() => component);
			})
			.catch((error) => {
				if (cancelled) return;
				const message =
					error instanceof Error
						? error.message
						: t('failedToLoadChartPreview', 'Failed to load chart preview.');
				setModuleError(message);
			})
			.finally(() => {
				if (!cancelled) {
					setIsModuleLoading(false);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [language, retryKey]);

	if (!chartInput) {
		return <ChartErrorFallback error="Failed to parse chart data" />;
	}

	if (showSource) {
		return (
			<div className={cn("relative", className)}>
				<button
					type="button"
					onClick={() => setShowSource(false)}
					className={TOGGLE_CLASS}
				>
					{t('showChart', 'Show Chart')}
				</button>
				<pre className="overflow-x-auto p-4 pt-10 font-mono text-sm bg-muted/50 rounded-md">
					<code>{content}</code>
				</pre>
			</div>
		);
	}

	// Top left, and only on hover: Plotly parks its toolbar in the top right, and
	// a permanent control competes with the chart title.
	return (
		<div className={cn("group relative", className)}>
			<button
				type="button"
				onClick={() => setShowSource(true)}
				className={cn(
					TOGGLE_CLASS,
					"opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100",
				)}
			>
				{t('viewSource', 'View Source')}
			</button>
			{isModuleLoading ? <ChartLoadingFallback /> : null}
			{!isModuleLoading && moduleError ? (
				<ChartModuleFallback
					error={moduleError}
					onRetry={() => setRetryKey((value) => value + 1)}
				/>
			) : null}
			{!isModuleLoading && !moduleError && ChartComponent ? (
				<ChartComponent input={chartInput} />
			) : null}
		</div>
	);
}

export default ChartCodeBlock;
