---
title: Charts
section: Styling
slug: charts
---

# Charts

MotoView ships a **complete set of chart components** — 66 of them — rendered as **server-side SVG**. There is **no charting JavaScript**: the canister computes the geometry in Motoko and returns `<svg>`, so charts are SEO-readable, work without scripts, and theme automatically. Tooltips are native SVG `<title>` (hover any shape); a CSS `:hover` highlights it. Everything uses the Fluent design tokens, so charts follow the active theme (try the picker on the live **[/charts](/charts)** gallery).

## Data is just strings

You don't pass typed arrays — you pass **CSV-style strings**, which means a chart works with a literal *or* a dynamic `@(expr)`:

```razor
<!-- literal -->
<ColumnChart values="120,98,145,160,132" labels="Jan,Feb,Mar,Apr,May" title="Signups" />

<!-- dynamic: any Text expression from @code -->
<LineChart series=@(model.chartData) labels=@(model.months) title="Revenue" />
```

The conventions:

| Prop | Format | Used by |
|---|---|---|
| `values` | `"42,30,55,20"` | single-series charts |
| `labels` | `"Q1,Q2,Q3,Q4"` | category axis |
| `series` | `"Sales:10,20,30;Costs:5,8,12"` | multi-series (`name:v,v;…`) |
| `points` | `"1,2;3,5;4,4"` | scatter (`x,y;…`) |
| `points` (bubble) | `"x,y,size;…"` | bubble |
| `ohlc` | `"Mon:100,110,95,108;…"` | candlestick / OHLC |
| `links` | `"Source>Target:value;…"` | sankey |
| `edges` | `"A>B;A>C;B>D"` | arc / dendrogram |

> Data containing `>` (sankey/arc/dendrogram) can't sit in a literal attribute — pass it via `@(expr)` from `@code`.

## Options

Any chart accepts: `title`, `width`, `height` (viewBox px), `unit` (suffix on values, e.g. `"%"`/`"$"`), and the booleans `hideAxes` / `hideGrid` / `hideLegend`. Numeric charts also take `yMin` / `yMax`.

```razor
<BarChart values="42,30,55" labels="Q1,Q2,Q3" title="Revenue" unit="$" width=480 height=300 hideGrid />
```

## The catalog

**Comparison** — `BarChart`, `ColumnChart`, `GroupedColumnChart`, `StackedColumnChart`, `LollipopChart`, `BulletChart`, `DotPlot`, `DumbbellChart`, `RangePlot`, `SlopeChart`, `DivergingBarChart`, `WaterfallChart`, `PictogramChart`, `ParallelCoordinates`, `MatrixChart`, `SmallMultiples`, `TableChart`.

**Trend & temporal** — `LineChart`, `SplineChart`, `StepLineChart`, `AreaChart`, `StackedAreaChart`, `Sparkline`, `CandlestickChart`, `OHLCChart`, `GanttChart`, `StreamGraph`, `BumpChart`, `BumpAreaChart`, `BarcodeChart`, `HorizonChart`.

**Part-to-whole & hierarchical** — `PieChart`, `DonutChart`, `SemiDonutChart`, `GaugeChart`, `RadialBarChart`, `NightingaleChart`, `WaffleChart`, `TreemapChart`, `CircularTreemap`, `FunnelChart`, `PyramidChart`, `MarimekkoChart`, `PopulationPyramid`, `SunburstChart`.

**Distribution** — `Histogram`, `RadialHistogram`, `BoxPlot`, `ViolinPlot`, `StripPlot`, `BeeswarmChart`, `DensityPlot`, `RidgelinePlot`, `WordCloud`.

**Correlation** — `ScatterChart`, `BubbleChart`, `ConnectedScatterChart`, `Heatmap`, `HexbinChart`, `QuadrantChart`, `RadarChart`.

**Flow & relationship** — `SankeyDiagram`, `ChordDiagram`, `ArcDiagram`, `Dendrogram`, `VennDiagram`.

## What isn't included (and why)

A few chart types need external data or heavy layout algorithms that don't fit a pure server-side SVG renderer, so they're deliberately left out rather than faked:

- **Geographic / choropleth / tile maps** — need TopoJSON geometry + map projections.
- **Force-directed network diagrams** — need an iterative physics layout.
- **Contour plots** — need grid interpolation over a continuous field.
- **Flowcharts** — need manual node placement / routing.

## How it works

Each component compiles to one call into `mo:motoview/Charts` (the `runtime/src/Charts.mo` module) — e.g. `<BarChart …>` becomes `b.raw(Charts.bar(values, labels, opts))`. The module is pure functions returning SVG `Text`, built on shared helpers (scales, nice ticks, axes, an arc/polar toolkit, a categorical palette, a legend builder). To add your own chart, add a function there and a one-line `gen_builtin` arm — see [Components](components.md) and [Styling & Themes](styling-and-themes.md).
