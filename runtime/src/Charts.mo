/// MotoView charts — pure, server-rendered SVG.
///
/// Every public chart function (defined BELOW this foundation, one per file
/// section) takes data as CSV-style STRINGS plus an options record `O`, and
/// RETURNS a complete `<svg class="mv-chart" viewBox=...>…</svg>` as `Text`.
/// No state, no async. Data string conventions:
///   * values:  "42,30,55,20"
///   * labels:  "Q1,Q2,Q3,Q4"
///   * series:  "Sales:10,20,30;Costs:5,8,12"   (name:vals; ; shared x = labels)
///   * xy:      "1,2;3,5;4,4"                     (one "x,y" pair per ; segment)
///
/// Theming: the SVG references CSS classes (.mv-chart-*) so it adapts to
/// light/dark/Teams/HC themes; series colors are ALSO baked as hex (palette/0)
/// so charts are correct before CSS loads.
import Text "mo:base/Text";
import Float "mo:base/Float";
import Int "mo:base/Int";
import Nat "mo:base/Nat";
import Buffer "mo:base/Buffer";
import Array "mo:base/Array";
import Iter "mo:base/Iter";
import Char "mo:base/Char";
import Nat32 "mo:base/Nat32";

module {

  // ---- data types ----------------------------------------------------------
  public type Series = (Text, [Float]); // (name, values)
  public type Point = (Float, Float);   // (x, y)

  // ---- options -------------------------------------------------------------
  /// Chart options. Emit overrides via Motoko record-update: `{ def with ... }`.
  public type O = {
    width : Nat;        // viewBox width  (px units)
    height : Nat;       // viewBox height
    title : Text;       // "" = none
    colors : [Text];    // [] = use built-in palette; else hex/CSS-color overrides
    showAxes : Bool;
    showGrid : Bool;
    showLegend : Bool;
    yMin : ?Float;      // null = auto (data min, clamped to 0 for bars)
    yMax : ?Float;      // null = auto (nice-rounded data max)
    unit : Text;        // suffix on y tick / tooltip values, e.g. "%" or "$"
  };

  /// Default options. Arms emit `Charts.bar(v, l, { def with title = "…" })`.
  public let def : O = {
    width = 640;
    height = 360;
    title = "";
    colors = [];
    showAxes = true;
    showGrid = true;
    showLegend = true;
    yMin = null;
    yMax = null;
    unit = "";
  };

  // Inner plot padding (room for axis labels / ticks / title).
  let padL : Float = 56.0;
  let padR : Float = 16.0;
  let padT : Float = 28.0;
  let padB : Float = 36.0;

  // ---- parsers -------------------------------------------------------------
  // Trim ASCII spaces/tabs from both ends (base Text has no trim in 0.29 stable
  // API surface we rely on, so roll a tiny tolerant one).
  func trim(t : Text) : Text {
    Text.trimStart(Text.trimEnd(t, #char ' '), #char ' ');
  };

  func toFloat(t : Text) : ?Float {
    let s = trim(t);
    if (s == "") { return null };
    var neg = false;
    var i : Nat = 0;
    let cs = Text.toArray(s);
    let n = cs.size();
    if (n == 0) { return null };
    if (cs[0] == '-') { neg := true; i := 1 } else if (cs[0] == '+') { i := 1 };
    var intPart : Float = 0.0;
    var seenDigit = false;
    while (i < n and cs[i] != '.') {
      let c = cs[i];
      if (c >= '0' and c <= '9') {
        intPart := intPart * 10.0 + Float.fromInt(Int.abs(charDigit(c)));
        seenDigit := true;
      } else { return null };
      i += 1;
    };
    var frac : Float = 0.0;
    var scale : Float = 1.0;
    if (i < n and cs[i] == '.') {
      i += 1;
      while (i < n) {
        let c = cs[i];
        if (c >= '0' and c <= '9') {
          scale := scale / 10.0;
          frac := frac + Float.fromInt(Int.abs(charDigit(c))) * scale;
          seenDigit := true;
        } else { return null };
        i += 1;
      };
    };
    if (not seenDigit) { return null };
    let v = intPart + frac;
    ?(if (neg) { -v } else { v });
  };

  func charDigit(c : Char) : Int { let n : Int = Nat32.toNat(Char.toNat32(c)); n - 48 };

  /// "42, 30 ,55" -> [42.0, 30.0, 55.0]. Empty/garbage tokens are skipped.
  public func parseFloats(csv : Text) : [Float] {
    let out = Buffer.Buffer<Float>(8);
    for (tok in Text.split(csv, #char ',')) {
      switch (toFloat(tok)) { case (?f) { out.add(f) }; case null {} };
    };
    Buffer.toArray(out);
  };

  /// "Q1, Q2 ,Q3" -> ["Q1","Q2","Q3"]. Preserves order; trims spaces.
  public func parseLabels(csv : Text) : [Text] {
    let out = Buffer.Buffer<Text>(8);
    for (tok in Text.split(csv, #char ',')) { out.add(trim(tok)) };
    Buffer.toArray(out);
  };

  /// "Sales:10,20;Costs:5,8" -> [("Sales",[10,20]),("Costs",[5,8])].
  /// A segment without ':' is treated as an unnamed series ("").
  public func parseSeries(spec : Text) : [Series] {
    let out = Buffer.Buffer<Series>(4);
    for (seg in Text.split(spec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        switch (Text.split(s, #char ':').next()) {
          case (_) {};
        };
        // split once on the first ':'
        let parts = Iter.toArray(Text.split(s, #char ':'));
        if (parts.size() >= 2) {
          let name = trim(parts[0]);
          // rejoin the remainder in case values contained no ':' (they won't,
          // but be safe): values are everything after the first ':'.
          var rest = parts[1];
          var k = 2;
          while (k < parts.size()) { rest := rest # ":" # parts[k]; k += 1 };
          out.add((name, parseFloats(rest)));
        } else {
          out.add(("", parseFloats(s)));
        };
      };
    };
    Buffer.toArray(out);
  };

  /// "1,2;3,5;4,4" -> [(1,2),(3,5),(4,4)]. Each ';' segment is an "x,y" pair.
  public func parseXY(spec : Text) : [Point] {
    let out = Buffer.Buffer<Point>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        let pair = Iter.toArray(Text.split(s, #char ','));
        if (pair.size() >= 2) {
          switch (toFloat(pair[0]), toFloat(pair[1])) {
            case (?x, ?y) { out.add((x, y)) };
            case _ {};
          };
        };
      };
    };
    Buffer.toArray(out);
  };

  // ---- number formatting ---------------------------------------------------
  /// Compact, human-friendly number: drops trailing ".0", keeps up to 2 decimals.
  /// (Float.toText emits long forms like 30.000000000; we trim to 2 places.)
  public func fmtNum(x : Float) : Text {
    if (x != x) { return "0" }; // NaN guard
    let neg = x < 0.0;
    let a = Float.abs(x);
    let whole = Float.floor(a);
    let fracF = a - whole;
    // round fraction to 2 dp
    let scaled = Float.floor(fracF * 100.0 + 0.5);
    var wi : Int = Float.toInt(whole);
    var f2 : Int = Float.toInt(scaled);
    if (f2 >= 100) { wi += 1; f2 := 0 };
    var s = Int.toText(wi);
    if (f2 > 0) {
      let fs = if (f2 < 10) { "0" # Int.toText(f2) } else { Int.toText(f2) };
      // strip a trailing zero (e.g. "50" -> "5")
      let fs2 = if (Text.endsWith(fs, #char '0')) {
        let arr = Text.toArray(fs);
        Text.fromChar(arr[0]);
      } else { fs };
      s := s # "." # fs2;
    };
    if (neg and (wi != 0 or f2 != 0)) { "-" # s } else { s };
  };

  // ---- scales & ticks ------------------------------------------------------
  /// Build a linear scale: maps a domain value into a pixel coordinate.
  public type Scale = Float -> Float;

  public func linScale(d0 : Float, d1 : Float, r0 : Float, r1 : Float) : Scale {
    let dspan = if (d1 - d0 == 0.0) { 1.0 } else { d1 - d0 };
    func(v : Float) : Float { r0 + (v - d0) / dspan * (r1 - r0) };
  };

  func maxOf(xs : [Float]) : Float {
    var m : Float = 0.0; var first = true;
    for (x in xs.vals()) { if (first or x > m) { m := x; first := false } };
    m;
  };
  func minOf(xs : [Float]) : Float {
    var m : Float = 0.0; var first = true;
    for (x in xs.vals()) { if (first or x < m) { m := x; first := false } };
    m;
  };
  public func arrMax(xs : [Float]) : Float { maxOf(xs) };
  public func arrMin(xs : [Float]) : Float { minOf(xs) };

  /// A "nice" upper bound >= raw (1/2/5 * 10^k), for clean axis maxima.
  public func niceCeil(raw : Float) : Float {
    if (raw <= 0.0) { return 1.0 };
    let exp = Float.floor(Float.log(raw) / Float.log(10.0));
    let pow10 = Float.pow(10.0, exp);
    let f = raw / pow10;
    let nf = if (f <= 1.0) { 1.0 } else if (f <= 2.0) { 2.0 }
             else if (f <= 5.0) { 5.0 } else { 10.0 };
    nf * pow10;
  };

  /// Evenly spaced tick values from lo..hi inclusive (count+1 entries).
  public func ticks(lo : Float, hi : Float, count : Nat) : [Float] {
    let n = if (count == 0) { 1 } else { count };
    let step = (hi - lo) / Float.fromInt(n);
    let out = Buffer.Buffer<Float>(n + 1);
    var i : Nat = 0;
    while (i <= n) { out.add(lo + step * Float.fromInt(i)); i += 1 };
    Buffer.toArray(out);
  };

  // ---- palette -------------------------------------------------------------
  // Baked categorical hex (mirrors the CSS --mv-cat-1..10), so SVG is colored
  // even before motoview.css applies. Index wraps.
  let cat : [Text] = [
    "#0f6cbd", "#107c10", "#d13438", "#ca5010", "#8764b8",
    "#038387", "#c239b3", "#986f0b", "#005b70", "#a4262c",
  ];

  /// Resolve series color i: an `opts.colors` override wins, else the baked
  /// palette (wrapping). Always returns a usable CSS/SVG color.
  public func palette(opts : O, i : Nat) : Text {
    if (opts.colors.size() > 0) { opts.colors[i % opts.colors.size()] }
    else { cat[i % cat.size()] };
  };

  // ---- text / svg escaping -------------------------------------------------
  /// Escape text for safe use inside SVG element content or attribute values.
  public func esc(t : Text) : Text {
    var s = t;
    s := Text.replace(s, #char '&', "&amp;");
    s := Text.replace(s, #char '<', "&lt;");
    s := Text.replace(s, #char '>', "&gt;");
    s := Text.replace(s, #char '\"', "&quot;");
    s := Text.replace(s, #char '\'', "&#39;");
    s;
  };

  // ---- svg scaffold --------------------------------------------------------
  /// Open the root <svg class="mv-chart ..."> with a viewBox; pass an extra
  /// modifier class (e.g. "mv-chart-bar") and the title (rendered as <title>
  /// for a11y + an inner <text> when non-empty).
  public func svgOpen(opts : O, modClass : Text) : Text {
    let w = Nat.toText(opts.width);
    let h = Nat.toText(opts.height);
    var s = "<svg class=\"mv-chart " # esc(modClass)
      # "\" viewBox=\"0 0 " # w # " " # h
      # "\" role=\"img\" preserveAspectRatio=\"xMidYMid meet\""
      # " xmlns=\"http://www.w3.org/2000/svg\">";
    if (opts.title != "") {
      s #= "<title>" # esc(opts.title) # "</title>";
      s #= "<text class=\"mv-chart-title\" x=\"" # fmtNum(Float.fromInt(Int.abs(opts.width)) / 2.0)
        # "\" y=\"18\" text-anchor=\"middle\">" # esc(opts.title) # "</text>";
    };
    s;
  };
  public func svgClose() : Text { "</svg>" };

  // Inner plot rectangle (after padding), as floats.
  public func plotLeft() : Float { padL };
  public func plotRight(opts : O) : Float { Float.fromInt(Int.abs(opts.width)) - padR };
  public func plotTop() : Float { padT };
  public func plotBottom(opts : O) : Float { Float.fromInt(Int.abs(opts.height)) - padB };

  // ---- axes & gridlines ----------------------------------------------------
  /// Horizontal gridlines + left (y) axis ticks/labels for a numeric y scale.
  public func axisLeft(opts : O, ySc : Scale, yticks : [Float]) : Text {
    let x0 = plotLeft();
    let x1 = plotRight(opts);
    let b = Buffer.Buffer<Text>(yticks.size() * 2 + 1);
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(x0) # "\" y1=\"" # fmtNum(plotTop())
        # "\" x2=\"" # fmtNum(x0) # "\" y2=\"" # fmtNum(plotBottom(opts)) # "\"/>");
    };
    for (tv in yticks.vals()) {
      let y = ySc(tv);
      if (opts.showGrid) {
        b.add("<line class=\"mv-chart-grid\" x1=\"" # fmtNum(x0) # "\" y1=\"" # fmtNum(y)
          # "\" x2=\"" # fmtNum(x1) # "\" y2=\"" # fmtNum(y) # "\"/>");
      };
      b.add("<text class=\"mv-chart-tick\" x=\"" # fmtNum(x0 - 6.0) # "\" y=\"" # fmtNum(y + 4.0)
        # "\" text-anchor=\"end\">" # esc(fmtNum(tv) # opts.unit) # "</text>");
    };
    Text.join("", b.vals());
  };

  /// Bottom (x) axis line + category labels positioned at given x centers.
  public func axisBottom(opts : O, labels : [Text], centers : [Float]) : Text {
    let yB = plotBottom(opts);
    let b = Buffer.Buffer<Text>(labels.size() + 1);
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(yB)
        # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(yB) # "\"/>");
    };
    var i : Nat = 0;
    while (i < labels.size() and i < centers.size()) {
      b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(centers[i])
        # "\" y=\"" # fmtNum(yB + 16.0) # "\" text-anchor=\"middle\">" # esc(labels[i]) # "</text>");
      i += 1;
    };
    Text.join("", b.vals());
  };

  // ---- path / shape builders ----------------------------------------------
  /// SVG points attribute body for a polyline/polygon from (x,y) pixel pairs.
  public func polyPoints(pts : [(Float, Float)]) : Text {
    let b = Buffer.Buffer<Text>(pts.size());
    for ((x, y) in pts.vals()) { b.add(fmtNum(x) # "," # fmtNum(y)) };
    Text.join(" ", b.vals());
  };

  /// A filled-area path "M x0,base L … L xN,base Z" under a line.
  public func areaPath(pts : [(Float, Float)], baseY : Float) : Text {
    if (pts.size() == 0) { return "" };
    let b = Buffer.Buffer<Text>(pts.size() + 3);
    let (x0, _) = pts[0];
    b.add("M " # fmtNum(x0) # "," # fmtNum(baseY));
    for ((x, y) in pts.vals()) { b.add("L " # fmtNum(x) # "," # fmtNum(y)) };
    let (xn, _) = pts[pts.size() - 1];
    b.add("L " # fmtNum(xn) # "," # fmtNum(baseY));
    b.add("Z");
    Text.join(" ", b.vals());
  };

  // Polar helpers for pie/donut: angle in radians, 0 = 12 o'clock, clockwise.
  let twoPi : Float = 6.283185307179586;
  public func polarX(cx : Float, r : Float, ang : Float) : Float { cx + r * sin_(ang) };
  public func polarY(cy : Float, r : Float, ang : Float) : Float { cy - r * cos_(ang) };

  /// Donut/pie slice path from startFrac..endFrac of the circle (0..1).
  /// innerR = 0 -> a pie wedge; innerR > 0 -> a donut segment.
  public func arcPath(cx : Float, cy : Float, outerR : Float, innerR : Float, startFrac : Float, endFrac : Float) : Text {
    let a0 = startFrac * twoPi;
    let a1 = endFrac * twoPi;
    let large = if (endFrac - startFrac > 0.5) { "1" } else { "0" };
    let ox0 = polarX(cx, outerR, a0); let oy0 = polarY(cy, outerR, a0);
    let ox1 = polarX(cx, outerR, a1); let oy1 = polarY(cy, outerR, a1);
    if (innerR <= 0.0) {
      "M " # fmtNum(cx) # "," # fmtNum(cy)
        # " L " # fmtNum(ox0) # "," # fmtNum(oy0)
        # " A " # fmtNum(outerR) # "," # fmtNum(outerR) # " 0 " # large # " 1 "
        # fmtNum(ox1) # "," # fmtNum(oy1) # " Z";
    } else {
      let ix1 = polarX(cx, innerR, a1); let iy1 = polarY(cy, innerR, a1);
      let ix0 = polarX(cx, innerR, a0); let iy0 = polarY(cy, innerR, a0);
      "M " # fmtNum(ox0) # "," # fmtNum(oy0)
        # " A " # fmtNum(outerR) # "," # fmtNum(outerR) # " 0 " # large # " 1 "
        # fmtNum(ox1) # "," # fmtNum(oy1)
        # " L " # fmtNum(ix1) # "," # fmtNum(iy1)
        # " A " # fmtNum(innerR) # "," # fmtNum(innerR) # " 0 " # large # " 0 "
        # fmtNum(ix0) # "," # fmtNum(iy0) # " Z";
    };
  };

  // Minimal sin/cos (Taylor, range-reduced) — base trig isn't guaranteed in the
  // pinned API surface, and we only need modest precision for SVG arcs.
  func sin_(x0 : Float) : Float {
    var x = x0;
    while (x > 3.141592653589793) { x -= twoPi };
    while (x < -3.141592653589793) { x += twoPi };
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0)));
  };
  func cos_(x : Float) : Float { sin_(x + 1.5707963267948966) };

  // ---- legend --------------------------------------------------------------
  /// A foreignObject-free SVG legend: colored swatches + names along the top.
  /// `entries` = [(name, color)]. Positioned just under the title row.
  public func legend(opts : O, entries : [(Text, Text)]) : Text {
    if (not opts.showLegend or entries.size() == 0) { return "" };
    let b = Buffer.Buffer<Text>(entries.size() + 1);
    b.add("<g class=\"mv-chart-legend\">");
    var x : Float = plotLeft();
    let y : Float = Float.fromInt(Int.abs(opts.height)) - 10.0;
    var i : Nat = 0;
    for ((name, color) in entries.vals()) {
      b.add("<rect class=\"mv-chart-swatch\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(y - 9.0)
        # "\" width=\"10\" height=\"10\" rx=\"2\" fill=\"" # esc(color) # "\"/>");
      b.add("<text class=\"mv-chart-legend-label\" x=\"" # fmtNum(x + 14.0) # "\" y=\"" # fmtNum(y)
        # "\">" # esc(name) # "</text>");
      // advance ~ swatch + label width estimate (7px/char)
      x += 24.0 + Float.fromInt(name.size()) * 7.0;
      i += 1;
    };
    b.add("</g>");
    Text.join("", b.vals());
  };

  // ---- module-local base bridges (kept at the bottom for readability) -------
  // These wrap the base modules used by the tiny char->code path above.




  // ===== BarChart =====
  // ---- BarChart: HORIZONTAL bars -------------------------------------------
  /// One horizontal bar per (label, value); the value axis runs along the
  /// bottom. `<BarChart values="42,30,55" labels="Q1,Q2,Q3" />`.
  public func bar(valuesCsv : Text, labelsCsv : Text, opts : O) : Text {
    let vals = parseFloats(valuesCsv);
    let labels = parseLabels(labelsCsv);
    let n = vals.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-bar") # svgClose() };

    let (lo, hi) = yDomain(opts, arrMin(vals), arrMax(vals), true);
    let xSc = linScale(lo, hi, plotLeft(), plotRight(opts));
    let top = plotTop();
    let bot = plotBottom(opts);
    let band = (bot - top) / Float.fromInt(n);
    let barH = band * 0.62;
    let baseX = xSc(if (lo < 0.0) { 0.0 } else { lo });

    let b = Buffer.Buffer<Text>(n * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-bar"));

    // value gridlines + bottom value axis
    let xticks = ticks(lo, hi, 4);
    for (tv in xticks.vals()) {
      let x = xSc(tv);
      if (opts.showGrid) {
        b.add("<line class=\"mv-chart-grid\" x1=\"" # fmtNum(x) # "\" y1=\"" # fmtNum(top)
          # "\" x2=\"" # fmtNum(x) # "\" y2=\"" # fmtNum(bot) # "\"/>");
      };
      b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(bot + 16.0)
        # "\" text-anchor=\"middle\">" # esc(fmtNum(tv) # opts.unit) # "</text>");
    };
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(baseX) # "\" y1=\"" # fmtNum(top)
        # "\" x2=\"" # fmtNum(baseX) # "\" y2=\"" # fmtNum(bot) # "\"/>");
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(bot)
        # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(bot) # "\"/>");
    };

    let color = palette(opts, 0);
    var i : Nat = 0;
    while (i < n) {
      let v = vals[i];
      let yc = top + band * Float.fromInt(i) + (band - barH) / 2.0;
      let xv = xSc(v);
      let x0 = minF(baseX, xv);
      let w = Float.abs(xv - baseX);
      let lbl = labelAt(labels, i);
      b.add("<g class=\"mv-chart-bar-g\">");
      b.add("<rect class=\"mv-chart-bar-rect\" x=\"" # fmtNum(x0) # "\" y=\"" # fmtNum(yc)
        # "\" width=\"" # fmtNum(w) # "\" height=\"" # fmtNum(barH)
        # "\" rx=\"2\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc(lbl # ": " # fmtNum(v) # opts.unit) # "</title></rect>");
      b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(plotLeft() - 6.0)
        # "\" y=\"" # fmtNum(yc + barH / 2.0 + 4.0) # "\" text-anchor=\"end\">" # esc(lbl) # "</text>");
      b.add("</g>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

// NOTE: also requires these shared helpers (added once for the whole axis
// family, alongside the foundation): parseXYZ, axisBottomNumeric, yDomain,
// xDomainNice, maxF, sqrt_. See the StackedColumnChart entry's css/example for
// where they live. (minF, labelAt already exist from the radial family.)


  // ===== ColumnChart =====
  // ---- ColumnChart: VERTICAL columns ---------------------------------------
  /// One vertical column per (label, value). `<ColumnChart values="42,30,55"
  /// labels="Q1,Q2,Q3" />`.
  public func column(valuesCsv : Text, labelsCsv : Text, opts : O) : Text {
    let vals = parseFloats(valuesCsv);
    let labels = parseLabels(labelsCsv);
    let n = vals.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-column") # svgClose() };

    let (lo, hi) = yDomain(opts, arrMin(vals), arrMax(vals), true);
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(n);
    let colW = band * 0.62;
    let baseY = ySc(if (lo < 0.0) { 0.0 } else { lo });

    let b = Buffer.Buffer<Text>(n * 2 + 8);
    b.add(svgOpen(opts, "mv-chart-column"));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 4)));
    let centers = Array.tabulate<Float>(n, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    let color = palette(opts, 0);
    var i : Nat = 0;
    while (i < n) {
      let v = vals[i];
      let yv = ySc(v);
      let y0 = minF(baseY, yv);
      let h = Float.abs(yv - baseY);
      let xc = left + band * Float.fromInt(i) + (band - colW) / 2.0;
      let lbl = labelAt(labels, i);
      b.add("<rect class=\"mv-chart-col-rect\" x=\"" # fmtNum(xc) # "\" y=\"" # fmtNum(y0)
        # "\" width=\"" # fmtNum(colW) # "\" height=\"" # fmtNum(h)
        # "\" rx=\"2\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc(lbl # ": " # fmtNum(v) # opts.unit) # "</title></rect>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };


  // ===== GroupedColumnChart =====
  // ---- GroupedColumnChart: multi-series side-by-side columns ---------------
  /// `<GroupedColumnChart series="Sales:10,20,30;Costs:5,8,12"
  ///                      labels="Q1,Q2,Q3" />`.
  public func groupedColumn(seriesSpec : Text, labelsCsv : Text, opts : O) : Text {
    let series = parseSeries(seriesSpec);
    let labels = parseLabels(labelsCsv);
    let ns = series.size();
    if (ns == 0) { return svgOpen(opts, "mv-chart-grouped") # svgClose() };

    var cats : Nat = 0;
    for ((_, vs) in series.vals()) { if (vs.size() > cats) { cats := vs.size() } };
    if (cats == 0) { return svgOpen(opts, "mv-chart-grouped") # svgClose() };

    // overall data range across all series
    let allVals = Buffer.Buffer<Float>(cats * ns);
    for ((_, vs) in series.vals()) { for (v in vs.vals()) { allVals.add(v) } };
    let arr = Buffer.toArray(allVals);
    let (lo, hi) = yDomain(opts, arrMin(arr), arrMax(arr), true);
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(cats);
    let groupW = band * 0.72;
    let colW = groupW / Float.fromInt(ns);
    let baseY = ySc(if (lo < 0.0) { 0.0 } else { lo });

    let b = Buffer.Buffer<Text>(cats * ns + 8);
    b.add(svgOpen(opts, "mv-chart-grouped"));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 4)));
    let centers = Array.tabulate<Float>(cats, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    var ci : Nat = 0;
    while (ci < cats) {
      let gx = left + band * Float.fromInt(ci) + (band - groupW) / 2.0;
      var si : Nat = 0;
      while (si < ns) {
        let (name, vs) = series[si];
        if (ci < vs.size()) {
          let v = vs[ci];
          let yv = ySc(v);
          let y0 = minF(baseY, yv);
          let h = Float.abs(yv - baseY);
          let x = gx + colW * Float.fromInt(si);
          let color = palette(opts, si);
          let lbl = labelAt(labels, ci);
          b.add("<rect class=\"mv-chart-col-rect\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(y0)
            # "\" width=\"" # fmtNum(colW * 0.9) # "\" height=\"" # fmtNum(h)
            # "\" rx=\"2\" fill=\"" # esc(color) # "\">"
            # "<title>" # esc(name # " " # lbl # ": " # fmtNum(v) # opts.unit) # "</title></rect>");
        };
        si += 1;
      };
      ci += 1;
    };
    let entries = Array.tabulate<(Text, Text)>(ns, func(i) { (series[i].0, palette(opts, i)) });
    b.add(legend(opts, entries));
    b.add(svgClose());
    Text.join("", b.vals());
  };


  // ===== StackedColumnChart =====
  // ---- shared axis-family helpers (added ONCE for all six axis charts) ------
  /// "1,2,8;3,5,20" -> [(1,2,8),(3,5,20)]. Each ';' segment is "x,y,size";
  /// a missing third value defaults to 1.0. Used by BubbleChart.
  public func parseXYZ(spec : Text) : [(Float, Float, Float)] {
    let out = Buffer.Buffer<(Float, Float, Float)>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        let trip = Iter.toArray(Text.split(s, #char ','));
        if (trip.size() >= 2) {
          switch (toFloat(trip[0]), toFloat(trip[1])) {
            case (?x, ?y) {
              let z = if (trip.size() >= 3) {
                switch (toFloat(trip[2])) { case (?z0) { z0 }; case null { 1.0 } };
              } else { 1.0 };
              out.add((x, y, z));
            };
            case _ {};
          };
        };
      };
    };
    Buffer.toArray(out);
  };
  func yDomain(opts : O, dataMin : Float, dataMax : Float, baseAtZero : Bool) : (Float, Float) {
    let lo = switch (opts.yMin) { case (?v) { v };
      case null { if (baseAtZero) { minF(0.0, dataMin) } else { dataMin } } };
    let hi = switch (opts.yMax) { case (?v) { v };
      case null { niceCeil(maxF(dataMax, lo + 1.0)) } };
    (lo, if (hi <= lo) { lo + 1.0 } else { hi });
  };
  func xDomainNice(lo0 : Float, hi0 : Float) : (Float, Float) {
    if (hi0 <= lo0) {
      let pad = if (Float.abs(lo0) < 1.0) { 1.0 } else { Float.abs(lo0) * 0.1 };
      (lo0 - pad, hi0 + pad);
    } else { let pad = (hi0 - lo0) * 0.05; (lo0 - pad, hi0 + pad) };
  };
  func maxF(a : Float, b : Float) : Float { if (a > b) { a } else { b } };
  func sqrt_(x : Float) : Float {
    if (x <= 0.0) { return 0.0 };
    var g = x; var i : Nat = 0;
    while (i < 20) { g := (g + x / g) / 2.0; i += 1 }; g;
  };
  /// Vertical gridlines + bottom NUMERIC x axis (scatter/bubble).
  public func axisBottomNumeric(opts : O, xSc : Scale, xticks : [Float]) : Text {
    let yB = plotBottom(opts); let yT = plotTop();
    let b = Buffer.Buffer<Text>(xticks.size() * 2 + 1);
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(yB)
        # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(yB) # "\"/>");
    };
    for (tv in xticks.vals()) {
      let x = xSc(tv);
      if (opts.showGrid) {
        b.add("<line class=\"mv-chart-grid\" x1=\"" # fmtNum(x) # "\" y1=\"" # fmtNum(yT)
          # "\" x2=\"" # fmtNum(x) # "\" y2=\"" # fmtNum(yB) # "\"/>");
      };
      b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(yB + 16.0)
        # "\" text-anchor=\"middle\">" # esc(fmtNum(tv)) # "</text>");
    };
    Text.join("", b.vals());
  };

  // ---- StackedColumnChart: multi-series stacked per category ----------------
  /// `<StackedColumnChart series="Sales:10,20,30;Costs:5,8,12"
  ///                      labels="Q1,Q2,Q3" />`. Negative values are ignored
  /// (non-negative stacking model).
  public func stackedColumn(seriesSpec : Text, labelsCsv : Text, opts : O) : Text {
    let series = parseSeries(seriesSpec);
    let labels = parseLabels(labelsCsv);
    let ns = series.size();
    if (ns == 0) { return svgOpen(opts, "mv-chart-stacked") # svgClose() };

    var cats : Nat = 0;
    for ((_, vs) in series.vals()) { if (vs.size() > cats) { cats := vs.size() } };
    if (cats == 0) { return svgOpen(opts, "mv-chart-stacked") # svgClose() };

    // stacked totals per category determine the y range
    var dmax : Float = 0.0;
    var ci0 : Nat = 0;
    while (ci0 < cats) {
      var s0 : Float = 0.0;
      for ((_, vs) in series.vals()) {
        if (ci0 < vs.size() and vs[ci0] > 0.0) { s0 += vs[ci0] };
      };
      if (s0 > dmax) { dmax := s0 };
      ci0 += 1;
    };
    let (lo, hi) = yDomain(opts, 0.0, dmax, true);
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(cats);
    let colW = band * 0.62;

    let b = Buffer.Buffer<Text>(cats * ns + 8);
    b.add(svgOpen(opts, "mv-chart-stacked"));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 4)));
    let centers = Array.tabulate<Float>(cats, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    var ci : Nat = 0;
    while (ci < cats) {
      let xc = left + band * Float.fromInt(ci) + (band - colW) / 2.0;
      var acc : Float = 0.0; // running stacked total (value units)
      var si : Nat = 0;
      while (si < ns) {
        let (name, vs) = series[si];
        if (ci < vs.size() and vs[ci] > 0.0) {
          let v = vs[ci];
          let yTop = ySc(acc + v);
          let yBottom = ySc(acc);
          let h = Float.abs(yBottom - yTop);
          let color = palette(opts, si);
          let lbl = labelAt(labels, ci);
          b.add("<rect class=\"mv-chart-col-rect\" x=\"" # fmtNum(xc) # "\" y=\"" # fmtNum(yTop)
            # "\" width=\"" # fmtNum(colW) # "\" height=\"" # fmtNum(h)
            # "\" fill=\"" # esc(color) # "\">"
            # "<title>" # esc(name # " " # lbl # ": " # fmtNum(v) # opts.unit) # "</title></rect>");
          acc += v;
        };
        si += 1;
      };
      ci += 1;
    };
    let entries = Array.tabulate<(Text, Text)>(ns, func(i) { (series[i].0, palette(opts, i)) });
    b.add(legend(opts, entries));
    b.add(svgClose());
    Text.join("", b.vals());
  };


  // ===== ScatterChart =====
  // ---- ScatterChart: xy points ---------------------------------------------
  /// `<ScatterChart points="1,2;3,5;4,4" />`. Both axes are numeric.
  public func scatter(pointsSpec : Text, opts : O) : Text {
    let pts = parseXY(pointsSpec);
    let n = pts.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-scatter") # svgClose() };

    let xs = Array.map<Point, Float>(pts, func(p) { p.0 });
    let ys = Array.map<Point, Float>(pts, func(p) { p.1 });
    let (xlo, xhi) = xDomainNice(arrMin(xs), arrMax(xs));
    let (ylo, yhi) = yDomain(opts, arrMin(ys), arrMax(ys), false);
    let xSc = linScale(xlo, xhi, plotLeft(), plotRight(opts));
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());

    let b = Buffer.Buffer<Text>(n + 8);
    b.add(svgOpen(opts, "mv-chart-scatter"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 4)));
    b.add(axisBottomNumeric(opts, xSc, ticks(xlo, xhi, 4)));

    let color = palette(opts, 0);
    for ((x, y) in pts.vals()) {
      let cx = xSc(x);
      let cy = ySc(y);
      b.add("<circle class=\"mv-chart-point\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"4\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc("(" # fmtNum(x) # ", " # fmtNum(y) # opts.unit # ")") # "</title></circle>");
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };


  // ===== BubbleChart =====
  // ---- BubbleChart: xy + size ----------------------------------------------
  /// `<BubbleChart points="1,2,8;3,5,20;4,4,12" />`. The third value is the
  /// bubble magnitude (area-proportional radius). Each bubble cycles the
  /// categorical palette so they read as distinct entities.
  public func bubble(pointsSpec : Text, opts : O) : Text {
    let pts = parseXYZ(pointsSpec);
    let n = pts.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-bubble") # svgClose() };

    let xs = Array.map<(Float, Float, Float), Float>(pts, func(p) { p.0 });
    let ys = Array.map<(Float, Float, Float), Float>(pts, func(p) { p.1 });
    let zs = Array.map<(Float, Float, Float), Float>(pts, func(p) { p.2 });
    let (xlo, xhi) = xDomainNice(arrMin(xs), arrMax(xs));
    let (ylo, yhi) = yDomain(opts, arrMin(ys), arrMax(ys), false);
    let zmax = maxF(arrMax(zs), 1.0);
    let xSc = linScale(xlo, xhi, plotLeft(), plotRight(opts));
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());

    let b = Buffer.Buffer<Text>(n + 8);
    b.add(svgOpen(opts, "mv-chart-bubble"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 4)));
    b.add(axisBottomNumeric(opts, xSc, ticks(xlo, xhi, 4)));

    let rMax : Float = 28.0; // max bubble radius in px
    let rMin : Float = 4.0;
    var i : Nat = 0;
    for ((x, y, z) in pts.vals()) {
      let cx = xSc(x);
      let cy = ySc(y);
      // area-proportional radius: r = sqrt(z/zmax) * (rMax-rMin) + rMin
      let frac = if (z <= 0.0) { 0.0 } else { sqrt_(z / zmax) };
      let r = rMin + frac * (rMax - rMin);
      let color = palette(opts, i);
      b.add("<circle class=\"mv-chart-bubble-pt\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"" # fmtNum(r) # "\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc("(" # fmtNum(x) # ", " # fmtNum(y) # ") size " # fmtNum(z) # opts.unit) # "</title></circle>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };


  // ===== LineChart =====
  // =========================================================================
  // ====  LINE / AREA CHART FAMILY  ==========================================
  // ====  Shared private helpers (declared once, used by every chart in the
  // ====  family below). They build ONLY on the foundation helpers above.
  // =========================================================================

  /// Resolve series from EITHER a multi-series "A:1,2;B:3,4" spec OR a plain
  /// "values" CSV ("1,2,3"). A plain CSV parses (via parseSeries) to a single
  /// unnamed series, so one code path serves both `values=` and `series=`.
  func seriesOf(spec : Text, valuesCsv : Text) : [Series] {
    if (valuesCsv != "") { [("", parseFloats(valuesCsv))] } else { parseSeries(spec) };
  };

  /// Longest value-count across all series (the number of x slots).
  func maxLen(ss : [Series]) : Nat {
    var m : Nat = 0;
    for ((_, vs) in ss.vals()) { if (vs.size() > m) { m := vs.size() } };
    m;
  };

  /// Every value across all series flattened (for auto y-domain).
  func flatten(ss : [Series]) : [Float] {
    let out = Buffer.Buffer<Float>(16);
    for ((_, vs) in ss.vals()) { for (v in vs.vals()) { out.add(v) } };
    Buffer.toArray(out);
  };

  /// [lo, hi] y-domain honouring opts.yMin/yMax, else auto: lo = min(data,0) so
  /// an all-positive series shows a 0 baseline; hi = niceCeil(max).
  func yDomainArr(opts : O, vals : [Float]) : (Float, Float) {
    let dataMax = if (vals.size() == 0) { 1.0 } else { arrMax(vals) };
    let dataMin = if (vals.size() == 0) { 0.0 } else { arrMin(vals) };
    let lo = switch (opts.yMin) {
      case (?m) { m };
      case null { if (dataMin < 0.0) { -niceCeil(-dataMin) } else { 0.0 } };
    };
    let hi = switch (opts.yMax) { case (?m) { m }; case null { niceCeil(dataMax) } };
    (lo, if (hi <= lo) { lo + 1.0 } else { hi });
  };

  /// X pixel centers for `n` evenly spaced category slots across the plot
  /// (single point is centered).
  func xCenters(opts : O, n : Nat) : [Float] {
    let x0 = plotLeft();
    let x1 = plotRight(opts);
    let out = Buffer.Buffer<Float>(n);
    if (n == 0) { return [] };
    if (n == 1) { out.add((x0 + x1) / 2.0); return Buffer.toArray(out) };
    let step = (x1 - x0) / Float.fromInt(n - 1);
    var i : Nat = 0;
    while (i < n) { out.add(x0 + step * Float.fromInt(i)); i += 1 };
    Buffer.toArray(out);
  };

  /// Per-series legend entries [(name, color)] (unnamed -> "Series N").
  func legendEntries(opts : O, ss : [Series]) : [(Text, Text)] {
    let out = Buffer.Buffer<(Text, Text)>(ss.size());
    var i : Nat = 0;
    for ((name, _) in ss.vals()) {
      out.add((if (name == "") { "Series " # Nat.toText(i + 1) } else { name }, palette(opts, i)));
      i += 1;
    };
    Buffer.toArray(out);
  };

  /// One vertex <circle> with a native <title> tooltip (no JS).
  func dot(cx : Float, cy : Float, color : Text, lbl : Text, v : Float, unit : Text) : Text {
    "<circle class=\"mv-chart-dot\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
      # "\" r=\"3\" fill=\"" # esc(color) # "\"><title>"
      # esc(lbl # (if (lbl == "") { "" } else { ": " }) # fmtNum(v) # unit)
      # "</title></circle>";
  };

  /// Smooth curve through pixel points via Catmull-Rom -> cubic Bezier (1/6
  /// tension). < 3 points falls back to straight "L" joins.
  func smoothPath(pts : [(Float, Float)]) : Text {
    let n = pts.size();
    if (n == 0) { return "" };
    let (x0, y0) = pts[0];
    if (n < 3) {
      let b = Buffer.Buffer<Text>(n + 1);
      b.add("M " # fmtNum(x0) # "," # fmtNum(y0));
      var i : Nat = 1;
      while (i < n) { let (x, y) = pts[i]; b.add("L " # fmtNum(x) # "," # fmtNum(y)); i += 1 };
      return Text.join(" ", b.vals());
    };
    let b = Buffer.Buffer<Text>(n + 1);
    b.add("M " # fmtNum(x0) # "," # fmtNum(y0));
    var i : Nat = 0;
    while (i + 1 < n) {
      let p0 = if (i == 0) { pts[0] } else { pts[i - 1] };
      let p1 = pts[i];
      let p2 = pts[i + 1];
      let p3 = if (i + 2 < n) { pts[i + 2] } else { pts[i + 1] };
      let (p0x, p0y) = p0; let (p1x, p1y) = p1;
      let (p2x, p2y) = p2; let (p3x, p3y) = p3;
      let c1x = p1x + (p2x - p0x) / 6.0;
      let c1y = p1y + (p2y - p0y) / 6.0;
      let c2x = p2x - (p3x - p1x) / 6.0;
      let c2y = p2y - (p3y - p1y) / 6.0;
      b.add("C " # fmtNum(c1x) # "," # fmtNum(c1y) # " "
        # fmtNum(c2x) # "," # fmtNum(c2y) # " " # fmtNum(p2x) # "," # fmtNum(p2y));
      i += 1;
    };
    Text.join(" ", b.vals());
  };

  /// Stepped (HV) path: hold each value, jump at the next x.
  func stepPath(pts : [(Float, Float)]) : Text {
    let n = pts.size();
    if (n == 0) { return "" };
    let (x0, y0) = pts[0];
    let b = Buffer.Buffer<Text>(n * 2);
    b.add("M " # fmtNum(x0) # "," # fmtNum(y0));
    var i : Nat = 1;
    while (i < n) {
      let (x, y) = pts[i];
      b.add("H " # fmtNum(x)); // horizontal, holding previous y
      b.add("V " # fmtNum(y)); // then vertical to the new value
      i += 1;
    };
    Text.join(" ", b.vals());
  };

  // kind: 0 = straight | 1 = smooth | 2 = step.
  func linePath(pts : [(Float, Float)], kind : Nat) : Text {
    if (kind == 1) { smoothPath(pts) } else if (kind == 2) { stepPath(pts) } else {
      let n = pts.size();
      if (n == 0) { return "" };
      let (x0, y0) = pts[0];
      let b = Buffer.Buffer<Text>(n + 1);
      b.add("M " # fmtNum(x0) # "," # fmtNum(y0));
      var i : Nat = 1;
      while (i < n) { let (x, y) = pts[i]; b.add("L " # fmtNum(x) # "," # fmtNum(y)); i += 1 };
      Text.join(" ", b.vals());
    };
  };

  /// Pixel points for one series' values along the shared x centers.
  func seriesPts(vs : [Float], centers : [Float], ySc : Scale) : [(Float, Float)] {
    let out = Buffer.Buffer<(Float, Float)>(vs.size());
    var i : Nat = 0;
    while (i < vs.size() and i < centers.size()) { out.add((centers[i], ySc(vs[i]))); i += 1 };
    Buffer.toArray(out);
  };

  // ---- core line/area renderer (line, spline, step, area all flow here) ----
  // kind: 0 straight | 1 smooth | 2 step ; filled = draw the area under each line.
  func renderLines(spec : Text, valuesCsv : Text, labelsCsv : Text, opts : O, modClass : Text, kind : Nat, filled : Bool) : Text {
    let ss = seriesOf(spec, valuesCsv);
    let n = maxLen(ss);
    let labels = parseLabels(labelsCsv);
    let (lo, hi) = yDomainArr(opts, flatten(ss));
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let centers = xCenters(opts, n);
    let baseY = ySc(if (lo > 0.0) { lo } else { 0.0 });

    let b = Buffer.Buffer<Text>(ss.size() * 3 + 8);
    b.add(svgOpen(opts, modClass));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 5)));
    if (labels.size() > 0) { b.add(axisBottom(opts, labels, centers)) }
    else if (opts.showAxes) { b.add(axisBottom(opts, [], centers)) };

    var si : Nat = 0;
    for ((name, vs) in ss.vals()) {
      let color = palette(opts, si);
      let pts = seriesPts(vs, centers, ySc);
      if (pts.size() > 0) {
        if (filled) {
          let d = if (kind == 1) {
            let (fx, _) = pts[0]; let (lx, _) = pts[pts.size() - 1];
            smoothPath(pts) # " L " # fmtNum(lx) # "," # fmtNum(baseY) # " L " # fmtNum(fx) # "," # fmtNum(baseY) # " Z";
          } else if (kind == 2) {
            let (fx, _) = pts[0]; let (lx, _) = pts[pts.size() - 1];
            stepPath(pts) # " L " # fmtNum(lx) # "," # fmtNum(baseY) # " L " # fmtNum(fx) # "," # fmtNum(baseY) # " Z";
          } else { areaPath(pts, baseY) };
          b.add("<path class=\"mv-chart-area\" d=\"" # d # "\" fill=\"" # esc(color) # "\" fill-opacity=\"0.18\"/>");
        };
        b.add("<path class=\"mv-chart-line\" d=\"" # linePath(pts, kind) # "\" fill=\"none\" stroke=\"" # esc(color) # "\"/>");
        var pi : Nat = 0;
        for ((cx, cy) in pts.vals()) {
          let lbl = if (pi < labels.size()) { labels[pi] } else { name };
          b.add(dot(cx, cy, color, lbl, vs[pi], opts.unit));
          pi += 1;
        };
      };
      si += 1;
    };

    b.add(legend(opts, legendEntries(opts, ss)));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  /// LineChart — multi-series straight polylines with axes, grid, legend,
  /// vertex dots + native <title> tooltips. `data` is a multi-series
  /// "A:1,2;B:3,4" spec OR a plain values CSV "1,2,3" (one unnamed series).
  public func line(data : Text, labels : Text, opts : O) : Text {
    renderLines(data, "", labels, opts, "mv-chart-line-c", 0, false);
  };


  // ===== SplineChart =====
  /// SplineChart — smoothed (Catmull-Rom cubic Bezier) multi-series line.
  /// Same data conventions as LineChart.
  public func spline(data : Text, labels : Text, opts : O) : Text {
    renderLines(data, "", labels, opts, "mv-chart-spline", 1, false);
  };


  // ===== StepLineChart =====
  /// StepLineChart — stepped (HV) multi-series line; good for discrete state
  /// changes (plan tier, status, counts). Same data conventions as LineChart.
  public func stepLine(data : Text, labels : Text, opts : O) : Text {
    renderLines(data, "", labels, opts, "mv-chart-step", 2, false);
  };


  // ===== AreaChart =====
  /// AreaChart — a filled straight line (single or multi-series). Each series
  /// fills down to the 0 baseline at 0.18 opacity. Same data conventions as
  /// LineChart.
  public func area(data : Text, labels : Text, opts : O) : Text {
    renderLines(data, "", labels, opts, "mv-chart-area-c", 0, true);
  };


  // ===== StackedAreaChart =====
  // ---- stacked-area renderer -----------------------------------------------
  // Series are summed cumulatively; each band fills between the running baseline
  // and the new running total. Shared x = labels. Top edge is stroked; the band
  // carries a native <title> tooltip.
  func renderStackedArea(spec : Text, labelsCsv : Text, opts : O, kind : Nat) : Text {
    let ss = seriesOf(spec, "");
    let n = maxLen(ss);
    let labels = parseLabels(labelsCsv);

    let totals = Array.init<Float>(n, 0.0);
    for ((_, vs) in ss.vals()) {
      var i : Nat = 0;
      while (i < n) { let v = if (i < vs.size()) { vs[i] } else { 0.0 }; totals[i] += v; i += 1 };
    };
    let (lo, hi) = yDomainArr(opts, Array.freeze(totals));
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let centers = xCenters(opts, n);

    let b = Buffer.Buffer<Text>(ss.size() * 2 + 8);
    b.add(svgOpen(opts, "mv-chart-stackedarea"));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 5)));
    if (labels.size() > 0) { b.add(axisBottom(opts, labels, centers)) }
    else if (opts.showAxes) { b.add(axisBottom(opts, [], centers)) };

    let running = Array.init<Float>(n, 0.0);
    var si : Nat = 0;
    for ((name, vs) in ss.vals()) {
      let color = palette(opts, si);
      let top = Buffer.Buffer<(Float, Float)>(n);
      let bot = Buffer.Buffer<(Float, Float)>(n);
      var i : Nat = 0;
      while (i < n) {
        let v = if (i < vs.size()) { vs[i] } else { 0.0 };
        let base0 = running[i];
        let base1 = base0 + v;
        bot.add((centers[i], ySc(base0)));
        top.add((centers[i], ySc(base1)));
        running[i] := base1;
        i += 1;
      };
      let topArr = Buffer.toArray(top);
      let botArr = Buffer.toArray(bot);
      if (topArr.size() > 0) {
        let topD = linePath(topArr, kind);
        let rev = Buffer.Buffer<(Float, Float)>(botArr.size());
        var j : Int = botArr.size() - 1;
        while (j >= 0) { rev.add(botArr[Int.abs(j)]); j -= 1 };
        let backB = Buffer.Buffer<Text>(botArr.size());
        for ((x, y) in rev.vals()) { backB.add("L " # fmtNum(x) # "," # fmtNum(y)) };
        let d = topD # " " # Text.join(" ", backB.vals()) # " Z";
        let nm = if (name == "") { "Series " # Nat.toText(si + 1) } else { name };
        b.add("<path class=\"mv-chart-area mv-chart-stackband\" d=\"" # d # "\" fill=\"" # esc(color) # "\" fill-opacity=\"0.55\"><title>" # esc(nm) # "</title></path>");
        b.add("<path class=\"mv-chart-line\" d=\"" # topD # "\" fill=\"none\" stroke=\"" # esc(color) # "\"/>");
      };
      si += 1;
    };

    b.add(legend(opts, legendEntries(opts, ss)));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  /// StackedAreaChart — cumulative filled bands (multi-series spec required;
  /// shared x = labels). Each band stacks on the running total of those below.
  public func stackedArea(spec : Text, labels : Text, opts : O) : Text {
    renderStackedArea(spec, labels, opts, 0);
  };


  // ===== Sparkline =====
  /// Sparkline — tiny inline trend line: NO axes/grid/legend/title-text/dots,
  /// just the polyline + a last-point marker. Pass a single values CSV (a
  /// multi-series spec is accepted but only the first series is drawn).
  /// Defaults to a compact 120x32 viewBox unless width/height are overridden.
  public func sparkline(values : Text, opts0 : O) : Text {
    let small : O = {
      width = if (opts0.width == def.width) { 120 } else { opts0.width };
      height = if (opts0.height == def.height) { 32 } else { opts0.height };
      title = opts0.title;
      colors = opts0.colors;
      showAxes = false;
      showGrid = false;
      showLegend = false;
      yMin = opts0.yMin;
      yMax = opts0.yMax;
      unit = opts0.unit;
    };
    let ss = seriesOf(values, "");
    let vs = if (ss.size() > 0) { ss[0].1 } else { [] };
    let (lo, hi) = yDomainArr(small, vs);
    let w = Float.fromInt(Int.abs(small.width));
    let h = Float.fromInt(Int.abs(small.height));
    let inset : Float = 2.0;
    let ySc = linScale(lo, hi, h - inset, inset);
    let n = vs.size();
    let centers = Buffer.Buffer<Float>(n);
    if (n == 1) { centers.add(w / 2.0) }
    else if (n > 1) {
      let step = (w - inset * 2.0) / Float.fromInt(n - 1);
      var i : Nat = 0;
      while (i < n) { centers.add(inset + step * Float.fromInt(i)); i += 1 };
    };
    let cArr = Buffer.toArray(centers);
    let pts = seriesPts(vs, cArr, ySc);
    let color = palette(small, 0);

    var s = "<svg class=\"mv-chart mv-chart-sparkline\" viewBox=\"0 0 "
      # Nat.toText(small.width) # " " # Nat.toText(small.height)
      # "\" role=\"img\" preserveAspectRatio=\"none\" xmlns=\"http://www.w3.org/2000/svg\">";
    if (small.title != "") { s #= "<title>" # esc(small.title) # "</title>" };
    if (pts.size() > 0) {
      s #= "<polyline class=\"mv-chart-line\" points=\"" # polyPoints(pts) # "\" fill=\"none\" stroke=\"" # esc(color) # "\"/>";
      let (lx, ly) = pts[pts.size() - 1];
      s #= "<circle class=\"mv-chart-dot\" cx=\"" # fmtNum(lx) # "\" cy=\"" # fmtNum(ly) # "\" r=\"2\" fill=\"" # esc(color) # "\"/>";
    };
    s #= svgClose();
    s;
  };


  // ===== PieChart =====
// ---- PieChart ------------------------------------------------------------
/// Full pie. `<PieChart values="42,30,55,20" labels="Q1,Q2,Q3,Q4" />`.
/// Each wedge gets a native <title> tooltip with value + percentage.
public func pie(values : Text, labels : Text, opts : O) : Text {
  pieLike(parseFloats(values), parseLabels(labels), opts, 0.0, "mv-chart-pie", "");
};

// Shared helpers used by pie/donut (place once, above `pie`):

// Center of the plot area (above any title row), used by all radial charts.
func centerXY(opts : O) : (Float, Float) {
  let cx = Float.fromInt(Int.abs(opts.width)) / 2.0;
  let top = if (opts.title != "") { 30.0 } else { 8.0 };
  let bot = if (opts.showLegend) { 24.0 } else { 8.0 };
  let h = Float.fromInt(Int.abs(opts.height));
  let cy = top + (h - top - bot) / 2.0;
  (cx, cy);
};
func minF(a : Float, b : Float) : Float { if (a < b) { a } else { b } };
func sum(xs : [Float]) : Float { var s : Float = 0.0; for (x in xs.vals()) { if (x > 0.0) { s += x } }; s };
func labelAt(labels : [Text], i : Nat) : Text { if (i < labels.size()) { labels[i] } else { "" } };

// The shared pie/donut renderer: innerFrac in 0..1 (0 = pie, 0.62 = donut).
func pieLike(values : [Float], labels : [Text], opts : O, innerFrac : Float, modClass : Text, centerText : Text) : Text {
  let b = Buffer.Buffer<Text>(values.size() * 2 + 6);
  b.add(svgOpen(opts, modClass));
  let (cx, cy) = centerXY(opts);
  let maxR = minF(cx, cy) - 8.0;
  let outerR = if (maxR < 4.0) { 4.0 } else { maxR };
  let innerR = outerR * innerFrac;
  let total = sum(values);
  if (total <= 0.0) {
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  b.add("<g class=\"mv-chart-slices\">");
  let legendEntries = Buffer.Buffer<(Text, Text)>(values.size());
  var acc : Float = 0.0;
  var i : Nat = 0;
  while (i < values.size()) {
    let v = values[i];
    if (v > 0.0) {
      let startFrac = acc / total;
      acc += v;
      let endFrac = acc / total;
      let color = palette(opts, i);
      let name = labelAt(labels, i);
      let pct = v / total * 100.0;
      let tip = (if (name != "") { name # ": " } else { "" }) # fmtNum(v) # opts.unit # " (" # fmtNum(pct) # "%)";
      b.add("<path class=\"mv-chart-slice\" fill=\"" # esc(color) # "\" d=\"" # arcPath(cx, cy, outerR, innerR, startFrac, endFrac) # "\"><title>" # esc(tip) # "</title></path>");
      legendEntries.add((if (name != "") { name } else { fmtNum(pct) # "%" }, color));
    };
    i += 1;
  };
  b.add("</g>");
  if (centerText != "") {
    b.add("<text class=\"mv-chart-center\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy + 5.0) # "\" text-anchor=\"middle\">" # esc(centerText) # "</text>");
  };
  b.add(legend(opts, Buffer.toArray(legendEntries)));
  b.add(svgClose());
  Text.join("", b.vals());
};


  // ===== DonutChart =====
// ---- DonutChart ----------------------------------------------------------
/// Donut (pie with a hole) that prints the total in the center.
/// `<DonutChart values="42,30,55,20" labels="Q1,Q2,Q3,Q4" />`.
/// Reuses the shared pieLike renderer (see PieChart) with innerFrac 0.62 and a
/// centered total label.
public func donut(values : Text, labels : Text, opts : O) : Text {
  let vs = parseFloats(values);
  let center = fmtNum(sum(vs)) # opts.unit;
  pieLike(vs, parseLabels(labels), opts, 0.62, "mv-chart-donut", center);
};


  // ===== SemiDonutChart =====
// ---- SemiDonutChart ------------------------------------------------------
/// Half donut: slices laid out across the top semicircle (9 o'clock through 12
/// to 3 o'clock). Compact composition meter; prints the total in the hollow.
/// `<SemiDonutChart values="60,30,10" labels="Direct,Search,Social" />`.
public func semiDonut(values : Text, labels : Text, opts : O) : Text {
  let vs = parseFloats(values);
  let labs = parseLabels(labels);
  let b = Buffer.Buffer<Text>(vs.size() * 2 + 6);
  b.add(svgOpen(opts, "mv-chart-semidonut"));
  let cx = Float.fromInt(Int.abs(opts.width)) / 2.0;
  let h = Float.fromInt(Int.abs(opts.height));
  let cy = h - (if (opts.showLegend) { 26.0 } else { 12.0 });
  let maxR = minF(cx - 8.0, cy - (if (opts.title != "") { 30.0 } else { 8.0 }));
  let outerR = if (maxR < 4.0) { 4.0 } else { maxR };
  let innerR = outerR * 0.6;
  let total = sum(vs);
  if (total <= 0.0) {
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  // top semicircle spans frac 0.75 -> 1.25 (through 12 o'clock).
  let startBase : Float = 0.75;
  let spanFrac : Float = 0.5;
  b.add("<g class=\"mv-chart-slices\">");
  let legendEntries = Buffer.Buffer<(Text, Text)>(vs.size());
  var acc : Float = 0.0;
  var i : Nat = 0;
  while (i < vs.size()) {
    let v = vs[i];
    if (v > 0.0) {
      let s0 = startBase + (acc / total) * spanFrac;
      acc += v;
      let s1 = startBase + (acc / total) * spanFrac;
      let color = palette(opts, i);
      let name = labelAt(labs, i);
      let pct = v / total * 100.0;
      let tip = (if (name != "") { name # ": " } else { "" }) # fmtNum(v) # opts.unit # " (" # fmtNum(pct) # "%)";
      b.add("<path class=\"mv-chart-slice\" fill=\"" # esc(color) # "\" d=\"" # arcPath(cx, cy, outerR, innerR, s0, s1) # "\"><title>" # esc(tip) # "</title></path>");
      legendEntries.add((if (name != "") { name } else { fmtNum(pct) # "%" }, color));
    };
    i += 1;
  };
  b.add("</g>");
  b.add("<text class=\"mv-chart-center\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy - 4.0) # "\" text-anchor=\"middle\">" # esc(fmtNum(total) # opts.unit) # "</text>");
  b.add(legend(opts, Buffer.toArray(legendEntries)));
  b.add(svgClose());
  Text.join("", b.vals());
};


  // ===== GaugeChart =====
// ---- GaugeChart ----------------------------------------------------------
/// Single-value gauge: a 180 arc from yMin..yMax (default 0..100) with the
/// filled portion up to `value` colored brand, a track behind it, and the value
/// printed big in the middle. Only the first parsed value is used.
/// `<GaugeChart values="72" yMax=100 unit="%" title="CPU" />`.
public func gauge(values : Text, opts : O) : Text {
  let vs = parseFloats(values);
  let v = if (vs.size() > 0) { vs[0] } else { 0.0 };
  let lo = switch (opts.yMin) { case (?m) { m }; case null { 0.0 } };
  let hi = switch (opts.yMax) { case (?m) { m }; case null { 100.0 } };
  let span = if (hi - lo == 0.0) { 1.0 } else { hi - lo };
  var frac = (v - lo) / span;
  if (frac < 0.0) { frac := 0.0 };
  if (frac > 1.0) { frac := 1.0 };
  let b = Buffer.Buffer<Text>(8);
  b.add(svgOpen(opts, "mv-chart-gauge"));
  let cx = Float.fromInt(Int.abs(opts.width)) / 2.0;
  let h = Float.fromInt(Int.abs(opts.height));
  let cy = h - (if (opts.title != "") { 16.0 } else { 12.0 });
  let maxR = minF(cx - 8.0, cy - (if (opts.title != "") { 30.0 } else { 8.0 }));
  let outerR = if (maxR < 4.0) { 4.0 } else { maxR };
  let innerR = outerR * 0.62;
  let startBase : Float = 0.75;
  let spanFrac : Float = 0.5;
  let color = palette(opts, 0);
  b.add("<path class=\"mv-chart-gauge-track\" d=\"" # arcPath(cx, cy, outerR, innerR, startBase, startBase + spanFrac) # "\"/>");
  if (frac > 0.0) {
    let tip = fmtNum(v) # opts.unit # " of " # fmtNum(hi) # opts.unit;
    b.add("<path class=\"mv-chart-gauge-value\" fill=\"" # esc(color) # "\" d=\"" # arcPath(cx, cy, outerR, innerR, startBase, startBase + frac * spanFrac) # "\"><title>" # esc(tip) # "</title></path>");
  };
  b.add("<text class=\"mv-chart-center mv-chart-gauge-num\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy - 6.0) # "\" text-anchor=\"middle\">" # esc(fmtNum(v) # opts.unit) # "</text>");
  if (opts.showAxes) {
    let lx = cx - (outerR + innerR) / 2.0;
    let rx = cx + (outerR + innerR) / 2.0;
    b.add("<text class=\"mv-chart-tick\" x=\"" # fmtNum(lx) # "\" y=\"" # fmtNum(cy + 14.0) # "\" text-anchor=\"middle\">" # esc(fmtNum(lo) # opts.unit) # "</text>");
    b.add("<text class=\"mv-chart-tick\" x=\"" # fmtNum(rx) # "\" y=\"" # fmtNum(cy + 14.0) # "\" text-anchor=\"middle\">" # esc(fmtNum(hi) # opts.unit) # "</text>");
  };
  b.add(svgClose());
  Text.join("", b.vals());
};


  // ===== RadarChart =====
// ---- RadarChart ----------------------------------------------------------
/// Multi-axis radar/spider. Series spec shares the axis labels:
/// `<RadarChart series="Team A:80,70,90,60,75;Team B:60,85,70,80,65"
///             labels="Speed,Power,Range,Agility,Defense" />`.
/// Each series is a translucent polygon with a <title> tooltip per vertex.
public func radar(spec : Text, labels : Text, opts : O) : Text {
  let series = parseSeries(spec);
  let labs = parseLabels(labels);
  let b = Buffer.Buffer<Text>(series.size() * 3 + 12);
  b.add(svgOpen(opts, "mv-chart-radar"));
  let (cx, cy) = centerXY(opts);
  let maxR = minF(cx, cy) - 18.0;
  let radius = if (maxR < 8.0) { 8.0 } else { maxR };
  var axes : Nat = labs.size();
  for ((_, vals) in series.vals()) { if (vals.size() > axes) { axes := vals.size() } };
  if (axes == 0) {
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  let allVals = Buffer.Buffer<Float>(axes);
  for ((_, vals) in series.vals()) { for (x in vals.vals()) { allVals.add(x) } };
  let rawMax = arrMax(Buffer.toArray(allVals));
  let hi = switch (opts.yMax) { case (?m) { m }; case null { niceCeil(if (rawMax <= 0.0) { 1.0 } else { rawMax }) } };
  let denom = if (hi == 0.0) { 1.0 } else { hi };
  let twoPi : Float = 6.283185307179586;
  let angAt = func(i : Nat) : Float { twoPi * Float.fromInt(i) / Float.fromInt(axes) };
  if (opts.showGrid) {
    b.add("<g class=\"mv-chart-radar-grid\">");
    let rings : Nat = 4;
    var r : Nat = 1;
    while (r <= rings) {
      let rr = radius * Float.fromInt(r) / Float.fromInt(rings);
      let ring = Buffer.Buffer<(Float, Float)>(axes);
      var a : Nat = 0;
      while (a < axes) { ring.add((polarX(cx, rr, angAt(a)), polarY(cy, rr, angAt(a)))); a += 1 };
      b.add("<polygon class=\"mv-chart-grid\" points=\"" # polyPoints(Buffer.toArray(ring)) # "\"/>");
      r += 1;
    };
    var a2 : Nat = 0;
    while (a2 < axes) {
      b.add("<line class=\"mv-chart-grid\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(cy) # "\" x2=\"" # fmtNum(polarX(cx, radius, angAt(a2))) # "\" y2=\"" # fmtNum(polarY(cy, radius, angAt(a2))) # "\"/>");
      a2 += 1;
    };
    b.add("</g>");
  };
  if (opts.showAxes) {
    var a : Nat = 0;
    while (a < axes) {
      let lx = polarX(cx, radius + 12.0, angAt(a));
      let ly = polarY(cy, radius + 12.0, angAt(a));
      let anchor = if (lx > cx + 1.0) { "start" } else if (lx < cx - 1.0) { "end" } else { "middle" };
      b.add("<text class=\"mv-chart-tick mv-chart-radar-axis\" x=\"" # fmtNum(lx) # "\" y=\"" # fmtNum(ly + 3.0) # "\" text-anchor=\"" # anchor # "\">" # esc(labelAt(labs, a)) # "</text>");
      a += 1;
    };
  };
  let legendEntries = Buffer.Buffer<(Text, Text)>(series.size());
  var si : Nat = 0;
  for ((name, vals) in series.vals()) {
    let color = palette(opts, si);
    let pts = Buffer.Buffer<(Float, Float)>(axes);
    var a : Nat = 0;
    while (a < axes) {
      let v = if (a < vals.size()) { vals[a] } else { 0.0 };
      var fr = v / denom;
      if (fr < 0.0) { fr := 0.0 };
      let rr = radius * fr;
      pts.add((polarX(cx, rr, angAt(a)), polarY(cy, rr, angAt(a))));
      a += 1;
    };
    let ptsArr = Buffer.toArray(pts);
    b.add("<polygon class=\"mv-chart-radar-area\" fill=\"" # esc(color) # "\" stroke=\"" # esc(color) # "\" points=\"" # polyPoints(ptsArr) # "\"><title>" # esc(name) # "</title></polygon>");
    var k : Nat = 0;
    while (k < ptsArr.size()) {
      let (px, py) = ptsArr[k];
      let v = if (k < vals.size()) { vals[k] } else { 0.0 };
      let tip = (if (name != "") { name # " \u{b7} " } else { "" }) # labelAt(labs, k) # ": " # fmtNum(v) # opts.unit;
      b.add("<circle class=\"mv-chart-radar-dot\" cx=\"" # fmtNum(px) # "\" cy=\"" # fmtNum(py) # "\" r=\"2.5\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
      k += 1;
    };
    legendEntries.add((if (name != "") { name } else { "Series " # Nat.toText(si + 1) }, color));
    si += 1;
  };
  b.add(legend(opts, Buffer.toArray(legendEntries)));
  b.add(svgClose());
  Text.join("", b.vals());
};


  // ===== RadialBarChart =====
// ---- RadialBarChart ------------------------------------------------------
/// Concentric progress rings (a.k.a. radial bar). Each value is drawn as its
/// own ring, filled to value/max of the circle. Each ring has a faint track + a
/// colored arc with a <title> tooltip.
/// `<RadialBarChart values="80,55,30" labels="Sales,Marketing,Support" />`.
public func radialBar(values : Text, labels : Text, opts : O) : Text {
  let vs = parseFloats(values);
  let labs = parseLabels(labels);
  let b = Buffer.Buffer<Text>(vs.size() * 3 + 6);
  b.add(svgOpen(opts, "mv-chart-radialbar"));
  let (cx, cy) = centerXY(opts);
  let maxR = minF(cx, cy) - 8.0;
  let outerMost = if (maxR < 8.0) { 8.0 } else { maxR };
  let n = vs.size();
  if (n == 0) {
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  let rawMax = arrMax(vs);
  let hi = switch (opts.yMax) { case (?m) { m }; case null { if (rawMax <= 0.0) { 100.0 } else { niceCeil(rawMax) } } };
  let denom = if (hi == 0.0) { 1.0 } else { hi };
  let innerHole = outerMost * 0.30;
  let bandSpan = (outerMost - innerHole) / Float.fromInt(n);
  let gap = bandSpan * 0.22;
  let legendEntries = Buffer.Buffer<(Text, Text)>(n);
  var i : Nat = 0;
  while (i < n) {
    let rOuter = outerMost - bandSpan * Float.fromInt(i);
    let rInner = rOuter - bandSpan + gap;
    let v = vs[i];
    var fr = v / denom;
    if (fr < 0.0) { fr := 0.0 };
    if (fr > 1.0) { fr := 1.0 };
    let color = palette(opts, i);
    let name = labelAt(labs, i);
    // full track ring, drawn as two 180 arcs to dodge the full-circle
    // degenerate-path case in arcPath.
    b.add("<path class=\"mv-chart-radialbar-track\" d=\"" # arcPath(cx, cy, rOuter, rInner, 0.0, 0.5) # "\"/>");
    b.add("<path class=\"mv-chart-radialbar-track\" d=\"" # arcPath(cx, cy, rOuter, rInner, 0.5, 1.0) # "\"/>");
    if (fr > 0.0) {
      let pct = v / denom * 100.0;
      let tip = (if (name != "") { name # ": " } else { "" }) # fmtNum(v) # opts.unit # " (" # fmtNum(pct) # "%)";
      if (fr >= 1.0) {
        b.add("<path class=\"mv-chart-radialbar-value\" fill=\"" # esc(color) # "\" d=\"" # arcPath(cx, cy, rOuter, rInner, 0.0, 0.5) # "\"><title>" # esc(tip) # "</title></path>");
        b.add("<path class=\"mv-chart-radialbar-value\" fill=\"" # esc(color) # "\" d=\"" # arcPath(cx, cy, rOuter, rInner, 0.5, 1.0) # "\"/>");
      } else {
        b.add("<path class=\"mv-chart-radialbar-value\" fill=\"" # esc(color) # "\" d=\"" # arcPath(cx, cy, rOuter, rInner, 0.0, fr) # "\"><title>" # esc(tip) # "</title></path>");
      };
    };
    legendEntries.add((if (name != "") { name } else { fmtNum(v) # opts.unit }, color));
    i += 1;
  };
  b.add(legend(opts, Buffer.toArray(legendEntries)));
  b.add(svgClose());
  Text.join("", b.vals());
};


};
