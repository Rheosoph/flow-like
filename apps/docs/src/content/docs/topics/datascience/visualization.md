---
title: Data Visualization
description: Build clear, responsive charts and dashboards with A2UI
sidebar:
  order: 5
---

Flow-Like pages can present analytical results with NivoChart, PlotlyChart, Table, and supporting text components. Choose the visual from the question and data shape, then keep units, scope, and freshness visible.

## Chart libraries

| Component | Best fit |
|-----------|----------|
| **NivoChart** | Common dashboard charts with a consistent A2UI configuration |
| **PlotlyChart** | Scientific, statistical, and more specialized interactive plots |
| **Table** | Precise values, record review, and drill-down |

NivoChart supports chart types including bar, line, pie, radar, heatmap, scatter, funnel, treemap, sunburst, calendar, bump, area bump, circle packing, network, Sankey, stream, swarm plot, Voronoi, waffle, Marimekko, parallel coordinates, radial bar, box plot, bullet, and chord.

PlotlyChart exposes series, axis, data, layout, and configuration properties for Plotly-compatible views.

## Choose the visual from the question

| Question | Good starting point | Watch for |
|----------|---------------------|-----------|
| How does a value change over time? | Line chart | Missing intervals, timezone, uneven sampling |
| How do categories compare? | Sorted bar chart | Too many categories, truncated labels |
| What contributes to a total? | Stacked bar | Parts that do not share a real whole |
| What is the distribution? | Histogram or box plot | Bin choice, outliers, sample size |
| How do two measures relate? | Scatter plot | Overplotting, hidden segments |
| Where is intensity concentrated? | Heatmap | Unlabeled color scale |
| How does volume flow? | Sankey or funnel | Implied causality, inconsistent stages |
| Which exact records need action? | Table | Unbounded rows, missing sort or filters |

Use pie or donut charts only for a small number of meaningful parts. Avoid three-dimensional effects that distort area or angle.

## Configure the component

### NivoChart

The core properties include:

- chart type;
- title;
- data;
- height;
- colors;
- animation;
- legend visibility and position;
- index field and value keys;
- margins and axes;
- chart-specific style properties;
- an advanced Nivo configuration override.

The required data shape varies by chart type. Validate the final chart input instead of assuming one table shape works for every chart.

### PlotlyChart

PlotlyChart supports:

- chart type and title;
- one or more named series;
- x- and y-axis configuration;
- data, layout, and config values;
- width, height, responsive behavior, and legend placement.

Use Plotly when its trace and layout model better represents the analysis. Keep the page-level visual language consistent even when two libraries are used.

## Push data from a workflow

The A2UI chart node family includes:

| Need | Node |
|------|------|
| Replace chart data | [Push Data to Chart](/nodes/ui/elements/charts/a2ui-push-csv-to-chart/) |
| Update chart layout | [Set Chart Layout](/nodes/ui/elements/charts/a2ui-set-chart-layout/) |
| Update visual style | [Set Chart Style](/nodes/ui/elements/charts/a2ui-set-chart-style/) |
| Apply Nivo configuration | [Set Nivo Chart Config](/nodes/ui/elements/charts/a2ui-set-nivo-config/) |
| Prepare chart data with an agent | [Chart Data Agent](/nodes/ui/elements/charts/agent/a2ui-chart-data-agent/) |

A typical dashboard workflow:

1. reads validated filter values;
2. queries and aggregates the source;
3. checks row count and freshness;
4. pushes structured results to the chart and detail table;
5. updates title, scope, and status text;
6. handles empty and error states explicitly.

Aggregate before sending data to the page. Thousands of raw event rows rarely make a more useful interactive chart than a purpose-built query result.

## Pair charts with evidence

A chart should be supported by:

- a title that states the question or measure;
- visible units and time range;
- the aggregation and important filters;
- a freshness timestamp;
- a small table or drill-down route for exact values;
- a note for known exclusions or incomplete data.

When a dashboard uses governed KPIs, link the metric definition or repeat the essential formula and grain near the view.

## Color and themes

- Use a small, stable palette across a dashboard.
- Reserve semantic colors for consistent meanings such as success, warning, and failure.
- Check lines, labels, gridlines, selections, and tooltips in light and dark mode.
- Do not encode status or category with color alone.
- Use a sequential scale for ordered magnitude and a diverging scale only around a meaningful midpoint.
- Keep sufficient contrast between adjacent series and the page background.

## Axes and formatting

- Start quantitative axes at zero when bar length represents magnitude, unless a clearly marked exception is necessary.
- Label units once and format values consistently.
- Use the app's reporting timezone and state it when dates can be ambiguous.
- Keep decimal precision appropriate to the decision.
- Sort categories deliberately.
- Avoid dual axes when separate aligned charts communicate the relationship more honestly.

## Interaction

Use interactions to answer a next question:

- filter or highlight a related view;
- navigate to a detail page;
- reveal a tooltip with exact values;
- select records for a bounded workflow action.

Preserve selected filters in route or query state when the view should be shareable. Keep destructive actions outside ordinary chart gestures.

## Accessible visualization

- Provide a textual summary of the main finding.
- Offer a table or equivalent data view for exact values.
- Use visible focus states and keyboard-accessible controls.
- Keep labels readable at the smallest supported layout.
- Avoid rapid or unnecessary animation.
- Announce loading, empty, error, and refreshed states.

## Dashboard checklist

- [ ] Every chart answers a specific question
- [ ] Data shape matches the selected chart type
- [ ] Units, filters, time range, and freshness are visible
- [ ] Query output is aggregated and bounded
- [ ] Exact values are available in a table or drill-down
- [ ] Color works in light and dark mode and is not the only signal
- [ ] Loading, empty, stale, and error states are distinct
- [ ] Interactions lead to a useful next step
- [ ] The main finding is available as text

## Troubleshooting

| Symptom | Check |
|---------|-------|
| Chart does not render | Component ID, chart type, required data shape |
| Labels or series are missing | Index field, keys, series names, null values |
| Scale is misleading | Units, aggregation, axis bounds, category order |
| Theme looks wrong | Explicit colors, contrast, tooltip and grid styles |
| Page is slow | Row count, number of series, aggregation, animation |
| Chart and table disagree | Query versions, filters, refresh timing |

## Next steps

- [Building internal tools](/topics/internal-tools/overview/)
- [Business intelligence](/topics/business-intelligence/overview/)
- [DataFusion and SQL](/topics/datascience/datafusion/)
- [AI-powered analysis](/topics/datascience/ai-analysis/)
