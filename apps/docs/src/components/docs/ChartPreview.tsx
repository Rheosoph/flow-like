import { lazy, Suspense, useMemo } from "react";
import { parseChartData } from "@flow-like/flow-like-ui/components/editor/ui/chart-data-parser";

const NivoPreview = lazy(
	() =>
		import("@flow-like/flow-like-ui/components/editor/ui/chart-nivo-preview"),
);
const PlotlyPreview = lazy(
	() =>
		import("@flow-like/flow-like-ui/components/editor/ui/chart-plotly-preview"),
);

export default function ChartPreview({
	source,
	language,
}: { source: string; language: "nivo" | "plotly" }) {
	const input = useMemo(
		() => parseChartData(source, language),
		[source, language],
	);
	const Preview = language === "nivo" ? NivoPreview : PlotlyPreview;
	return (
		<div
			className="not-content docs-chart-preview"
			aria-label={`${language === "nivo" ? "Nivo" : "Plotly"} example output`}
			style={{ minHeight: 350 }}
		>
			<Suspense fallback={<p>Loading chart preview…</p>}>
				<Preview input={input} height={350} />
			</Suspense>
		</div>
	);
}
