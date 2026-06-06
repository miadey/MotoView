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

  // ===== LollipopChart =====
  // ---- LollipopChart: a thin stem + a dot at each value (vertical) ----------
  /// `<LollipopChart values="42,30,55,20" labels="Q1,Q2,Q3,Q4" />`. Like a
  /// column chart but drawn as a stem line topped by a circle — lighter ink,
  /// same comparison. Baseline is 0 (or yMin). Native <title> per lollipop.
  public func lollipop(valuesCsv : Text, labelsCsv : Text, opts : O) : Text {
    let vals = parseFloats(valuesCsv);
    let labels = parseLabels(labelsCsv);
    let n = vals.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-lollipop") # svgClose() };

    let (lo, hi) = lolliDomain(opts, arrMin(vals), arrMax(vals));
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(n);
    let baseY = ySc(if (lo < 0.0) { 0.0 } else { lo });

    let b = Buffer.Buffer<Text>(n * 2 + 8);
    b.add(svgOpen(opts, "mv-chart-lollipop"));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 4)));
    let centers = Array.tabulate<Float>(n, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    let color = palette(opts, 0);
    var i : Nat = 0;
    while (i < n) {
      let v = vals[i];
      let yv = ySc(v);
      let cx = centers[i];
      let lbl = labelAt(labels, i);
      let tip = lbl # (if (lbl == "") { "" } else { ": " }) # fmtNum(v) # opts.unit;
      b.add("<g class=\"mv-chart-lolli-g\">");
      b.add("<line class=\"mv-chart-lolli-stem\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(baseY)
        # "\" x2=\"" # fmtNum(cx) # "\" y2=\"" # fmtNum(yv) # "\" stroke=\"" # esc(color) # "\"/>");
      b.add("<circle class=\"mv-chart-lolli-dot\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(yv)
        # "\" r=\"5\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
      b.add("</g>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // lollipop y-domain: 0 baseline for all-positive, symmetric otherwise.
  func lolliDomain(opts : O, dataMin : Float, dataMax : Float) : (Float, Float) {
    let lo = switch (opts.yMin) { case (?v) { v };
      case null { if (dataMin < 0.0) { -niceCeil(-dataMin) } else { 0.0 } } };
    let hi = switch (opts.yMax) { case (?v) { v }; case null { niceCeil(dataMax) } };
    (lo, if (hi <= lo) { lo + 1.0 } else { hi });
  };

  // ===== BulletChart =====
  // ---- BulletChart: measure vs target over qualitative bands ----------------
  /// One horizontal bullet per row. Row spec (";"-separated):
  ///   name:value:target:b1,b2,b3   (bands ascend from 0; last band = scale max)
  /// `<BulletChart rows="Revenue:180:200:120,160,220" />`. A thick measure bar
  /// sits over graded background bands; a tick marks the target.
  public func bullet(rowsSpec : Text, opts : O) : Text {
    let rows = bulletParse(rowsSpec);
    let n = rows.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-bullet") # svgClose() };

    // global scale max = max over all bands/values/targets.
    var gmax : Float = 1.0;
    for (r in rows.vals()) {
      gmax := maxF(gmax, r.1);
      gmax := maxF(gmax, r.2);
      gmax := maxF(gmax, arrMax(r.3));
    };
    let hi = switch (opts.yMax) { case (?m) { m }; case null { niceCeil(gmax) } };
    let left = plotLeft();
    let right = plotRight(opts);
    let xSc = linScale(0.0, if (hi <= 0.0) { 1.0 } else { hi }, left, right);
    let top = plotTop();
    let bot = plotBottom(opts);
    let bandH = (bot - top) / Float.fromInt(n);
    let rowH = bandH * 0.7;

    let b = Buffer.Buffer<Text>(n * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-bullet"));
    // bottom numeric axis for the shared scale.
    b.add(axisBottomNumeric(opts, xSc, ticks(0.0, if (hi <= 0.0) { 1.0 } else { hi }, 4)));

    let measureColor = palette(opts, 0);
    var i : Nat = 0;
    while (i < n) {
      let (name, value, target, bands) = rows[i];
      let yc = top + bandH * Float.fromInt(i) + (bandH - rowH) / 2.0;
      // qualitative bands (lightest -> darkest), drawn back-to-front.
      var prev : Float = 0.0;
      var bi : Nat = 0;
      let sorted = bulletSort(bands);
      while (bi < sorted.size()) {
        let edge = sorted[bi];
        let x0 = xSc(prev);
        let x1 = xSc(edge);
        let shade = bulletShade(bi, sorted.size());
        b.add("<rect class=\"mv-chart-bullet-band\" x=\"" # fmtNum(x0) # "\" y=\"" # fmtNum(yc)
          # "\" width=\"" # fmtNum(maxF(0.0, x1 - x0)) # "\" height=\"" # fmtNum(rowH)
          # "\" fill-opacity=\"" # shade # "\"/>");
        prev := edge;
        bi += 1;
      };
      // measure bar (thinner, centered).
      let mH = rowH * 0.42;
      let my = yc + (rowH - mH) / 2.0;
      let mw = maxF(0.0, xSc(value) - left);
      b.add("<rect class=\"mv-chart-bullet-measure\" x=\"" # fmtNum(left) # "\" y=\"" # fmtNum(my)
        # "\" width=\"" # fmtNum(mw) # "\" height=\"" # fmtNum(mH) # "\" fill=\"" # esc(measureColor) # "\">"
        # "<title>" # esc(name # ": " # fmtNum(value) # opts.unit # " / target " # fmtNum(target) # opts.unit) # "</title></rect>");
      // target tick.
      let tx = xSc(target);
      b.add("<line class=\"mv-chart-bullet-target\" x1=\"" # fmtNum(tx) # "\" y1=\"" # fmtNum(yc)
        # "\" x2=\"" # fmtNum(tx) # "\" y2=\"" # fmtNum(yc + rowH) # "\"/>");
      // row label at left.
      b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(left - 6.0)
        # "\" y=\"" # fmtNum(yc + rowH / 2.0 + 4.0) # "\" text-anchor=\"end\">" # esc(name) # "</text>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse "name:value:target:b1,b2,.." rows separated by ';'.
  func bulletParse(spec : Text) : [(Text, Float, Float, [Float])] {
    let out = Buffer.Buffer<(Text, Float, Float, [Float])>(4);
    for (seg in Text.split(spec, #char ';')) {
      let s = Text.trimStart(Text.trimEnd(seg, #char ' '), #char ' ');
      if (s != "") {
        let parts = Iter.toArray(Text.split(s, #char ':'));
        if (parts.size() >= 4) {
          let name = Text.trimStart(Text.trimEnd(parts[0], #char ' '), #char ' ');
          let value = bulletF(parts[1]);
          let target = bulletF(parts[2]);
          let bands = parseFloats(parts[3]);
          out.add((name, value, target, bands));
        };
      };
    };
    Buffer.toArray(out);
  };
  func bulletF(t : Text) : Float {
    let fs = parseFloats(t);
    if (fs.size() > 0) { fs[0] } else { 0.0 };
  };
  // Ascending copy of band edges (insertion sort; tiny arrays).
  func bulletSort(xs : [Float]) : [Float] {
    let a = Array.thaw<Float>(xs);
    var i : Nat = 1;
    while (i < a.size()) {
      let key = a[i];
      var j : Int = Int.abs(i) - 1;
      while (j >= 0 and a[Int.abs(j)] > key) { a[Int.abs(j) + 1] := a[Int.abs(j)]; j -= 1 };
      a[Int.abs(j + 1)] := key;
      i += 1;
    };
    Array.freeze(a);
  };
  // Band shade opacity: lighter for lower bands, darker for higher.
  func bulletShade(idx : Nat, total : Nat) : Text {
    if (total == 0) { return "0.1" };
    let step = 0.22 / Float.fromInt(total);
    let op = 0.08 + step * Float.fromInt(idx + 1);
    fmtNum(op);
  };

  // ===== DotPlot =====
  // ---- DotPlot: one dot per category on a shared value axis ------------------
  /// `<DotPlot values="42,30,55,20" labels="Q1,Q2,Q3,Q4" />`. A Cleveland dot
  /// plot: each row is a category; a single dot marks its value along the bottom
  /// numeric axis. Light guide line per row + native <title> tooltip.
  public func dotPlot(valuesCsv : Text, labelsCsv : Text, opts : O) : Text {
    let vals = parseFloats(valuesCsv);
    let labels = parseLabels(labelsCsv);
    let n = vals.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-dotplot") # svgClose() };

    let lo = switch (opts.yMin) { case (?v) { v };
      case null { if (arrMin(vals) < 0.0) { -niceCeil(-arrMin(vals)) } else { 0.0 } } };
    let hi = switch (opts.yMax) { case (?v) { v }; case null { niceCeil(arrMax(vals)) } };
    let hiU = if (hi <= lo) { lo + 1.0 } else { hi };
    let left = plotLeft();
    let right = plotRight(opts);
    let xSc = linScale(lo, hiU, left, right);
    let top = plotTop();
    let bot = plotBottom(opts);
    let band = (bot - top) / Float.fromInt(n);

    let b = Buffer.Buffer<Text>(n * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-dotplot"));
    b.add(axisBottomNumeric(opts, xSc, ticks(lo, hiU, 4)));

    let color = palette(opts, 0);
    var i : Nat = 0;
    while (i < n) {
      let v = vals[i];
      let cy = top + band * (Float.fromInt(i) + 0.5);
      let cx = xSc(v);
      let lbl = labelAt(labels, i);
      // row guide line.
      b.add("<line class=\"mv-chart-dotplot-guide\" x1=\"" # fmtNum(left) # "\" y1=\"" # fmtNum(cy)
        # "\" x2=\"" # fmtNum(right) # "\" y2=\"" # fmtNum(cy) # "\"/>");
      b.add("<circle class=\"mv-chart-dotplot-dot\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"5\" fill=\"" # esc(color) # "\"><title>" # esc(lbl # (if (lbl == "") { "" } else { ": " }) # fmtNum(v) # opts.unit) # "</title></circle>");
      // category label at left.
      b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(left - 6.0)
        # "\" y=\"" # fmtNum(cy + 4.0) # "\" text-anchor=\"end\">" # esc(lbl) # "</text>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== DumbbellChart =====
  // ---- DumbbellChart: start->end pairs per category -------------------------
  /// One dumbbell per row. Row spec (";"-separated): label:start,end
  /// `<DumbbellChart rows="Q1:120,180;Q2:90,140" startName="2023" endName="2024" />`.
  /// Two colored dots joined by a connector show change between two states.
  public func dumbbell(rowsSpec : Text, startName : Text, endName : Text, opts : O) : Text {
    let rows = dumbParse(rowsSpec);
    let n = rows.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-dumbbell") # svgClose() };

    // value domain over both endpoints.
    let all = Buffer.Buffer<Float>(n * 2);
    for (r in rows.vals()) { all.add(r.1); all.add(r.2) };
    let arr = Buffer.toArray(all);
    let lo = switch (opts.yMin) { case (?v) { v };
      case null { if (arrMin(arr) < 0.0) { -niceCeil(-arrMin(arr)) } else { 0.0 } } };
    let hi = switch (opts.yMax) { case (?v) { v }; case null { niceCeil(arrMax(arr)) } };
    let hiU = if (hi <= lo) { lo + 1.0 } else { hi };
    let left = plotLeft();
    let right = plotRight(opts);
    let xSc = linScale(lo, hiU, left, right);
    let top = plotTop();
    let bot = plotBottom(opts);
    let band = (bot - top) / Float.fromInt(n);

    let cStart = palette(opts, 0);
    let cEnd = palette(opts, 1);

    let b = Buffer.Buffer<Text>(n * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-dumbbell"));
    b.add(axisBottomNumeric(opts, xSc, ticks(lo, hiU, 4)));

    var i : Nat = 0;
    while (i < n) {
      let (lbl, sv, ev) = rows[i];
      let cy = top + band * (Float.fromInt(i) + 0.5);
      let sx = xSc(sv);
      let ex = xSc(ev);
      b.add("<g class=\"mv-chart-dumbbell-g\">");
      b.add("<line class=\"mv-chart-dumbbell-bar\" x1=\"" # fmtNum(sx) # "\" y1=\"" # fmtNum(cy)
        # "\" x2=\"" # fmtNum(ex) # "\" y2=\"" # fmtNum(cy) # "\"/>");
      b.add("<circle class=\"mv-chart-dumbbell-dot\" cx=\"" # fmtNum(sx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"5\" fill=\"" # esc(cStart) # "\"><title>" # esc(lbl # " " # startName # ": " # fmtNum(sv) # opts.unit) # "</title></circle>");
      b.add("<circle class=\"mv-chart-dumbbell-dot\" cx=\"" # fmtNum(ex) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"5\" fill=\"" # esc(cEnd) # "\"><title>" # esc(lbl # " " # endName # ": " # fmtNum(ev) # opts.unit) # "</title></circle>");
      b.add("</g>");
      b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(left - 6.0)
        # "\" y=\"" # fmtNum(cy + 4.0) # "\" text-anchor=\"end\">" # esc(lbl) # "</text>");
      i += 1;
    };
    let sN = if (startName == "") { "Start" } else { startName };
    let eN = if (endName == "") { "End" } else { endName };
    b.add(legend(opts, [(sN, cStart), (eN, cEnd)]));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse "label:start,end" rows separated by ';'.
  func dumbParse(spec : Text) : [(Text, Float, Float)] {
    let out = Buffer.Buffer<(Text, Float, Float)>(4);
    for (seg in Text.split(spec, #char ';')) {
      let s = Text.trimStart(Text.trimEnd(seg, #char ' '), #char ' ');
      if (s != "") {
        let parts = Iter.toArray(Text.split(s, #char ':'));
        if (parts.size() >= 2) {
          let lbl = Text.trimStart(Text.trimEnd(parts[0], #char ' '), #char ' ');
          let pair = parseFloats(parts[1]);
          if (pair.size() >= 2) { out.add((lbl, pair[0], pair[1])) };
        };
      };
    };
    Buffer.toArray(out);
  };

  // ===== RangePlot =====
  // ---- RangePlot: a low..high floating bar per category --------------------
  /// One floating range bar per row. Row spec (";"-separated): label:low,high
  /// `<RangePlot rows="Mon:12,24;Tue:9,30" />`. Useful for min/max, hi/lo, or
  /// confidence ranges. End caps + native <title> tooltip per range.
  public func rangePlot(rowsSpec : Text, opts : O) : Text {
    let rows = rangeParse(rowsSpec);
    let n = rows.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-range") # svgClose() };

    let all = Buffer.Buffer<Float>(n * 2);
    for (r in rows.vals()) { all.add(r.1); all.add(r.2) };
    let arr = Buffer.toArray(all);
    let lo = switch (opts.yMin) { case (?v) { v };
      case null { if (arrMin(arr) < 0.0) { -niceCeil(-arrMin(arr)) } else { 0.0 } } };
    let hi = switch (opts.yMax) { case (?v) { v }; case null { niceCeil(arrMax(arr)) } };
    let hiU = if (hi <= lo) { lo + 1.0 } else { hi };
    let left = plotLeft();
    let right = plotRight(opts);
    let xSc = linScale(lo, hiU, left, right);
    let top = plotTop();
    let bot = plotBottom(opts);
    let band = (bot - top) / Float.fromInt(n);
    let barH = band * 0.4;
    let capH = band * 0.6;

    let color = palette(opts, 0);
    let b = Buffer.Buffer<Text>(n * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-range"));
    b.add(axisBottomNumeric(opts, xSc, ticks(lo, hiU, 4)));

    var i : Nat = 0;
    while (i < n) {
      let (lbl, lowV, highV) = rows[i];
      let cy = top + band * (Float.fromInt(i) + 0.5);
      let lowX = xSc(lowV);
      let highX = xSc(highV);
      let x0 = minF(lowX, highX);
      let w = Float.abs(highX - lowX);
      let tip = lbl # ": " # fmtNum(lowV) # opts.unit # " \u{2013} " # fmtNum(highV) # opts.unit;
      b.add("<g class=\"mv-chart-range-g\">");
      b.add("<rect class=\"mv-chart-range-bar\" x=\"" # fmtNum(x0) # "\" y=\"" # fmtNum(cy - barH / 2.0)
        # "\" width=\"" # fmtNum(w) # "\" height=\"" # fmtNum(barH) # "\" rx=\"2\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc(tip) # "</title></rect>");
      // end caps.
      b.add("<line class=\"mv-chart-range-cap\" x1=\"" # fmtNum(lowX) # "\" y1=\"" # fmtNum(cy - capH / 2.0)
        # "\" x2=\"" # fmtNum(lowX) # "\" y2=\"" # fmtNum(cy + capH / 2.0) # "\" stroke=\"" # esc(color) # "\"/>");
      b.add("<line class=\"mv-chart-range-cap\" x1=\"" # fmtNum(highX) # "\" y1=\"" # fmtNum(cy - capH / 2.0)
        # "\" x2=\"" # fmtNum(highX) # "\" y2=\"" # fmtNum(cy + capH / 2.0) # "\" stroke=\"" # esc(color) # "\"/>");
      b.add("</g>");
      b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(left - 6.0)
        # "\" y=\"" # fmtNum(cy + 4.0) # "\" text-anchor=\"end\">" # esc(lbl) # "</text>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse "label:low,high" rows separated by ';'.
  func rangeParse(spec : Text) : [(Text, Float, Float)] {
    let out = Buffer.Buffer<(Text, Float, Float)>(4);
    for (seg in Text.split(spec, #char ';')) {
      let s = Text.trimStart(Text.trimEnd(seg, #char ' '), #char ' ');
      if (s != "") {
        let parts = Iter.toArray(Text.split(s, #char ':'));
        if (parts.size() >= 2) {
          let lbl = Text.trimStart(Text.trimEnd(parts[0], #char ' '), #char ' ');
          let pair = parseFloats(parts[1]);
          if (pair.size() >= 2) { out.add((lbl, pair[0], pair[1])) };
        };
      };
    };
    Buffer.toArray(out);
  };

  // ===== SlopeChart =====
  // ---- SlopeChart: before->after connecting lines (two columns) -------------
  /// One slope line per row. Row spec (";"-separated): label:before,after
  /// `<SlopeChart rows="Alpha:120,180;Beta:90,75" beforeName="2023" afterName="2024" />`.
  /// Two value columns; a line per category connects its two values, with the
  /// category + value labelled at each end. Native <title> tooltip per line.
  public func slope(rowsSpec : Text, beforeName : Text, afterName : Text, opts : O) : Text {
    let rows = slopeParse(rowsSpec);
    let n = rows.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-slope") # svgClose() };

    let all = Buffer.Buffer<Float>(n * 2);
    for (r in rows.vals()) { all.add(r.1); all.add(r.2) };
    let arr = Buffer.toArray(all);
    let lo = switch (opts.yMin) { case (?v) { v };
      case null { if (arrMin(arr) < 0.0) { -niceCeil(-arrMin(arr)) } else { 0.0 } } };
    let hi = switch (opts.yMax) { case (?v) { v }; case null { niceCeil(arrMax(arr)) } };
    let hiU = if (hi <= lo) { lo + 1.0 } else { hi };
    let ySc = linScale(lo, hiU, plotBottom(opts), plotTop());
    // two columns, inset from the plot edges for the value labels.
    let xL = plotLeft() + (plotRight(opts) - plotLeft()) * 0.18;
    let xR = plotRight(opts) - (plotRight(opts) - plotLeft()) * 0.18;

    let b = Buffer.Buffer<Text>(n * 3 + 10);
    b.add(svgOpen(opts, "mv-chart-slope"));
    // vertical reference axes.
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(xL) # "\" y1=\"" # fmtNum(plotTop())
        # "\" x2=\"" # fmtNum(xL) # "\" y2=\"" # fmtNum(plotBottom(opts)) # "\"/>");
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(xR) # "\" y1=\"" # fmtNum(plotTop())
        # "\" x2=\"" # fmtNum(xR) # "\" y2=\"" # fmtNum(plotBottom(opts)) # "\"/>");
    };
    // column headers.
    let bN = if (beforeName == "") { "Before" } else { beforeName };
    let aN = if (afterName == "") { "After" } else { afterName };
    b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(xL) # "\" y=\"" # fmtNum(plotBottom(opts) + 16.0) # "\" text-anchor=\"middle\">" # esc(bN) # "</text>");
    b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(xR) # "\" y=\"" # fmtNum(plotBottom(opts) + 16.0) # "\" text-anchor=\"middle\">" # esc(aN) # "</text>");

    var i : Nat = 0;
    while (i < n) {
      let (lbl, bv, av) = rows[i];
      let color = palette(opts, i);
      let yL = ySc(bv);
      let yR = ySc(av);
      let tip = lbl # ": " # fmtNum(bv) # opts.unit # " \u{2192} " # fmtNum(av) # opts.unit;
      b.add("<g class=\"mv-chart-slope-g\">");
      b.add("<line class=\"mv-chart-slope-line\" x1=\"" # fmtNum(xL) # "\" y1=\"" # fmtNum(yL)
        # "\" x2=\"" # fmtNum(xR) # "\" y2=\"" # fmtNum(yR) # "\" stroke=\"" # esc(color) # "\">"
        # "<title>" # esc(tip) # "</title></line>");
      b.add("<circle class=\"mv-chart-slope-dot\" cx=\"" # fmtNum(xL) # "\" cy=\"" # fmtNum(yL) # "\" r=\"3.5\" fill=\"" # esc(color) # "\"/>");
      b.add("<circle class=\"mv-chart-slope-dot\" cx=\"" # fmtNum(xR) # "\" cy=\"" # fmtNum(yR) # "\" r=\"3.5\" fill=\"" # esc(color) # "\"/>");
      // left label (category + value), right value.
      b.add("<text class=\"mv-chart-tick mv-chart-slope-lbl\" x=\"" # fmtNum(xL - 8.0) # "\" y=\"" # fmtNum(yL + 4.0) # "\" text-anchor=\"end\">" # esc(lbl # " " # fmtNum(bv)) # "</text>");
      b.add("<text class=\"mv-chart-tick mv-chart-slope-lbl\" x=\"" # fmtNum(xR + 8.0) # "\" y=\"" # fmtNum(yR + 4.0) # "\" text-anchor=\"start\">" # esc(fmtNum(av)) # "</text>");
      b.add("</g>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse "label:before,after" rows separated by ';'.
  func slopeParse(spec : Text) : [(Text, Float, Float)] {
    let out = Buffer.Buffer<(Text, Float, Float)>(4);
    for (seg in Text.split(spec, #char ';')) {
      let s = Text.trimStart(Text.trimEnd(seg, #char ' '), #char ' ');
      if (s != "") {
        let parts = Iter.toArray(Text.split(s, #char ':'));
        if (parts.size() >= 2) {
          let lbl = Text.trimStart(Text.trimEnd(parts[0], #char ' '), #char ' ');
          let pair = parseFloats(parts[1]);
          if (pair.size() >= 2) { out.add((lbl, pair[0], pair[1])) };
        };
      };
    };
    Buffer.toArray(out);
  };

  // ===== DivergingBarChart =====
  // ---- DivergingBarChart: signed bars L/R of a centered zero ----------------
  /// `<DivergingBarChart values="-12,8,-5,20" labels="A,B,C,D" />`. Positive
  /// values grow right, negative left, from a centered zero baseline. Positive
  /// and negative bars use distinct palette colors. Native <title> per bar.
  public func divergingBar(valuesCsv : Text, labelsCsv : Text, opts : O) : Text {
    let vals = parseFloats(valuesCsv);
    let labels = parseLabels(labelsCsv);
    let n = vals.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-diverging") # svgClose() };

    // symmetric domain about zero so the axis is centered.
    let m = maxF(Float.abs(arrMin(vals)), Float.abs(arrMax(vals)));
    let hi = switch (opts.yMax) { case (?v) { v }; case null { niceCeil(if (m <= 0.0) { 1.0 } else { m }) } };
    let lo = -hi;
    let left = plotLeft();
    let right = plotRight(opts);
    let xSc = linScale(lo, hi, left, right);
    let top = plotTop();
    let bot = plotBottom(opts);
    let band = (bot - top) / Float.fromInt(n);
    let barH = band * 0.62;
    let zeroX = xSc(0.0);

    let cPos = palette(opts, 0);
    let cNeg = palette(opts, 2);

    let b = Buffer.Buffer<Text>(n * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-diverging"));
    // symmetric numeric x ticks + grid.
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
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(zeroX) # "\" y1=\"" # fmtNum(top)
        # "\" x2=\"" # fmtNum(zeroX) # "\" y2=\"" # fmtNum(bot) # "\"/>");
    };

    var i : Nat = 0;
    while (i < n) {
      let v = vals[i];
      let yc = top + band * Float.fromInt(i) + (band - barH) / 2.0;
      let xv = xSc(v);
      let x0 = minF(zeroX, xv);
      let w = Float.abs(xv - zeroX);
      let color = if (v < 0.0) { cNeg } else { cPos };
      let lbl = labelAt(labels, i);
      b.add("<g class=\"mv-chart-diverging-g\">");
      b.add("<rect class=\"mv-chart-diverging-rect\" x=\"" # fmtNum(x0) # "\" y=\"" # fmtNum(yc)
        # "\" width=\"" # fmtNum(w) # "\" height=\"" # fmtNum(barH) # "\" rx=\"2\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc(lbl # (if (lbl == "") { "" } else { ": " }) # fmtNum(v) # opts.unit) # "</title></rect>");
      // label on the opposite side of the bar from zero.
      let lblX = if (v < 0.0) { zeroX + 6.0 } else { zeroX - 6.0 };
      let anchor = if (v < 0.0) { "start" } else { "end" };
      b.add("<text class=\"mv-chart-tick mv-chart-diverging-lbl\" x=\"" # fmtNum(lblX)
        # "\" y=\"" # fmtNum(yc + barH / 2.0 + 4.0) # "\" text-anchor=\"" # anchor # "\">" # esc(lbl) # "</text>");
      b.add("</g>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== WaterfallChart =====
  // ---- WaterfallChart: running total of signed deltas ----------------------
  /// `<WaterfallChart deltas="120,-30,45,-20" labels="Start,Q1,Q2,Q3" />`. Each
  /// value is a delta applied to a running total; each floating column spans the
  /// previous cumulative to the new one. Rising vs falling steps use distinct
  /// colors; connectors link successive steps. Native <title> per step.
  public func waterfall(deltasCsv : Text, labelsCsv : Text, opts : O) : Text {
    let deltas = parseFloats(deltasCsv);
    let labels = parseLabels(labelsCsv);
    let n = deltas.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-waterfall") # svgClose() };

    // running cumulative; track min/max of the cumulative path (incl. 0).
    let starts = Array.init<Float>(n, 0.0);
    let ends = Array.init<Float>(n, 0.0);
    var acc : Float = 0.0;
    var loData : Float = 0.0;
    var hiData : Float = 0.0;
    var i0 : Nat = 0;
    while (i0 < n) {
      starts[i0] := acc;
      acc += deltas[i0];
      ends[i0] := acc;
      if (acc < loData) { loData := acc };
      if (acc > hiData) { hiData := acc };
      i0 += 1;
    };
    let lo = switch (opts.yMin) { case (?v) { v };
      case null { if (loData < 0.0) { -niceCeil(-loData) } else { 0.0 } } };
    let hi = switch (opts.yMax) { case (?v) { v }; case null { niceCeil(if (hiData <= 0.0) { 1.0 } else { hiData }) } };
    let hiU = if (hi <= lo) { lo + 1.0 } else { hi };
    let ySc = linScale(lo, hiU, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(n);
    let colW = band * 0.62;

    let cUp = palette(opts, 1);
    let cDown = palette(opts, 2);

    let b = Buffer.Buffer<Text>(n * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-waterfall"));
    b.add(axisLeft(opts, ySc, ticks(lo, hiU, 4)));
    let centers = Array.tabulate<Float>(n, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    var i : Nat = 0;
    while (i < n) {
      let d = deltas[i];
      let yStart = ySc(starts[i]);
      let yEnd = ySc(ends[i]);
      let y0 = minF(yStart, yEnd);
      let h = Float.abs(yEnd - yStart);
      let xc = centers[i] - colW / 2.0;
      let color = if (d < 0.0) { cDown } else { cUp };
      let lbl = labelAt(labels, i);
      // connector from previous step's end to this step's start.
      if (i > 0) {
        let yPrev = ySc(ends[i - 1]);
        b.add("<line class=\"mv-chart-waterfall-conn\" x1=\"" # fmtNum(centers[i - 1] + colW / 2.0)
          # "\" y1=\"" # fmtNum(yPrev) # "\" x2=\"" # fmtNum(centers[i] - colW / 2.0)
          # "\" y2=\"" # fmtNum(yPrev) # "\"/>");
      };
      let tip = lbl # ": " # (if (d >= 0.0) { "+" } else { "" }) # fmtNum(d) # opts.unit # " (\u{2192} " # fmtNum(ends[i]) # opts.unit # ")";
      b.add("<rect class=\"mv-chart-waterfall-rect\" x=\"" # fmtNum(xc) # "\" y=\"" # fmtNum(y0)
        # "\" width=\"" # fmtNum(colW) # "\" height=\"" # fmtNum(maxF(1.0, h)) # "\" rx=\"2\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc(tip) # "</title></rect>");
      i += 1;
    };
    b.add(legend(opts, [("Increase", cUp), ("Decrease", cDown)]));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== PictogramChart =====
  // ---- PictogramChart: value/total as a grid of repeated glyphs -------------
  /// `<PictogramChart value="37" total="50" />`. Draws `total` glyphs in a grid;
  /// the first `value` are filled (brand), the rest muted — an isotype/unit chart.
  /// `cols` sets glyphs per row (default 10). Native <title> shows value/total.
  public func pictogram(valueCsv : Text, totalCsv : Text, colsCsv : Text, opts : O) : Text {
    let value = pictoF(valueCsv);
    let total0 = pictoF(totalCsv);
    let total = if (total0 < 1.0) { 1.0 } else { total0 };
    let nTotal = Int.abs(Float.toInt(Float.floor(total + 0.5)));
    var nFilled = Int.abs(Float.toInt(Float.floor((if (value < 0.0) { 0.0 } else { value }) + 0.5)));
    if (nFilled > nTotal) { nFilled := nTotal };
    let cols0 = Int.abs(Float.toInt(Float.floor(pictoF(colsCsv) + 0.5)));
    let cols = if (cols0 < 1) { 10 } else { cols0 };

    let b = Buffer.Buffer<Text>(nTotal + 8);
    b.add(svgOpen(opts, "mv-chart-pictogram"));
    let left = plotLeft() - 40.0; // pictograms need less left padding
    let right = plotRight(opts);
    let top = plotTop();
    let bot = Float.fromInt(Int.abs(opts.height)) - (if (opts.showLegend) { 26.0 } else { 12.0 });
    let rows = (nTotal + cols - 1) / cols;
    let availW = right - left;
    let availH = bot - top;
    let cellW = availW / Float.fromInt(cols);
    let cellH = if (rows == 0) { availH } else { availH / Float.fromInt(rows) };
    let cell = minF(cellW, cellH);
    let r = cell * 0.36;

    let cOn = palette(opts, 0);
    let tip = fmtNum(value) # " of " # fmtNum(total) # opts.unit;
    var k : Nat = 0;
    while (k < nTotal) {
      let row = k / cols;
      let col = k % cols;
      let cx = left + cellW * (Float.fromInt(col) + 0.5);
      let cy = top + cellH * (Float.fromInt(row) + 0.5);
      let on = k < nFilled;
      let cls = if (on) { "mv-chart-picto-on" } else { "mv-chart-picto-off" };
      let fill = if (on) { esc(cOn) } else { "var(--colorNeutralBackground5, #d6d6d6)" };
      b.add("<circle class=\"" # cls # "\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"" # fmtNum(r) # "\" fill=\"" # fill # "\">"
        # (if (k == 0) { "<title>" # esc(tip) # "</title>" } else { "" }) # "</circle>");
      k += 1;
    };
    // summary label.
    let pct = value / total * 100.0;
    b.add(legend(opts, [(fmtNum(value) # "/" # fmtNum(total) # " (" # fmtNum(pct) # "%)", cOn)]));
    b.add(svgClose());
    Text.join("", b.vals());
  };
  func pictoF(t : Text) : Float {
    let fs = parseFloats(t);
    if (fs.size() > 0) { fs[0] } else { 0.0 };
  };

  // ===== WaffleChart =====
  // ---- WaffleChart: 10x10 grid of squares colored by category share ---------
  /// `<WaffleChart values="42,30,18,10" labels="Direct,Search,Social,Mail" />`.
  /// Values are normalized to 100 cells; each category fills a whole number of
  /// cells (largest-remainder rounding so the 100 cells are fully allocated).
  /// Cells fill column-major from bottom-left so categories stack visually.
  public func waffle(values : Text, labels : Text, opts : O) : Text {
    let vs = parseFloats(values);
    let labs = parseLabels(labels);
    let b = Buffer.Buffer<Text>(110);
    b.add(svgOpen(opts, "mv-chart-waffle"));
    let total = sum(vs);
    let (cx0, cy0) = centerXY(opts);
    if (total <= 0.0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx0) # "\" y=\"" # fmtNum(cy0) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    // largest-remainder allocation of 100 cells across categories
    let n = vs.size();
    let cells = Array.init<Nat>(n, 0);
    let rema = Array.init<Float>(n, 0.0);
    var allocated : Nat = 0;
    var i : Nat = 0;
    while (i < n) {
      let exact = (if (vs[i] > 0.0) { vs[i] } else { 0.0 }) / total * 100.0;
      let fl = Float.floor(exact);
      cells[i] := Int.abs(Float.toInt(fl));
      rema[i] := exact - fl;
      allocated += cells[i];
      i += 1;
    };
    // distribute the leftover cells to the largest remainders
    var leftover : Int = 100 - allocated;
    while (leftover > 0) {
      var best : Nat = 0; var bestVal : Float = -1.0; var found = false;
      var k : Nat = 0;
      while (k < n) {
        if (rema[k] > bestVal) { bestVal := rema[k]; best := k; found := true };
        k += 1;
      };
      if (not found) { leftover := 0 } else {
        cells[best] += 1;
        rema[best] := -1.0; // don't pick the same one twice in a row pass
        leftover -= 1;
        // if we still have leftover but exhausted all, reset remainders to spread
        if (leftover > 0) {
          var allNeg = true;
          var m : Nat = 0;
          while (m < n) { if (rema[m] >= 0.0) { allNeg := false }; m += 1 };
          if (allNeg) {
            var p : Nat = 0;
            while (p < n) { rema[p] := if (vs[p] > 0.0) { vs[p] } else { 0.0 }; p += 1 };
          };
        };
      };
    };
    // geometry: a square grid centered in the plot area
    let plotW = plotRight(opts) - plotLeft();
    let topY = plotTop();
    let botY = if (opts.showLegend) { plotBottom(opts) + 18.0 } else { plotBottom(opts) };
    let avail = minF(plotW, botY - topY);
    let side = if (avail < 10.0) { 10.0 } else { avail };
    let gx0 = plotLeft() + (plotW - side) / 2.0;
    let gy0 = topY + ((botY - topY) - side) / 2.0;
    let cellSize = side / 10.0;
    let pad = cellSize * 0.12;
    // build a 100-length category index array (cat per cell), -1 = none
    let catOf = Array.init<Int>(100, -1);
    var ci : Nat = 0; var cursor : Nat = 0;
    while (ci < n) {
      var c2 : Nat = 0;
      while (c2 < cells[ci] and cursor < 100) { catOf[cursor] := ci; cursor += 1; c2 += 1 };
      ci += 1;
    };
    let emptyFill = "#e6e6e6";
    b.add("<g class=\"mv-chart-waffle-cells\">");
    var idx : Nat = 0;
    while (idx < 100) {
      // column-major from bottom-left: col = idx/10, row from bottom
      let col = idx / 10;
      let rowFromBottom = idx % 10;
      let x = gx0 + Float.fromInt(col) * cellSize + pad;
      let y = gy0 + Float.fromInt(9 - rowFromBottom) * cellSize + pad;
      let cat = catOf[idx];
      let fill = if (cat >= 0) { palette(opts, Int.abs(cat)) } else { emptyFill };
      let cls = if (cat >= 0) { "mv-chart-waffle-cell" } else { "mv-chart-waffle-cell mv-chart-waffle-empty" };
      let tip = if (cat >= 0) {
        let ix = Int.abs(cat);
        let nm = labelAt(labs, ix);
        let pct = vs[ix] / total * 100.0;
        (if (nm != "") { nm # ": " } else { "" }) # fmtNum(vs[ix]) # opts.unit # " (" # fmtNum(pct) # "%)";
      } else { "" };
      b.add("<rect class=\"" # cls # "\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(y)
        # "\" width=\"" # fmtNum(cellSize - pad * 2.0) # "\" height=\"" # fmtNum(cellSize - pad * 2.0)
        # "\" rx=\"2\" fill=\"" # esc(fill) # "\">"
        # (if (tip != "") { "<title>" # esc(tip) # "</title>" } else { "" }) # "</rect>");
      idx += 1;
    };
    b.add("</g>");
    let legendEntries = Buffer.Buffer<(Text, Text)>(n);
    var li : Nat = 0;
    while (li < n) {
      if (vs[li] > 0.0) {
        let nm = labelAt(labs, li);
        legendEntries.add((if (nm != "") { nm } else { fmtNum(vs[li] / total * 100.0) # "%" }, palette(opts, li)));
      };
      li += 1;
    };
    b.add(legend(opts, Buffer.toArray(legendEntries)));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== TreemapChart =====
  // ---- TreemapChart: squarified rectangles, flat "label:value" list ---------
  /// `<TreemapChart data="Search:42;Direct:30;Social:18;Email:10" />`.
  /// Areas are proportional to value; the squarify algorithm keeps rectangles
  /// close to square for readability. Each tile carries a <title> tooltip and an
  /// inline label when it is big enough.
  public func treemap(data : Text, opts : O) : Text {
    // parse "label:value" pairs (reuse the series parser: one value each)
    let pairs = parseSeries(data);
    let items = Buffer.Buffer<(Text, Float)>(pairs.size());
    var totalT : Float = 0.0;
    for ((nm, vs) in pairs.vals()) {
      let v = if (vs.size() > 0 and vs[0] > 0.0) { vs[0] } else { 0.0 };
      if (v > 0.0) { items.add((nm, v)); totalT += v };
    };
    let b = Buffer.Buffer<Text>(items.size() * 2 + 6);
    b.add(svgOpen(opts, "mv-chart-treemap"));
    let x0 = plotLeft();
    let y0 = plotTop();
    let x1 = plotRight(opts);
    let y1 = if (opts.showLegend) { plotBottom(opts) + 18.0 } else { plotBottom(opts) };
    if (totalT <= 0.0 or items.size() == 0) {
      let (cx, cy) = centerXY(opts);
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    let arr = Buffer.toArray(items);
    // sort descending by value (simple insertion sort; small N)
    let order = Array.init<Nat>(arr.size(), 0);
    var z : Nat = 0; while (z < arr.size()) { order[z] := z; z += 1 };
    var a0 : Nat = 1;
    while (a0 < arr.size()) {
      let key = order[a0];
      var jj : Int = a0 - 1;
      while (jj >= 0 and arr[order[Int.abs(jj)]].1 < arr[key].1) {
        order[Int.abs(jj) + 1] := order[Int.abs(jj)];
        jj -= 1;
      };
      order[Int.abs(jj) + 1] := key;
      a0 += 1;
    };
    let totalArea = (x1 - x0) * (y1 - y0);
    // scaled "area" per item in px^2
    let areas = Array.tabulate<Float>(arr.size(), func(k) { arr[order[k]].1 / totalT * totalArea });
    // squarify into the rect [x0,y0,x1,y1]
    var rx = x0; var ry = y0; var rw = x1 - x0; var rh = y1 - y0;
    var startI : Nat = 0;
    let outRects = Buffer.Buffer<(Float, Float, Float, Float, Nat)>(arr.size());
    func treemapWorst(rowSum : Float, rowMin : Float, rowMax : Float, sideLen : Float) : Float {
      if (rowSum <= 0.0 or sideLen <= 0.0) { return 1.0e18 };
      let s2 = rowSum * rowSum;
      let side2 = sideLen * sideLen;
      maxF(side2 * rowMax / s2, s2 / (side2 * rowMin));
    };
    while (startI < areas.size()) {
      let shortSide = minF(rw, rh);
      // grow a row while it improves the aspect ratio
      var rowSum : Float = 0.0;
      var rowMin : Float = 1.0e18;
      var rowMax : Float = 0.0;
      var endI = startI;
      var worst : Float = 1.0e18;
      var keepGoing = true;
      while (endI < areas.size() and keepGoing) {
        let a = areas[endI];
        let nSum = rowSum + a;
        let nMin = minF(rowMin, a);
        let nMax = maxF(rowMax, a);
        let nWorst = treemapWorst(nSum, nMin, nMax, shortSide);
        if (endI == startI or nWorst <= worst) {
          rowSum := nSum; rowMin := nMin; rowMax := nMax; worst := nWorst;
          endI += 1;
        } else { keepGoing := false };
      };
      // lay out the row [startI, endI) along the short side
      let count = endI - startI;
      if (rw >= rh) {
        // row is a vertical strip of width = rowSum/rh
        let stripW = if (rh > 0.0) { rowSum / rh } else { rw };
        var cy = ry;
        var k = startI;
        while (k < endI) {
          let cellH = if (rowSum > 0.0) { areas[k] / rowSum * rh } else { rh / Float.fromInt(count) };
          outRects.add((rx, cy, stripW, cellH, order[k]));
          cy += cellH;
          k += 1;
        };
        rx += stripW; rw -= stripW;
      } else {
        // row is a horizontal strip of height = rowSum/rw
        let stripH = if (rw > 0.0) { rowSum / rw } else { rh };
        var cxx = rx;
        var k = startI;
        while (k < endI) {
          let cellW = if (rowSum > 0.0) { areas[k] / rowSum * rw } else { rw / Float.fromInt(count) };
          outRects.add((cxx, ry, cellW, stripH, order[k]));
          cxx += cellW;
          k += 1;
        };
        ry += stripH; rh -= stripH;
      };
      startI := endI;
    };
    b.add("<g class=\"mv-chart-treemap-tiles\">");
    let gap : Float = 1.5;
    for ((tx, ty, tw, th, oi) in outRects.vals()) {
      let (nm, v) = arr[oi];
      let color = palette(opts, oi);
      let pct = v / totalT * 100.0;
      let tip = (if (nm != "") { nm # ": " } else { "" }) # fmtNum(v) # opts.unit # " (" # fmtNum(pct) # "%)";
      let iw = maxF(tw - gap, 0.0);
      let ih = maxF(th - gap, 0.0);
      b.add("<g class=\"mv-chart-treemap-tile\">");
      b.add("<rect x=\"" # fmtNum(tx) # "\" y=\"" # fmtNum(ty)
        # "\" width=\"" # fmtNum(iw) # "\" height=\"" # fmtNum(ih)
        # "\" rx=\"2\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></rect>");
      if (iw > 44.0 and ih > 24.0 and nm != "") {
        b.add("<text class=\"mv-chart-treemap-label\" x=\"" # fmtNum(tx + 6.0) # "\" y=\"" # fmtNum(ty + 16.0) # "\">" # esc(nm) # "</text>");
        b.add("<text class=\"mv-chart-treemap-val\" x=\"" # fmtNum(tx + 6.0) # "\" y=\"" # fmtNum(ty + 30.0) # "\">" # esc(fmtNum(v) # opts.unit) # "</text>");
      };
      b.add("</g>");
    };
    b.add("</g>");
    let legendEntries = Array.tabulate<(Text, Text)>(arr.size(), func(k) {
      let (nm, _) = arr[k]; (if (nm != "") { nm } else { "Item " # Nat.toText(k + 1) }, palette(opts, k));
    });
    b.add(legend(opts, legendEntries));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== FunnelChart =====
  // ---- FunnelChart: stacked descending centered trapezoids ------------------
  /// `<FunnelChart values="1000,720,430,180" labels="Visits,Signups,Trials,Paid" />`.
  /// Each stage is a horizontal band whose width is proportional to its value
  /// relative to the FIRST (largest) stage; bands taper toward the bottom.
  public func funnel(values : Text, labels : Text, opts : O) : Text {
    funnelLike(parseFloats(values), parseLabels(labels), opts, "mv-chart-funnel", false);
  };

  // Shared trapezoid stack renderer for funnel/pyramid.
  // ascending=false -> funnel (widest at top); ascending=true -> pyramid.
  func funnelLike(vs : [Float], labs : [Text], opts : O, modClass : Text, ascending : Bool) : Text {
    let b = Buffer.Buffer<Text>(vs.size() * 2 + 6);
    b.add(svgOpen(opts, modClass));
    let n = vs.size();
    let cx = Float.fromInt(Int.abs(opts.width)) / 2.0;
    let (ccx, ccy) = centerXY(opts);
    if (n == 0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ccx) # "\" y=\"" # fmtNum(ccy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    // max value drives the widest band
    var vmax : Float = 0.0;
    var t : Nat = 0; while (t < n) { if (vs[t] > vmax) { vmax := vs[t] }; t += 1 };
    if (vmax <= 0.0) { vmax := 1.0 };
    let topY = plotTop();
    let botY = if (opts.showLegend) { plotBottom(opts) + 6.0 } else { plotBottom(opts) + 14.0 };
    let plotH = botY - topY;
    let band = plotH / Float.fromInt(n);
    let maxW = (plotRight(opts) - plotLeft()) * 0.92;
    let gap = band * 0.12;
    // width fraction at a given stage value
    func wAt(v : Float) : Float { (if (v > 0.0) { v } else { 0.0 }) / vmax * maxW };
    b.add("<g class=\"mv-chart-funnel-bands\">");
    var i : Nat = 0;
    while (i < n) {
      // index into the values for top/bottom widths of this band
      let v = vs[i];
      // for funnel: top width = this value, bottom width = next value (taper)
      // for pyramid: we reverse the drawing order so the widest is at the bottom
      let drawIdx = if (ascending) { n - 1 - i } else { i };
      let vTop = vs[drawIdx];
      let vBot = if (ascending) {
        if (drawIdx > 0) { vs[drawIdx - 1] } else { vs[drawIdx] }
      } else {
        if (drawIdx + 1 < n) { vs[drawIdx + 1] } else { vs[drawIdx] }
      };
      let wTop = wAt(vTop);
      let wBot = wAt(vBot);
      let yT = topY + band * Float.fromInt(i) + gap / 2.0;
      let yB = topY + band * Float.fromInt(i + 1) - gap / 2.0;
      let xTL = cx - wTop / 2.0; let xTR = cx + wTop / 2.0;
      let xBL = cx - wBot / 2.0; let xBR = cx + wBot / 2.0;
      let color = palette(opts, drawIdx);
      let nm = labelAt(labs, drawIdx);
      let pct = vTop / vmax * 100.0;
      let tip = (if (nm != "") { nm # ": " } else { "" }) # fmtNum(vTop) # opts.unit # " (" # fmtNum(pct) # "%)";
      let poly = fmtNum(xTL) # "," # fmtNum(yT) # " " # fmtNum(xTR) # "," # fmtNum(yT)
        # " " # fmtNum(xBR) # "," # fmtNum(yB) # " " # fmtNum(xBL) # "," # fmtNum(yB);
      b.add("<g class=\"mv-chart-funnel-band\">");
      b.add("<polygon points=\"" # poly # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></polygon>");
      // centered value label
      b.add("<text class=\"mv-chart-funnel-val\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum((yT + yB) / 2.0 + 4.0) # "\" text-anchor=\"middle\">" # esc(fmtNum(vTop) # opts.unit) # "</text>");
      // stage label on the left margin
      if (nm != "") {
        b.add("<text class=\"mv-chart-funnel-label\" x=\"" # fmtNum(plotLeft() - 6.0) # "\" y=\"" # fmtNum((yT + yB) / 2.0 + 4.0) # "\" text-anchor=\"end\">" # esc(nm) # "</text>");
      };
      b.add("</g>");
      i += 1;
    };
    b.add("</g>");
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== PyramidChart =====
  // ---- PyramidChart: ascending centered trapezoids (widest at bottom) -------
  /// `<PyramidChart values="180,430,720,1000" labels="Paid,Trials,Signups,Visits" />`.
  /// A funnel flipped: stages widen toward the base. Order your values from the
  /// apex (top, smallest) down to the base (bottom, largest).
  /// (Uses the shared `funnelLike` helper defined with FunnelChart.)
  public func pyramid(values : Text, labels : Text, opts : O) : Text {
    funnelLike(parseFloats(values), parseLabels(labels), opts, "mv-chart-pyramid", true);
  };

  // ===== MarimekkoChart =====
  // ---- MarimekkoChart: variable-width 100% stacked columns ------------------
  /// `<MarimekkoChart series="A:30,20,10;B:10,25,15;C:5,10,20"
  ///                  labels="A,B,C" segments="Low,Mid,High" />`.
  /// Each named series becomes ONE column; the column WIDTH is proportional to
  /// the series total, and within it the values are stacked to 100% height.
  /// `labels` (optional) override the column names; `segments` names the stacked
  /// parts for the legend + tooltips.
  public func marimekko(seriesSpec : Text, labels : Text, segments : Text, opts : O) : Text {
    let series = parseSeries(seriesSpec);
    let colLabs = parseLabels(labels);
    let segLabs = parseLabels(segments);
    let ns = series.size();
    let b = Buffer.Buffer<Text>(ns * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-marimekko"));
    let (ccx, ccy) = centerXY(opts);
    if (ns == 0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ccx) # "\" y=\"" # fmtNum(ccy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    // column totals (only positive values count)
    let colTotals = Array.tabulate<Float>(ns, func(s) {
      var t : Float = 0.0; for (v in series[s].1.vals()) { if (v > 0.0) { t += v } }; t;
    });
    var grand : Float = 0.0; for (t in colTotals.vals()) { grand += t };
    if (grand <= 0.0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ccx) # "\" y=\"" # fmtNum(ccy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    // number of stacked segments = max value count across series
    var nseg : Nat = 0;
    for ((_, vs) in series.vals()) { if (vs.size() > nseg) { nseg := vs.size() } };
    let left = plotLeft();
    let right = plotRight(opts);
    let top = plotTop();
    let bot = plotBottom(opts);
    let plotW = right - left;
    let plotH = bot - top;
    let colGap : Float = 3.0;
    let totalGap = colGap * Float.fromInt(if (ns > 0) { ns - 1 } else { 0 });
    let usableW = maxF(plotW - totalGap, 1.0);
    // bottom axis baseline
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(left) # "\" y1=\"" # fmtNum(bot)
        # "\" x2=\"" # fmtNum(right) # "\" y2=\"" # fmtNum(bot) # "\"/>");
    };
    b.add("<g class=\"mv-chart-marimekko-cols\">");
    var x = left;
    var si : Nat = 0;
    while (si < ns) {
      let (sname, vs) = series[si];
      let colW = colTotals[si] / grand * usableW;
      let colName = if (si < colLabs.size() and colLabs[si] != "") { colLabs[si] } else { sname };
      // stack segments 100% within the column
      var acc : Float = 0.0;
      let ctot = if (colTotals[si] > 0.0) { colTotals[si] } else { 1.0 };
      var gi : Nat = 0;
      while (gi < nseg) {
        let v = if (gi < vs.size() and vs[gi] > 0.0) { vs[gi] } else { 0.0 };
        if (v > 0.0) {
          let frac0 = acc / ctot;
          acc += v;
          let frac1 = acc / ctot;
          let yTop = bot - frac1 * plotH;
          let yBot = bot - frac0 * plotH;
          let segName = if (gi < segLabs.size()) { segLabs[gi] } else { "Segment " # Nat.toText(gi + 1) };
          let color = palette(opts, gi);
          let pct = v / ctot * 100.0;
          let tip = colName # " \u{b7} " # segName # ": " # fmtNum(v) # opts.unit # " (" # fmtNum(pct) # "%)";
          b.add("<rect class=\"mv-chart-marimekko-cell\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(yTop)
            # "\" width=\"" # fmtNum(maxF(colW, 0.0)) # "\" height=\"" # fmtNum(maxF(yBot - yTop, 0.0))
            # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></rect>");
        };
        gi += 1;
      };
      // column label (centered under the column) + width %
      let cxc = x + colW / 2.0;
      b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(cxc) # "\" y=\"" # fmtNum(bot + 16.0) # "\" text-anchor=\"middle\">" # esc(colName) # "</text>");
      b.add("<text class=\"mv-chart-marimekko-wpct\" x=\"" # fmtNum(cxc) # "\" y=\"" # fmtNum(top - 4.0) # "\" text-anchor=\"middle\">" # esc(fmtNum(colTotals[si] / grand * 100.0) # "%") # "</text>");
      x += colW + colGap;
      si += 1;
    };
    b.add("</g>");
    // legend names the stacked segments
    let legendEntries = Array.tabulate<(Text, Text)>(nseg, func(g) {
      ((if (g < segLabs.size()) { segLabs[g] } else { "Segment " # Nat.toText(g + 1) }), palette(opts, g));
    });
    b.add(legend(opts, legendEntries));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== PopulationPyramid =====
  // ---- PopulationPyramid: two opposing horizontal bar series by age band ----
  /// `<PopulationPyramid left="Male:5,8,12,10,7;Female:4,7,11,12,9"
  ///                     labels="0-14,15-29,30-44,45-59,60+" />`.
  /// The `left` prop holds exactly two named series; the first is drawn to the
  /// LEFT of a central axis, the second to the RIGHT. One band per age group.
  public func populationPyramid(pairSpec : Text, labels : Text, opts : O) : Text {
    let series = parseSeries(pairSpec);
    let bands = parseLabels(labels);
    let b = Buffer.Buffer<Text>(32);
    b.add(svgOpen(opts, "mv-chart-poppyramid"));
    let (ccx, ccy) = centerXY(opts);
    if (series.size() < 1) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ccx) # "\" y=\"" # fmtNum(ccy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    let (lname, lvals) = series[0];
    let (rname, rvals) = if (series.size() >= 2) { series[1] } else { ("", []) };
    // number of age bands
    var nb : Nat = lvals.size();
    if (rvals.size() > nb) { nb := rvals.size() };
    if (bands.size() > nb) { nb := bands.size() };
    if (nb == 0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ccx) # "\" y=\"" # fmtNum(ccy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    // shared magnitude domain so both sides use the same scale
    var dmax : Float = 0.0;
    for (v in lvals.vals()) { if (v > dmax) { dmax := v } };
    for (v in rvals.vals()) { if (v > dmax) { dmax := v } };
    let hi = switch (opts.yMax) { case (?m) { m }; case null { niceCeil(if (dmax <= 0.0) { 1.0 } else { dmax }) } };
    let denom = if (hi <= 0.0) { 1.0 } else { hi };
    let left = plotLeft();
    let right = plotRight(opts);
    let top = plotTop();
    let bot = plotBottom(opts);
    // central gutter holds the age-band labels
    let gutter : Float = 44.0;
    let cx = (left + right) / 2.0;
    let halfW = (right - left - gutter) / 2.0;
    let leftAxisX = cx - gutter / 2.0;
    let rightAxisX = cx + gutter / 2.0;
    let band = (bot - top) / Float.fromInt(nb);
    let barH = band * 0.7;
    let lcolor = palette(opts, 0);
    let rcolor = palette(opts, 1);
    // central axes
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(leftAxisX) # "\" y1=\"" # fmtNum(top) # "\" x2=\"" # fmtNum(leftAxisX) # "\" y2=\"" # fmtNum(bot) # "\"/>");
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(rightAxisX) # "\" y1=\"" # fmtNum(top) # "\" x2=\"" # fmtNum(rightAxisX) # "\" y2=\"" # fmtNum(bot) # "\"/>");
    };
    b.add("<g class=\"mv-chart-poppyramid-bars\">");
    var i : Nat = 0;
    while (i < nb) {
      let yc = top + band * Float.fromInt(i) + (band - barH) / 2.0;
      let bl = if (i < bands.size()) { bands[i] } else { "" };
      // left bar grows leftward from leftAxisX
      let lv = if (i < lvals.size()) { lvals[i] } else { 0.0 };
      let lw = (if (lv > 0.0) { lv } else { 0.0 }) / denom * halfW;
      let ltip = (if (lname != "") { lname # " " } else { "" }) # bl # ": " # fmtNum(lv) # opts.unit;
      b.add("<rect class=\"mv-chart-poppyramid-l\" x=\"" # fmtNum(leftAxisX - lw) # "\" y=\"" # fmtNum(yc)
        # "\" width=\"" # fmtNum(lw) # "\" height=\"" # fmtNum(barH)
        # "\" fill=\"" # esc(lcolor) # "\"><title>" # esc(ltip) # "</title></rect>");
      // right bar grows rightward from rightAxisX
      let rv = if (i < rvals.size()) { rvals[i] } else { 0.0 };
      let rw = (if (rv > 0.0) { rv } else { 0.0 }) / denom * halfW;
      let rtip = (if (rname != "") { rname # " " } else { "" }) # bl # ": " # fmtNum(rv) # opts.unit;
      b.add("<rect class=\"mv-chart-poppyramid-r\" x=\"" # fmtNum(rightAxisX) # "\" y=\"" # fmtNum(yc)
        # "\" width=\"" # fmtNum(rw) # "\" height=\"" # fmtNum(barH)
        # "\" fill=\"" # esc(rcolor) # "\"><title>" # esc(rtip) # "</title></rect>");
      // age-band label centered in the gutter
      if (bl != "") {
        b.add("<text class=\"mv-chart-tick mv-chart-poppyramid-band\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(yc + barH / 2.0 + 4.0) # "\" text-anchor=\"middle\">" # esc(bl) # "</text>");
      };
      i += 1;
    };
    b.add("</g>");
    let entries = Buffer.Buffer<(Text, Text)>(2);
    entries.add((if (lname != "") { lname } else { "Left" }, lcolor));
    if (series.size() >= 2) { entries.add((if (rname != "") { rname } else { "Right" }, rcolor)) };
    b.add(legend(opts, Buffer.toArray(entries)));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== SunburstChart =====
  // ---- SunburstChart: concentric rings from "A/B:value" paths (1-2 levels) --
  /// `<SunburstChart paths="Tech/Web:30;Tech/Mobile:20;Sales/Inbound:25;Sales/Outbound:15;Ops:10" />`.
  /// Each path is "level1/level2:value" (level2 optional). Inner ring = level1
  /// groups (summed children); outer ring = the level2 children, aligned under
  /// their parent's angular span. A leaf with no child fills the whole radius.
  public func sunburst(pathsSpec : Text, opts : O) : Text {
    // parse "a/b:value" entries
    let parents = Buffer.Buffer<Text>(8);
    let pSums = Buffer.Buffer<Float>(8);
    // children stored parallel: childParentIdx[i], childName[i], childVal[i]
    let cParent = Buffer.Buffer<Nat>(16);
    let cName = Buffer.Buffer<Text>(16);
    let cVal = Buffer.Buffer<Float>(16);
    func sunburstFindParent(nm : Text) : ?Nat {
      var i : Nat = 0;
      while (i < parents.size()) { if (parents.get(i) == nm) { return ?i }; i += 1 };
      null;
    };
    var grand : Float = 0.0;
    for (seg in Text.split(pathsSpec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        // split "path:value" on the LAST ':'
        let cParts = Iter.toArray(Text.split(s, #char ':'));
        if (cParts.size() >= 2) {
          // rejoin everything before the last ':' as the path
          var pathPart = cParts[0];
          var k : Nat = 1;
          while (k + 1 < cParts.size()) { pathPart := pathPart # ":" # cParts[k]; k += 1 };
          let valTok = cParts[cParts.size() - 1];
          let v = switch (toFloatPub(valTok)) { case (?f) { f }; case null { 0.0 } };
          if (v > 0.0) {
            let lvls = Iter.toArray(Text.split(trim(pathPart), #char '/'));
            let p0 = trim(lvls[0]);
            let pIdx = switch (sunburstFindParent(p0)) {
              case (?ix) { pSums.put(ix, pSums.get(ix) + v); ix };
              case null { parents.add(p0); pSums.add(v); parents.size() - 1 };
            };
            if (lvls.size() >= 2 and trim(lvls[1]) != "") {
              cParent.add(pIdx); cName.add(trim(lvls[1])); cVal.add(v);
            } else {
              // leaf: a child equal to the parent (fills outer ring too)
              cParent.add(pIdx); cName.add(p0); cVal.add(v);
            };
            grand += v;
          };
        };
      };
    };
    let b = Buffer.Buffer<Text>(parents.size() * 2 + cParent.size() + 6);
    b.add(svgOpen(opts, "mv-chart-sunburst"));
    let (cx, cy) = centerXY(opts);
    let maxR = minF(cx, cy) - 8.0;
    let outerR = if (maxR < 12.0) { 12.0 } else { maxR };
    if (grand <= 0.0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    let innerR = outerR * 0.32; // hollow center
    let midR = innerR + (outerR - innerR) * 0.5;
    b.add("<g class=\"mv-chart-sunburst-rings\">");
    // inner ring: parents
    var acc : Float = 0.0;
    var pi : Nat = 0;
    let legendEntries = Buffer.Buffer<(Text, Text)>(parents.size());
    while (pi < parents.size()) {
      let pv = pSums.get(pi);
      let startFrac = acc / grand;
      let endFrac = (acc + pv) / grand;
      let color = palette(opts, pi);
      let nm = parents.get(pi);
      let pct = pv / grand * 100.0;
      let tip = nm # ": " # fmtNum(pv) # opts.unit # " (" # fmtNum(pct) # "%)";
      b.add("<path class=\"mv-chart-sunburst-arc mv-chart-sunburst-inner\" fill=\"" # esc(color) # "\" d=\"" # arcPath(cx, cy, midR, innerR, startFrac, endFrac) # "\"><title>" # esc(tip) # "</title></path>");
      legendEntries.add((nm, color));
      // outer ring: this parent's children, within [startFrac, endFrac]
      var cAcc : Float = 0.0;
      var shade : Nat = 0;
      var ciIdx : Nat = 0;
      while (ciIdx < cParent.size()) {
        if (cParent.get(ciIdx) == pi) {
          let cv = cVal.get(ciIdx);
          let cStart = startFrac + (cAcc / grand);
          cAcc += cv;
          let cEnd = startFrac + (cAcc / grand);
          let cnm = cName.get(ciIdx);
          let ctip = nm # " / " # cnm # ": " # fmtNum(cv) # opts.unit;
          b.add("<path class=\"mv-chart-sunburst-arc mv-chart-sunburst-outer\" fill=\"" # esc(palette(opts, pi)) # "\" fill-opacity=\"" # (if (shade % 2 == 0) { "0.75" } else { "0.5" }) # "\" d=\"" # arcPath(cx, cy, outerR, midR, cStart, cEnd) # "\"><title>" # esc(ctip) # "</title></path>");
          shade += 1;
        };
        ciIdx += 1;
      };
      acc += pv;
      pi += 1;
    };
    b.add("</g>");
    b.add(legend(opts, Buffer.toArray(legendEntries)));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // toFloat is private in the foundation; expose a tiny public bridge for the
  // sunburst path parser (namespaced so it won't collide). Include ONCE.
  public func toFloatPub(t : Text) : ?Float { toFloat(t) };

  // ===== Histogram =====
  // ---- shared distribution helpers (added ONCE for the whole family) -------
  // NOTE: these dist* helpers (distSort, distQuantile, distMean, distStdDev,
  // distGauss, distBandwidth, distKde, distPeak, distRangeOf, distSmooth,
  // distJitter) are shared by EVERY chart in the Distribution family. Add them
  // ONCE alongside Histogram; the other six entries reuse them.

  /// Ascending insertion sort of a Float array (datasets here are small).
  func distSort(xs : [Float]) : [Float] {
    let a = Array.thaw<Float>(xs);
    let n = a.size();
    var i : Nat = 1;
    while (i < n) {
      let key = a[i];
      var j : Int = i - 1;
      while (j >= 0 and a[Int.abs(j)] > key) {
        a[Int.abs(j) + 1] := a[Int.abs(j)];
        j -= 1;
      };
      a[Int.abs(j) + 1] := key;
      i += 1;
    };
    Array.freeze(a);
  };

  /// Linear-interpolated quantile (q in 0..1) of an ASCENDING-sorted array.
  func distQuantile(sorted : [Float], q : Float) : Float {
    let n = sorted.size();
    if (n == 0) { return 0.0 };
    if (n == 1) { return sorted[0] };
    let pos = q * Float.fromInt(n - 1);
    let lo = Float.floor(pos);
    let loi = Int.abs(Float.toInt(lo));
    let frac = pos - lo;
    if (loi + 1 >= n) { return sorted[n - 1] };
    sorted[loi] + (sorted[loi + 1] - sorted[loi]) * frac;
  };

  /// Arithmetic mean of an array (0 for empty).
  func distMean(xs : [Float]) : Float {
    let n = xs.size();
    if (n == 0) { return 0.0 };
    var s : Float = 0.0;
    for (x in xs.vals()) { s += x };
    s / Float.fromInt(n);
  };

  /// Population standard deviation (0 for < 2 points).
  func distStdDev(xs : [Float], mean : Float) : Float {
    let n = xs.size();
    if (n < 2) { return 0.0 };
    var s : Float = 0.0;
    for (x in xs.vals()) { let d = x - mean; s += d * d };
    sqrt_(s / Float.fromInt(n));
  };

  /// Gaussian kernel exp(-0.5 u^2) (constant factor omitted; density is
  /// peak-normalised by callers so the factor cancels).
  func distGauss(u : Float) : Float { Float.exp(-0.5 * u * u) };

  /// Silverman's rule-of-thumb 1-D KDE bandwidth (guards tiny samples).
  func distBandwidth(xs : [Float], sorted : [Float]) : Float {
    let n = xs.size();
    if (n < 2) { return 1.0 };
    let m = distMean(xs);
    let sd = distStdDev(xs, m);
    let iqr = distQuantile(sorted, 0.75) - distQuantile(sorted, 0.25);
    let iqrScaled = iqr / 1.349;
    var spread = sd;
    if (iqrScaled > 0.0 and iqrScaled < spread) { spread := iqrScaled };
    if (spread <= 0.0) { spread := if (sd > 0.0) { sd } else { 1.0 } };
    let bw = 0.9 * spread * Float.pow(Float.fromInt(n), -0.2);
    if (bw <= 0.0) { 1.0 } else { bw };
  };

  /// KDE at steps+1 evenly spaced x in [lo,hi]; returns [(x,density)] (NOT
  /// normalised — callers normalise to peak).
  func distKde(xs : [Float], bw : Float, lo : Float, hi : Float, steps : Nat) : [(Float, Float)] {
    let out = Buffer.Buffer<(Float, Float)>(steps + 1);
    let n = xs.size();
    if (n == 0 or steps == 0) { return [] };
    let span = if (hi - lo == 0.0) { 1.0 } else { hi - lo };
    let invN = 1.0 / (Float.fromInt(n) * bw);
    var i : Nat = 0;
    while (i <= steps) {
      let x = lo + span * Float.fromInt(i) / Float.fromInt(steps);
      var d : Float = 0.0;
      for (xv in xs.vals()) { d += distGauss((x - xv) / bw) };
      out.add((x, d * invN));
      i += 1;
    };
    Buffer.toArray(out);
  };

  /// Peak (max y) of a [(x,y)] density array (1.0 floor to avoid /0).
  func distPeak(d : [(Float, Float)]) : Float {
    var m : Float = 0.0;
    for ((_, y) in d.vals()) { if (y > m) { m := y } };
    if (m <= 0.0) { 1.0 } else { m };
  };

  /// Overall [min,max] across a list of groups, with small symmetric padding.
  func distRangeOf(groups : [[Float]]) : (Float, Float) {
    var lo : Float = 0.0; var hi : Float = 0.0; var first = true;
    for (g in groups.vals()) {
      for (v in g.vals()) {
        if (first) { lo := v; hi := v; first := false }
        else { if (v < lo) { lo := v }; if (v > hi) { hi := v } };
      };
    };
    if (first) { return (0.0, 1.0) };
    if (hi <= lo) { return (lo - 1.0, hi + 1.0) };
    let pad = (hi - lo) * 0.06;
    (lo - pad, hi + pad);
  };

  /// Smooth path through pixel pts via the foundation Catmull-Rom helper.
  func distSmooth(pts : [(Float, Float)]) : Text { smoothPath(pts) };

  /// Deterministic jitter in [-1,1] from (group,index) via a tiny hash; keeps
  /// strip/beeswarm output stable (server-render must be pure, no RNG).
  func distJitter(g : Nat, k : Nat) : Float {
    let h : Nat = (g * 73856093 + k * 19349663 + 12345) % 1000;
    Float.fromInt(h) / 500.0 - 1.0;
  };

  // ---- Histogram -----------------------------------------------------------
  /// Bins a single list of raw values into vertical frequency columns. `values`
  /// is a plain CSV; `bins` is the bin COUNT as text (""/"0" -> Sturges' rule).
  /// x axis = value range, y axis = per-bin count.
  /// `<Histogram values="..." bins="12" title="Latency" unit="ms" />`.
  public func histogram(valuesCsv : Text, binsCsv : Text, opts : O) : Text {
    let raw = parseFloats(valuesCsv);
    let n = raw.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-histogram") # svgClose() };

    let dataMin = arrMin(raw);
    let dataMax = arrMax(raw);
    let requested = switch (toFloat(binsCsv)) { case (?f) { Int.abs(Float.toInt(f)) }; case null { 0 } };
    var nbins : Nat = if (requested > 0) { requested } else {
      let s = Float.log(Float.fromInt(n)) / Float.log(2.0);
      let c = Int.abs(Float.toInt(Float.ceil(s))) + 1;
      if (c < 1) { 1 } else { c };
    };
    if (nbins > 40) { nbins := 40 };
    if (nbins < 1) { nbins := 1 };

    let span = if (dataMax - dataMin <= 0.0) { 1.0 } else { dataMax - dataMin };
    let binW = span / Float.fromInt(nbins);
    let counts = Array.init<Nat>(nbins, 0);
    for (v in raw.vals()) {
      var idx = Int.abs(Float.toInt(Float.floor((v - dataMin) / binW)));
      if (idx >= nbins) { idx := nbins - 1 };
      counts[idx] += 1;
    };
    var cmax : Nat = 0;
    for (c in counts.vals()) { if (c > cmax) { cmax := c } };

    let (ylo, yhi) = yDomain(opts, 0.0, Float.fromInt(cmax), true);
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());
    let xSc = linScale(dataMin, dataMin + binW * Float.fromInt(nbins), plotLeft(), plotRight(opts));
    let baseY = ySc(0.0);

    let b = Buffer.Buffer<Text>(nbins + 8);
    b.add(svgOpen(opts, "mv-chart-histogram"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 4)));
    b.add(axisBottomNumeric(opts, xSc, ticks(dataMin, dataMin + binW * Float.fromInt(nbins), 4)));

    let color = palette(opts, 0);
    var i : Nat = 0;
    while (i < nbins) {
      let lo = dataMin + binW * Float.fromInt(i);
      let hi = lo + binW;
      let x0 = xSc(lo);
      let x1 = xSc(hi);
      let w = Float.abs(x1 - x0) - 1.0;
      let cnt = counts[i];
      let yv = ySc(Float.fromInt(cnt));
      let h = Float.abs(baseY - yv);
      let tip = "[" # fmtNum(lo) # opts.unit # ", " # fmtNum(hi) # opts.unit # "): " # Nat.toText(cnt);
      b.add("<rect class=\"mv-chart-hist-bar\" x=\"" # fmtNum(x0 + 0.5) # "\" y=\"" # fmtNum(yv)
        # "\" width=\"" # fmtNum(if (w < 0.5) { 0.5 } else { w }) # "\" height=\"" # fmtNum(h)
        # "\" rx=\"1.5\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></rect>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== BoxPlot =====
  // ---- BoxPlot -------------------------------------------------------------
  /// One box-and-whisker per group from RAW values. Data is a labelled
  /// multi-series spec where each segment's values are that group's raw samples:
  /// `series="A:4,7,7,9,12;B:3,3,5,8,8,10"`. Renders min/Q1/median/Q3/max
  /// (Tukey whiskers at 1.5*IQR, clamped to data extent) + a mean dot.
  /// (Requires the shared dist* helpers from the Histogram entry.)
  public func boxPlot(seriesSpec : Text, opts : O) : Text {
    let groups = parseSeries(seriesSpec);
    let ng = groups.size();
    if (ng == 0) { return svgOpen(opts, "mv-chart-box") # svgClose() };

    let raws = Array.map<Series, [Float]>(groups, func(g) { g.1 });
    let (dlo, dhi) = distRangeOf(raws);
    let (ylo, yhi) = yDomain(opts, dlo, dhi, false);
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(ng);
    let boxW = band * 0.5;

    let b = Buffer.Buffer<Text>(ng * 6 + 8);
    b.add(svgOpen(opts, "mv-chart-box"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 5)));
    let labels = Array.map<Series, Text>(groups, func(g) { g.0 });
    let centers = Array.tabulate<Float>(ng, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    var i : Nat = 0;
    while (i < ng) {
      let (name, vals) = groups[i];
      if (vals.size() > 0) {
        let sorted = distSort(vals);
        let q1 = distQuantile(sorted, 0.25);
        let med = distQuantile(sorted, 0.5);
        let q3 = distQuantile(sorted, 0.75);
        let iqr = q3 - q1;
        let loFence = q1 - 1.5 * iqr;
        let hiFence = q3 + 1.5 * iqr;
        var wlo = sorted[sorted.size() - 1];
        var whi = sorted[0];
        for (v in sorted.vals()) {
          if (v >= loFence and v < wlo) { wlo := v };
          if (v <= hiFence and v > whi) { whi := v };
        };
        let mean = distMean(vals);
        let color = palette(opts, i);
        let cx = centers[i];
        let x0 = cx - boxW / 2.0;
        let yQ1 = ySc(q1); let yMed = ySc(med); let yQ3 = ySc(q3);
        let yWlo = ySc(wlo); let yWhi = ySc(whi);
        let cap = boxW * 0.4;
        b.add("<g class=\"mv-chart-box-g\">");
        b.add("<line class=\"mv-chart-box-whisk\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(yWlo)
          # "\" x2=\"" # fmtNum(cx) # "\" y2=\"" # fmtNum(yWhi) # "\" stroke=\"" # esc(color) # "\"/>");
        b.add("<line class=\"mv-chart-box-whisk\" x1=\"" # fmtNum(cx - cap) # "\" y1=\"" # fmtNum(yWhi)
          # "\" x2=\"" # fmtNum(cx + cap) # "\" y2=\"" # fmtNum(yWhi) # "\" stroke=\"" # esc(color) # "\"/>");
        b.add("<line class=\"mv-chart-box-whisk\" x1=\"" # fmtNum(cx - cap) # "\" y1=\"" # fmtNum(yWlo)
          # "\" x2=\"" # fmtNum(cx + cap) # "\" y2=\"" # fmtNum(yWlo) # "\" stroke=\"" # esc(color) # "\"/>");
        let tip = (if (name != "") { name # " \u{b7} " } else { "" })
          # "med " # fmtNum(med) # opts.unit # ", Q1 " # fmtNum(q1) # ", Q3 " # fmtNum(q3)
          # ", min " # fmtNum(wlo) # ", max " # fmtNum(whi);
        b.add("<rect class=\"mv-chart-box-rect\" x=\"" # fmtNum(x0) # "\" y=\"" # fmtNum(yQ3)
          # "\" width=\"" # fmtNum(boxW) # "\" height=\"" # fmtNum(Float.abs(yQ1 - yQ3))
          # "\" rx=\"2\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></rect>");
        b.add("<line class=\"mv-chart-box-median\" x1=\"" # fmtNum(x0) # "\" y1=\"" # fmtNum(yMed)
          # "\" x2=\"" # fmtNum(x0 + boxW) # "\" y2=\"" # fmtNum(yMed) # "\"/>");
        b.add("<circle class=\"mv-chart-box-mean\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(ySc(mean))
          # "\" r=\"2.5\"><title>" # esc("mean " # fmtNum(mean) # opts.unit) # "</title></circle>");
        b.add("</g>");
      };
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== ViolinPlot =====
  // ---- ViolinPlot ----------------------------------------------------------
  /// One mirrored KDE silhouette per group from RAW values (same data
  /// convention as BoxPlot: `series="A:..;B:.."`). Each violin is the kernel
  /// density estimate reflected about its center line, with median + quartile
  /// guide marks inside. (Requires the shared dist* helpers.)
  public func violinPlot(seriesSpec : Text, opts : O) : Text {
    let groups = parseSeries(seriesSpec);
    let ng = groups.size();
    if (ng == 0) { return svgOpen(opts, "mv-chart-violin") # svgClose() };

    let raws = Array.map<Series, [Float]>(groups, func(g) { g.1 });
    let (dlo, dhi) = distRangeOf(raws);
    let (ylo, yhi) = yDomain(opts, dlo, dhi, false);
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(ng);
    let halfW = band * 0.42;

    let b = Buffer.Buffer<Text>(ng * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-violin"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 5)));
    let labels = Array.map<Series, Text>(groups, func(g) { g.0 });
    let centers = Array.tabulate<Float>(ng, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    let steps : Nat = 48;
    var i : Nat = 0;
    while (i < ng) {
      let (name, vals) = groups[i];
      if (vals.size() > 0) {
        let sorted = distSort(vals);
        let bw = distBandwidth(vals, sorted);
        let gLo = sorted[0];
        let gHi = sorted[sorted.size() - 1];
        let density = distKde(vals, bw, gLo, gHi, steps);
        let peak = distPeak(density);
        let cx = centers[i];
        let color = palette(opts, i);
        let rightPts = Buffer.Buffer<(Float, Float)>(density.size());
        let leftPts = Buffer.Buffer<(Float, Float)>(density.size());
        for ((xv, d) in density.vals()) {
          let w = d / peak * halfW;
          let y = ySc(xv);
          rightPts.add((cx + w, y));
          leftPts.add((cx - w, y));
        };
        let poly = Buffer.Buffer<(Float, Float)>(density.size() * 2);
        for (p in rightPts.vals()) { poly.add(p) };
        var j : Int = leftPts.size() - 1;
        while (j >= 0) { poly.add(leftPts.get(Int.abs(j))); j -= 1 };
        let med = distQuantile(sorted, 0.5);
        let q1 = distQuantile(sorted, 0.25);
        let q3 = distQuantile(sorted, 0.75);
        let tip = (if (name != "") { name # " \u{b7} " } else { "" })
          # "n=" # Nat.toText(vals.size()) # ", med " # fmtNum(med) # opts.unit;
        b.add("<g class=\"mv-chart-violin-g\">");
        b.add("<polygon class=\"mv-chart-violin-body\" points=\"" # polyPoints(Buffer.toArray(poly))
          # "\" fill=\"" # esc(color) # "\" stroke=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></polygon>");
        b.add("<line class=\"mv-chart-violin-spine\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(ySc(q1))
          # "\" x2=\"" # fmtNum(cx) # "\" y2=\"" # fmtNum(ySc(q3)) # "\"/>");
        b.add("<circle class=\"mv-chart-violin-median\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(ySc(med))
          # "\" r=\"2.5\"><title>" # esc("median " # fmtNum(med) # opts.unit) # "</title></circle>");
        b.add("</g>");
      };
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== StripPlot =====
  // ---- StripPlot -----------------------------------------------------------
  /// Every raw datum drawn as a point on its group's vertical line (a "strip").
  /// Same data convention as BoxPlot: `series="A:4,7,7,9;B:3,5,8"`. Points get a
  /// small deterministic horizontal jitter so overlapping values stay legible.
  /// (Requires the shared dist* helpers — uses distJitter.)
  public func stripPlot(seriesSpec : Text, opts : O) : Text {
    let groups = parseSeries(seriesSpec);
    let ng = groups.size();
    if (ng == 0) { return svgOpen(opts, "mv-chart-strip") # svgClose() };

    let raws = Array.map<Series, [Float]>(groups, func(g) { g.1 });
    let (dlo, dhi) = distRangeOf(raws);
    let (ylo, yhi) = yDomain(opts, dlo, dhi, false);
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(ng);
    let jitterMax = band * 0.18;

    let b = Buffer.Buffer<Text>(ng * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-strip"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 5)));
    let labels = Array.map<Series, Text>(groups, func(g) { g.0 });
    let centers = Array.tabulate<Float>(ng, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    var i : Nat = 0;
    while (i < ng) {
      let (name, vals) = groups[i];
      let cx = centers[i];
      let color = palette(opts, i);
      b.add("<line class=\"mv-chart-strip-axis\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(plotTop())
        # "\" x2=\"" # fmtNum(cx) # "\" y2=\"" # fmtNum(plotBottom(opts)) # "\"/>");
      var k : Nat = 0;
      while (k < vals.size()) {
        let v = vals[k];
        let jr = distJitter(i, k);
        let px = cx + jr * jitterMax;
        let py = ySc(v);
        let tip = (if (name != "") { name # ": " } else { "" }) # fmtNum(v) # opts.unit;
        b.add("<circle class=\"mv-chart-strip-pt\" cx=\"" # fmtNum(px) # "\" cy=\"" # fmtNum(py)
          # "\" r=\"3\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
        k += 1;
      };
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== BeeswarmChart =====
  // ---- BeeswarmChart -------------------------------------------------------
  /// Like StripPlot but points are offset sideways to AVOID overlap (a
  /// "beeswarm"). Per group, values are sorted by y and greedily pushed left/
  /// right until they no longer collide with an already-placed neighbour. Same
  /// data convention: `series="A:..;B:.."`. (Requires shared dist* helpers.)
  public func beeswarm(seriesSpec : Text, opts : O) : Text {
    let groups = parseSeries(seriesSpec);
    let ng = groups.size();
    if (ng == 0) { return svgOpen(opts, "mv-chart-beeswarm") # svgClose() };

    let raws = Array.map<Series, [Float]>(groups, func(g) { g.1 });
    let (dlo, dhi) = distRangeOf(raws);
    let (ylo, yhi) = yDomain(opts, dlo, dhi, false);
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(ng);
    let r : Float = 3.0;
    let maxOff = band * 0.42;

    let b = Buffer.Buffer<Text>(ng * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-beeswarm"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 5)));
    let labels = Array.map<Series, Text>(groups, func(g) { g.0 });
    let centers = Array.tabulate<Float>(ng, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    var i : Nat = 0;
    while (i < ng) {
      let (name, vals) = groups[i];
      let cx = centers[i];
      let color = palette(opts, i);
      if (vals.size() > 0) {
        let sorted = distSort(vals);
        let placedY = Buffer.Buffer<Float>(sorted.size());
        let placedX = Buffer.Buffer<Float>(sorted.size());
        var k : Nat = 0;
        while (k < sorted.size()) {
          let v = sorted[k];
          let py = ySc(v);
          var off : Float = 0.0;
          var placed = false;
          var attempt : Nat = 0;
          let stepX = r * 2.0;
          while (not placed and attempt < 60) {
            let sign = if (attempt % 2 == 0) { 1.0 } else { -1.0 };
            let mag = Float.fromInt((attempt + 1) / 2) * stepX;
            let cand = sign * mag;
            var clash = false;
            var p : Nat = 0;
            while (p < placedY.size() and not clash) {
              let dy = Float.abs(placedY.get(p) - py);
              let dx = Float.abs(placedX.get(p) - cand);
              if (dy < stepX and dx < stepX) { clash := true };
              p += 1;
            };
            if (not clash) { off := cand; placed := true };
            attempt += 1;
          };
          if (off > maxOff) { off := maxOff };
          if (off < -maxOff) { off := -maxOff };
          placedY.add(py);
          placedX.add(off);
          let tip = (if (name != "") { name # ": " } else { "" }) # fmtNum(v) # opts.unit;
          b.add("<circle class=\"mv-chart-bee-pt\" cx=\"" # fmtNum(cx + off) # "\" cy=\"" # fmtNum(py)
            # "\" r=\"" # fmtNum(r) # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
          k += 1;
        };
      };
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== DensityPlot =====
  // ---- DensityPlot ---------------------------------------------------------
  /// A single smoothed KDE curve for one list of raw values (plain CSV). The x
  /// axis is the value range; y is the (peak-normalised) density. The area under
  /// the curve is filled. (Requires the shared dist* helpers.)
  public func densityPlot(valuesCsv : Text, opts : O) : Text {
    let raw = parseFloats(valuesCsv);
    let n = raw.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-density") # svgClose() };

    let sorted = distSort(raw);
    let bw = distBandwidth(raw, sorted);
    let (dlo, dhi) = distRangeOf([raw]);
    let lo = dlo - bw;
    let hi = dhi + bw;
    let steps : Nat = 80;
    let density = distKde(raw, bw, lo, hi, steps);
    let peak = distPeak(density);

    let xSc = linScale(lo, hi, plotLeft(), plotRight(opts));
    let ySc = linScale(0.0, peak * 1.08, plotBottom(opts), plotTop());
    let baseY = plotBottom(opts);

    let b = Buffer.Buffer<Text>(steps + 8);
    b.add(svgOpen(opts, "mv-chart-density"));
    b.add(axisLeft(opts, ySc, ticks(0.0, peak * 1.08, 4)));
    b.add(axisBottomNumeric(opts, xSc, ticks(lo, hi, 5)));

    let color = palette(opts, 0);
    let pts = Buffer.Buffer<(Float, Float)>(density.size());
    for ((xv, d) in density.vals()) { pts.add((xSc(xv), ySc(d))) };
    let ptsArr = Buffer.toArray(pts);
    if (ptsArr.size() > 0) {
      let (fx, _) = ptsArr[0];
      let (lx, _) = ptsArr[ptsArr.size() - 1];
      let d = distSmooth(ptsArr) # " L " # fmtNum(lx) # "," # fmtNum(baseY)
        # " L " # fmtNum(fx) # "," # fmtNum(baseY) # " Z";
      b.add("<path class=\"mv-chart-density-area\" d=\"" # d # "\" fill=\"" # esc(color) # "\"/>");
      b.add("<path class=\"mv-chart-density-line\" d=\"" # distSmooth(ptsArr)
        # "\" fill=\"none\" stroke=\"" # esc(color) # "\"><title>"
        # esc("n=" # Nat.toText(n) # ", mean " # fmtNum(distMean(raw)) # opts.unit) # "</title></path>");
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== RidgelinePlot =====
  // ---- RidgelinePlot -------------------------------------------------------
  /// Stacked, vertically overlapping density curves — one ridge per group
  /// (a.k.a. joyplot). Data is a labelled multi-series spec of RAW values:
  /// `series="2019:..;2020:..;2021:.."`. Groups are drawn top-to-bottom; each
  /// ridge's KDE is computed over the SHARED x range so they're comparable, and
  /// they overlap for the classic ridgeline look. (Requires shared dist* helpers.)
  public func ridgelinePlot(seriesSpec : Text, opts : O) : Text {
    let groups = parseSeries(seriesSpec);
    let ng = groups.size();
    if (ng == 0) { return svgOpen(opts, "mv-chart-ridgeline") # svgClose() };

    let raws = Array.map<Series, [Float]>(groups, func(g) { g.1 });
    let (dlo, dhi) = distRangeOf(raws);
    let xSc = linScale(dlo, dhi, plotLeft(), plotRight(opts));

    let b = Buffer.Buffer<Text>(ng * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-ridgeline"));
    b.add(axisBottomNumeric(opts, xSc, ticks(dlo, dhi, 5)));

    let top = plotTop();
    let bot = plotBottom(opts);
    let plotH = bot - top;
    let rowPitch = if (ng <= 1) { plotH * 0.6 } else { plotH / Float.fromInt(ng) };
    let ridgeH = rowPitch * 1.7;
    let steps : Nat = 64;

    // draw bottom-to-top so upper ridges paint OVER lower ones (overlap look)
    var ii : Int = ng - 1;
    while (ii >= 0) {
      let i = Int.abs(ii);
      let (name, vals) = groups[i];
      let baseY = top + rowPitch * (Float.fromInt(i) + 0.5);
      let color = palette(opts, i);
      b.add("<text class=\"mv-chart-ridge-label\" x=\"" # fmtNum(plotLeft() - 6.0)
        # "\" y=\"" # fmtNum(baseY + 3.0) # "\" text-anchor=\"end\">" # esc(name) # "</text>");
      b.add("<line class=\"mv-chart-ridge-base\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(baseY)
        # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(baseY) # "\"/>");
      if (vals.size() > 0) {
        let sorted = distSort(vals);
        let bw = distBandwidth(vals, sorted);
        let density = distKde(vals, bw, dlo, dhi, steps);
        let peak = distPeak(density);
        let pts = Buffer.Buffer<(Float, Float)>(density.size());
        for ((xv, d) in density.vals()) {
          let y = baseY - (d / peak) * ridgeH;
          pts.add((xSc(xv), y));
        };
        let ptsArr = Buffer.toArray(pts);
        if (ptsArr.size() > 0) {
          let (fx, _) = ptsArr[0];
          let (lx, _) = ptsArr[ptsArr.size() - 1];
          let path = distSmooth(ptsArr);
          let fillD = path # " L " # fmtNum(lx) # "," # fmtNum(baseY)
            # " L " # fmtNum(fx) # "," # fmtNum(baseY) # " Z";
          let tip = (if (name != "") { name # " \u{b7} " } else { "" })
            # "n=" # Nat.toText(vals.size()) # ", med " # fmtNum(distQuantile(sorted, 0.5)) # opts.unit;
          b.add("<path class=\"mv-chart-ridge-area\" d=\"" # fillD # "\" fill=\"" # esc(color)
            # "\"><title>" # esc(tip) # "</title></path>");
          b.add("<path class=\"mv-chart-ridge-line\" d=\"" # path # "\" fill=\"none\" stroke=\"" # esc(color) # "\"/>");
        };
      };
      ii -= 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== CandlestickChart =====
  // ---- CandlestickChart: OHLC candles (real body + high/low wick) ----------
  /// `<CandlestickChart ohlc="Mon:100,110,95,108;Tue:108,115,104,112" />`.
  /// Each ';' segment is "label:open,high,low,close". Up candles (close>=open)
  /// use palette index 1 (green family); down candles palette index 2 (red).
  public func candlestick(ohlcSpec : Text, opts : O) : Text {
    let bars = candleParse(ohlcSpec);
    let n = bars.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-candle") # svgClose() };

    // y-domain spans the extremes of every high/low.
    let allV = Buffer.Buffer<Float>(n * 2);
    for (bar in bars.vals()) { allV.add(bar.2); allV.add(bar.3) }; // high, low
    let arr = Buffer.toArray(allV);
    let dMin = arrMin(arr);
    let dMax = arrMax(arr);
    let lo = switch (opts.yMin) { case (?m) { m }; case null { dMin } };
    let hi0 = switch (opts.yMax) { case (?m) { m }; case null { dMax } };
    let hi = if (hi0 <= lo) { lo + 1.0 } else { hi0 };
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(n);
    let bodyW = band * 0.56;

    let b = Buffer.Buffer<Text>(n * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-candle"));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 4)));
    let labels = Array.map<(Text, Float, Float, Float, Float), Text>(bars, func(bar) { bar.0 });
    let centers = Array.tabulate<Float>(n, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    let upColor = palette(opts, 1);
    let downColor = palette(opts, 2);
    var i : Nat = 0;
    while (i < n) {
      let (lbl, o0, h0, l0, c0) = bars[i];
      let cx = centers[i];
      let up = c0 >= o0;
      let color = if (up) { upColor } else { downColor };
      let yHigh = ySc(h0);
      let yLow = ySc(l0);
      let yO = ySc(o0);
      let yC = ySc(c0);
      let yTop = minF(yO, yC);
      let bodyH0 = Float.abs(yC - yO);
      let bodyH = if (bodyH0 < 1.0) { 1.0 } else { bodyH0 }; // doji floor
      let bx = cx - bodyW / 2.0;
      let tip = lbl # " O:" # fmtNum(o0) # " H:" # fmtNum(h0) # " L:" # fmtNum(l0) # " C:" # fmtNum(c0) # opts.unit;
      b.add("<g class=\"mv-chart-candle-g\">");
      // wick: a single vertical line from high to low through the center
      b.add("<line class=\"mv-chart-candle-wick\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(yHigh)
        # "\" x2=\"" # fmtNum(cx) # "\" y2=\"" # fmtNum(yLow) # "\" stroke=\"" # esc(color) # "\"/>");
      b.add("<rect class=\"mv-chart-candle-body\" x=\"" # fmtNum(bx) # "\" y=\"" # fmtNum(yTop)
        # "\" width=\"" # fmtNum(bodyW) # "\" height=\"" # fmtNum(bodyH)
        # "\" rx=\"1\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc(tip) # "</title></rect>");
      b.add("</g>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse "label:o,h,l,c;..." -> [(label, open, high, low, close)]. Segments
  // with fewer than 4 numbers are skipped. low/high are recomputed defensively
  // so the wick always spans the true extremes.
  func candleParse(spec : Text) : [(Text, Float, Float, Float, Float)] {
    let out = Buffer.Buffer<(Text, Float, Float, Float, Float)>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = candleTrim(seg);
      if (s != "") {
        let parts = Iter.toArray(Text.split(s, #char ':'));
        var clbl = "";
        var nums = s;
        if (parts.size() >= 2) {
          clbl := candleTrim(parts[0]);
          var rest = parts[1];
          var k = 2;
          while (k < parts.size()) { rest := rest # ":" # parts[k]; k += 1 };
          nums := rest;
        };
        let fs = parseFloats(nums);
        if (fs.size() >= 4) {
          let o0 = fs[0]; var h0 = fs[1]; var l0 = fs[2]; let c0 = fs[3];
          // guard against transposed/sloppy data: high is the max, low the min.
          let hi = candleMax4(o0, h0, l0, c0);
          let lo = candleMin4(o0, h0, l0, c0);
          h0 := hi; l0 := lo;
          out.add((clbl, o0, h0, l0, c0));
        };
      };
    };
    Buffer.toArray(out);
  };
  func candleTrim(t : Text) : Text { Text.trimStart(Text.trimEnd(t, #char ' '), #char ' ') };
  func candleMax4(a : Float, b : Float, c : Float, d : Float) : Float {
    var m = a; if (b > m) { m := b }; if (c > m) { m := c }; if (d > m) { m := d }; m;
  };
  func candleMin4(a : Float, b : Float, c : Float, d : Float) : Float {
    var m = a; if (b < m) { m := b }; if (c < m) { m := c }; if (d < m) { m := d }; m;
  };

  // ===== OHLCChart =====
  // ---- OHLCChart: classic bar OHLC (high-low stick + L open / R close tick) -
  /// `<OHLCChart ohlc="Mon:100,110,95,108;Tue:108,115,104,112" />`.
  /// Reuses candleParse (see CandlestickChart). Each period is a vertical
  /// high-low line, with a left tick at open and a right tick at close.
  public func ohlc(ohlcSpec : Text, opts : O) : Text {
    let bars = candleParse(ohlcSpec);
    let n = bars.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-ohlc") # svgClose() };

    let allV = Buffer.Buffer<Float>(n * 2);
    for (bar in bars.vals()) { allV.add(bar.2); allV.add(bar.3) };
    let arr = Buffer.toArray(allV);
    let dMin = arrMin(arr);
    let dMax = arrMax(arr);
    let lo = switch (opts.yMin) { case (?m) { m }; case null { dMin } };
    let hi0 = switch (opts.yMax) { case (?m) { m }; case null { dMax } };
    let hi = if (hi0 <= lo) { lo + 1.0 } else { hi0 };
    let ySc = linScale(lo, hi, plotBottom(opts), plotTop());
    let left = plotLeft();
    let right = plotRight(opts);
    let band = (right - left) / Float.fromInt(n);
    let tickW = band * 0.30; // open/close tick arm length

    let b = Buffer.Buffer<Text>(n * 4 + 8);
    b.add(svgOpen(opts, "mv-chart-ohlc"));
    b.add(axisLeft(opts, ySc, ticks(lo, hi, 4)));
    let labels = Array.map<(Text, Float, Float, Float, Float), Text>(bars, func(bar) { bar.0 });
    let centers = Array.tabulate<Float>(n, func(i) { left + band * (Float.fromInt(i) + 0.5) });
    b.add(axisBottom(opts, labels, centers));

    let upColor = palette(opts, 1);
    let downColor = palette(opts, 2);
    var i : Nat = 0;
    while (i < n) {
      let (lbl, o0, h0, l0, c0) = bars[i];
      let cx = centers[i];
      let up = c0 >= o0;
      let color = if (up) { upColor } else { downColor };
      let yHigh = ySc(h0);
      let yLow = ySc(l0);
      let yO = ySc(o0);
      let yC = ySc(c0);
      let tip = lbl # " O:" # fmtNum(o0) # " H:" # fmtNum(h0) # " L:" # fmtNum(l0) # " C:" # fmtNum(c0) # opts.unit;
      b.add("<g class=\"mv-chart-ohlc-g\"><title>" # esc(tip) # "</title>");
      // high-low stick
      b.add("<line class=\"mv-chart-ohlc-bar\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(yHigh)
        # "\" x2=\"" # fmtNum(cx) # "\" y2=\"" # fmtNum(yLow) # "\" stroke=\"" # esc(color) # "\"/>");
      // open tick (left)
      b.add("<line class=\"mv-chart-ohlc-bar\" x1=\"" # fmtNum(cx - tickW) # "\" y1=\"" # fmtNum(yO)
        # "\" x2=\"" # fmtNum(cx) # "\" y2=\"" # fmtNum(yO) # "\" stroke=\"" # esc(color) # "\"/>");
      // close tick (right)
      b.add("<line class=\"mv-chart-ohlc-bar\" x1=\"" # fmtNum(cx) # "\" y1=\"" # fmtNum(yC)
        # "\" x2=\"" # fmtNum(cx + tickW) # "\" y2=\"" # fmtNum(yC) # "\" stroke=\"" # esc(color) # "\"/>");
      b.add("</g>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== GanttChart =====
  // ---- GanttChart: horizontal task bars over a numeric timeline -------------
  /// `<GanttChart tasks="Design:0,3;Build:3,8;Test:7,10" />`. Each ';' segment
  /// is "Task:start,end" on a shared numeric timeline. Bars stack top-to-bottom
  /// in declaration order; each cycles the palette and carries a tooltip.
  public func gantt(tasksSpec : Text, opts : O) : Text {
    let tasks = ganttParse(tasksSpec);
    let n = tasks.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-gantt") # svgClose() };

    // timeline domain = min start .. max end across all tasks.
    var tMin : Float = tasks[0].1;
    var tMax : Float = tasks[0].2;
    for ((_, s0, e0) in tasks.vals()) {
      if (s0 < tMin) { tMin := s0 };
      if (e0 > tMax) { tMax := e0 };
    };
    let lo = switch (opts.yMin) { case (?m) { m }; case null { tMin } };
    let hi0 = switch (opts.yMax) { case (?m) { m }; case null { tMax } };
    let hi = if (hi0 <= lo) { lo + 1.0 } else { hi0 };
    let xSc = linScale(lo, hi, plotLeft(), plotRight(opts));
    let top = plotTop();
    let bot = plotBottom(opts);
    let band = (bot - top) / Float.fromInt(n);
    let barH = band * 0.6;

    let b = Buffer.Buffer<Text>(n * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-gantt"));

    // timeline gridlines + bottom numeric axis
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
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(bot)
        # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(bot) # "\"/>");
    };

    var i : Nat = 0;
    while (i < n) {
      let (name, s0, e0) = tasks[i];
      let xa = xSc(minF(s0, e0));
      let xb = xSc(maxFG(s0, e0));
      let w0 = Float.abs(xb - xa);
      let w = if (w0 < 2.0) { 2.0 } else { w0 }; // zero-length milestone floor
      let yc = top + band * Float.fromInt(i) + (band - barH) / 2.0;
      let color = palette(opts, i);
      let tip = name # ": " # fmtNum(s0) # opts.unit # " \u{2192} " # fmtNum(e0) # opts.unit;
      b.add("<g class=\"mv-chart-gantt-g\">");
      b.add("<rect class=\"mv-chart-gantt-bar\" x=\"" # fmtNum(xa) # "\" y=\"" # fmtNum(yc)
        # "\" width=\"" # fmtNum(w) # "\" height=\"" # fmtNum(barH)
        # "\" rx=\"3\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc(tip) # "</title></rect>");
      b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(plotLeft() - 6.0)
        # "\" y=\"" # fmtNum(yc + barH / 2.0 + 4.0) # "\" text-anchor=\"end\">" # esc(name) # "</text>");
      b.add("</g>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse "Task:start,end;..." -> [(name, start, end)]. Segments needing a name
  // and >=2 numbers; extras ignored.
  func ganttParse(spec : Text) : [(Text, Float, Float)] {
    let out = Buffer.Buffer<(Text, Float, Float)>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = ganttTrim(seg);
      if (s != "") {
        let parts = Iter.toArray(Text.split(s, #char ':'));
        if (parts.size() >= 2) {
          let name = ganttTrim(parts[0]);
          var rest = parts[1];
          var k = 2;
          while (k < parts.size()) { rest := rest # ":" # parts[k]; k += 1 };
          let fs = parseFloats(rest);
          if (fs.size() >= 2) { out.add((name, fs[0], fs[1])) };
        };
      };
    };
    Buffer.toArray(out);
  };
  func ganttTrim(t : Text) : Text { Text.trimStart(Text.trimEnd(t, #char ' '), #char ' ') };
  func maxFG(a : Float, b : Float) : Float { if (a > b) { a } else { b } };

  // ===== StreamGraph =====
  // ---- StreamGraph: wiggle-centered stacked areas (themeriver) --------------
  /// `<StreamGraph series="A:5,8,6,9;B:3,4,7,5;C:2,3,3,6" labels="Q1,Q2,Q3,Q4" />`.
  /// Same data convention as StackedAreaChart, but each band flows around a
  /// centered baseline so the silhouette reads as a flowing stream. Negative
  /// values are clamped to 0.
  public func streamGraph(spec : Text, labelsCsv : Text, opts : O) : Text {
    let ss = parseSeries(spec);
    let ns = ss.size();
    if (ns == 0) { return svgOpen(opts, "mv-chart-stream") # svgClose() };

    var n : Nat = 0;
    for ((_, vs) in ss.vals()) { if (vs.size() > n) { n := vs.size() } };
    if (n == 0) { return svgOpen(opts, "mv-chart-stream") # svgClose() };
    let labels = parseLabels(labelsCsv);

    // per-x total (clamped >=0) -> the half-total gives the centered baseline.
    let totals = Array.init<Float>(n, 0.0);
    var si0 : Nat = 0;
    while (si0 < ns) {
      let (_, vs) = ss[si0];
      var i : Nat = 0;
      while (i < n) {
        let v = if (i < vs.size() and vs[i] > 0.0) { vs[i] } else { 0.0 };
        totals[i] += v;
        i += 1;
      };
      si0 += 1;
    };
    var maxTotal : Float = 0.0;
    for (t in totals.vals()) { if (t > maxTotal) { maxTotal := t } };
    if (maxTotal <= 0.0) { maxTotal := 1.0 };

    // symmetric domain about 0 so the centered stream fits the plot height.
    let half = maxTotal / 2.0;
    let ySc = linScale(-half, half, plotBottom(opts), plotTop());
    let centers = streamCenters(opts, n);

    let b = Buffer.Buffer<Text>(ns * 2 + 8);
    b.add(svgOpen(opts, "mv-chart-stream"));
    // light gridlines only (no numeric y axis: stream magnitude is relative).
    if (labels.size() > 0) { b.add(axisBottom(opts, labels, centers)) };

    // running baseline starts at -total/2 per x (centered).
    let running = Array.init<Float>(n, 0.0);
    do { var ri = 0; while (ri < n) { running[ri] := -(totals[ri] / 2.0); ri += 1 } };
    var si : Nat = 0;
    while (si < ns) {
      let (name0, vs) = ss[si];
      let color = palette(opts, si);
      let top = Buffer.Buffer<(Float, Float)>(n);
      let bot = Buffer.Buffer<(Float, Float)>(n);
      var i : Nat = 0;
      while (i < n) {
        let v = if (i < vs.size() and vs[i] > 0.0) { vs[i] } else { 0.0 };
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
        // smooth top edge forward, then smooth bottom edge backward, close.
        let topD = streamSmooth(topArr);
        let revB = Buffer.Buffer<(Float, Float)>(botArr.size());
        var j : Int = botArr.size() - 1;
        while (j >= 0) { revB.add(botArr[Int.abs(j)]); j -= 1 };
        let revArr = Buffer.toArray(revB);
        let botD = streamSmoothTail(revArr);
        let d = topD # " " # botD # " Z";
        let nm = if (name0 == "") { "Series " # Nat.toText(si + 1) } else { name0 };
        b.add("<path class=\"mv-chart-stream-band\" d=\"" # d # "\" fill=\"" # esc(color)
          # "\"><title>" # esc(nm) # "</title></path>");
      };
      si += 1;
    };
    let entries = Array.tabulate<(Text, Text)>(ns, func(i) {
      ((if (ss[i].0 == "") { "Series " # Nat.toText(i + 1) } else { ss[i].0 }), palette(opts, i));
    });
    b.add(legend(opts, entries));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // even x centers across the plot (single point centered).
  func streamCenters(opts : O, n : Nat) : [Float] {
    let x0 = plotLeft();
    let x1 = plotRight(opts);
    if (n == 0) { return [] };
    if (n == 1) { return [(x0 + x1) / 2.0] };
    let step = (x1 - x0) / Float.fromInt(n - 1);
    Array.tabulate<Float>(n, func(i) { x0 + step * Float.fromInt(i) });
  };
  // Catmull-Rom -> cubic Bezier smoothing, starting with an absolute moveto.
  func streamSmooth(pts : [(Float, Float)]) : Text {
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
    b.add(streamBezier(pts));
    Text.join(" ", b.vals());
  };
  // Same smoothing but WITHOUT the leading moveto (continues the current path,
  // used for the reversed bottom edge so the band stays one closed shape).
  func streamSmoothTail(pts : [(Float, Float)]) : Text {
    let n = pts.size();
    if (n == 0) { return "" };
    let (x0, y0) = pts[0];
    if (n < 3) {
      let b = Buffer.Buffer<Text>(n + 1);
      b.add("L " # fmtNum(x0) # "," # fmtNum(y0));
      var i : Nat = 1;
      while (i < n) { let (x, y) = pts[i]; b.add("L " # fmtNum(x) # "," # fmtNum(y)); i += 1 };
      return Text.join(" ", b.vals());
    };
    "L " # fmtNum(x0) # "," # fmtNum(y0) # " " # streamBezier(pts);
  };
  // The C-segment body shared by both smoothers.
  func streamBezier(pts : [(Float, Float)]) : Text {
    let n = pts.size();
    let b = Buffer.Buffer<Text>(n);
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

  // ===== BumpChart =====
  // ---- BumpChart: ranking lines over time (rank 1 at top) -------------------
  /// `<BumpChart series="Alice:1,2,1,3;Bob:2,1,3,1;Carol:3,3,2,2" labels="W1,W2,W3,W4" />`.
  /// Each series value is that entity's RANK (1 = best) at each time slot. Lines
  /// connect ranks across time with an emphasized dot per slot; rank 1 sits at
  /// the top of the plot.
  public func bump(spec : Text, labelsCsv : Text, opts : O) : Text {
    let ss = parseSeries(spec);
    let ns = ss.size();
    if (ns == 0) { return svgOpen(opts, "mv-chart-bump") # svgClose() };

    var n : Nat = 0;
    for ((_, vs) in ss.vals()) { if (vs.size() > n) { n := vs.size() } };
    if (n == 0) { return svgOpen(opts, "mv-chart-bump") # svgClose() };
    let labels = parseLabels(labelsCsv);

    // rank domain: 1 .. maxRank. Invert the y scale so rank 1 is at the TOP.
    var maxRank : Float = 1.0;
    for ((_, vs) in ss.vals()) { for (v in vs.vals()) { if (v > maxRank) { maxRank := v } } };
    // map rank 1 -> plotTop, rank maxRank -> plotBottom.
    let ySc = linScale(1.0, maxRank, plotTop(), plotBottom(opts));
    let centers = bumpCenters(opts, n);

    let b = Buffer.Buffer<Text>(ns * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-bump"));

    // left rank labels (1..maxRank) + horizontal gridlines
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(plotTop())
        # "\" x2=\"" # fmtNum(plotLeft()) # "\" y2=\"" # fmtNum(plotBottom(opts)) # "\"/>");
    };
    let mr : Nat = Int.abs(Float.toInt(maxRank));
    var r : Nat = 1;
    while (r <= mr) {
      let y = ySc(Float.fromInt(r));
      if (opts.showGrid) {
        b.add("<line class=\"mv-chart-grid\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(y)
          # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(y) # "\"/>");
      };
      b.add("<text class=\"mv-chart-tick\" x=\"" # fmtNum(plotLeft() - 6.0) # "\" y=\"" # fmtNum(y + 4.0)
        # "\" text-anchor=\"end\">" # esc("#" # Nat.toText(r)) # "</text>");
      r += 1;
    };
    if (labels.size() > 0) { b.add(axisBottom(opts, labels, centers)) };

    var si : Nat = 0;
    while (si < ns) {
      let (name0, vs) = ss[si];
      let color = palette(opts, si);
      let pts = Buffer.Buffer<(Float, Float)>(n);
      var i : Nat = 0;
      while (i < vs.size() and i < centers.size()) {
        pts.add((centers[i], ySc(vs[i])));
        i += 1;
      };
      let ptsArr = Buffer.toArray(pts);
      if (ptsArr.size() > 0) {
        let pb = Buffer.Buffer<Text>(ptsArr.size() + 1);
        let (x0, y0) = ptsArr[0];
        pb.add("M " # fmtNum(x0) # "," # fmtNum(y0));
        var k : Nat = 1;
        while (k < ptsArr.size()) { let (x, y) = ptsArr[k]; pb.add("L " # fmtNum(x) # "," # fmtNum(y)); k += 1 };
        let nm = if (name0 == "") { "Series " # Nat.toText(si + 1) } else { name0 };
        b.add("<g class=\"mv-chart-bump-g\">");
        b.add("<path class=\"mv-chart-bump-line\" d=\"" # Text.join(" ", pb.vals())
          # "\" fill=\"none\" stroke=\"" # esc(color) # "\"><title>" # esc(nm) # "</title></path>");
        var d : Nat = 0;
        while (d < ptsArr.size()) {
          let (px, py) = ptsArr[d];
          let lbl = if (d < labels.size()) { labels[d] } else { "" };
          let tip = nm # " " # lbl # ": #" # fmtNum(vs[d]);
          b.add("<circle class=\"mv-chart-bump-dot\" cx=\"" # fmtNum(px) # "\" cy=\"" # fmtNum(py)
            # "\" r=\"5\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
          d += 1;
        };
        b.add("</g>");
      };
      si += 1;
    };
    let entries = Array.tabulate<(Text, Text)>(ns, func(i) {
      ((if (ss[i].0 == "") { "Series " # Nat.toText(i + 1) } else { ss[i].0 }), palette(opts, i));
    });
    b.add(legend(opts, entries));
    b.add(svgClose());
    Text.join("", b.vals());
  };

  func bumpCenters(opts : O, n : Nat) : [Float] {
    let x0 = plotLeft();
    let x1 = plotRight(opts);
    if (n == 0) { return [] };
    if (n == 1) { return [(x0 + x1) / 2.0] };
    let step = (x1 - x0) / Float.fromInt(n - 1);
    Array.tabulate<Float>(n, func(i) { x0 + step * Float.fromInt(i) });
  };

  // ===== BarcodeChart =====
  // ---- BarcodeChart: 1D event ticks on a timeline (strip / rug plot) --------
  /// `<BarcodeChart events="3,7,7.5,12,18,18.2,25" />`. Each value is a tick on
  /// a horizontal axis. Pass `categories="a,b,a,c"` (parallel to events) to
  /// color-code ticks by category with a legend.
  public func barcode(eventsCsv : Text, categoriesCsv : Text, opts : O) : Text {
    let evs = parseFloats(eventsCsv);
    let cats = parseLabels(categoriesCsv);
    let n = evs.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-barcode") # svgClose() };

    let dMin = arrMin(evs);
    let dMax = arrMax(evs);
    let lo0 = switch (opts.yMin) { case (?m) { m }; case null { dMin } };
    let hi0 = switch (opts.yMax) { case (?m) { m }; case null { dMax } };
    // pad a flat domain so a single (or coincident) events still render.
    let (lo, hi) = if (hi0 <= lo0) {
      let pad = if (Float.abs(lo0) < 1.0) { 1.0 } else { Float.abs(lo0) * 0.1 };
      (lo0 - pad, lo0 + pad);
    } else { (lo0, hi0) };
    let xSc = linScale(lo, hi, plotLeft(), plotRight(opts));

    // distinct category -> palette index (stable first-seen order).
    let catKeys = Buffer.Buffer<Text>(4);
    for (cz in cats.vals()) {
      if (cz != "") {
        var found = false;
        for (k in catKeys.vals()) { if (k == cz) { found := true } };
        if (not found) { catKeys.add(cz) };
      };
    };
    let keys = Buffer.toArray(catKeys);

    // strip band geometry (a horizontal lane centered vertically in the plot).
    let top = plotTop();
    let bot = plotBottom(opts);
    let mid = (top + bot) / 2.0;
    let tickHalf = (bot - top) * 0.32; // half-height of each tick
    let y1 = mid - tickHalf;
    let y2 = mid + tickHalf;

    let b = Buffer.Buffer<Text>(n + 8);
    b.add(svgOpen(opts, "mv-chart-barcode"));

    // bottom numeric timeline axis + light vertical grid at ticks().
    let xticks = ticks(lo, hi, 4);
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(bot)
        # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(bot) # "\"/>");
    };
    for (tv in xticks.vals()) {
      let x = xSc(tv);
      b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(bot + 16.0)
        # "\" text-anchor=\"middle\">" # esc(fmtNum(tv) # opts.unit) # "</text>");
    };

    var i : Nat = 0;
    while (i < n) {
      let v = evs[i];
      let x = xSc(v);
      let cat = if (i < cats.size()) { cats[i] } else { "" };
      let ci = barcodeCatIndex(keys, cat);
      let color = palette(opts, ci);
      let tip = (if (cat != "") { cat # ": " } else { "" }) # fmtNum(v) # opts.unit;
      b.add("<line class=\"mv-chart-barcode-tick\" x1=\"" # fmtNum(x) # "\" y1=\"" # fmtNum(y1)
        # "\" x2=\"" # fmtNum(x) # "\" y2=\"" # fmtNum(y2) # "\" stroke=\"" # esc(color) # "\">"
        # "<title>" # esc(tip) # "</title></line>");
      i += 1;
    };
    if (keys.size() > 0) {
      let entries = Array.tabulate<(Text, Text)>(keys.size(), func(k) { (keys[k], palette(opts, k)) });
      b.add(legend(opts, entries));
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // index of `cat` within the distinct-keys array (0 if absent / uncategorized).
  func barcodeCatIndex(keys : [Text], cat : Text) : Nat {
    if (cat == "") { return 0 };
    var i : Nat = 0;
    while (i < keys.size()) { if (keys[i] == cat) { return i }; i += 1 };
    0;
  };

  // ===== Heatmap =====
  // ---- Heatmap: matrix of cells colored by value ---------------------------
  /// A row-major numeric matrix rendered as a grid of color-graded cells.
  /// `matrix` rows are ';'-separated, cells within a row ','-separated:
  ///   "12,30,5;8,22,40;3,9,18"  (3 rows x 3 cols)
  /// `rowLabels` / `colLabels` are CSV; either may be "". Color runs from a
  /// faint tint (min) to the full series color (max), per the foundation
  /// palette. Each cell carries a native <title> tooltip. Legend = a min..max
  /// color bar.
  public func heatmap(matrix : Text, rowLabels : Text, colLabels : Text, opts : O) : Text {
    let rows = heatRows(matrix);
    let nr = rows.size();
    if (nr == 0) { return svgOpen(opts, "mv-chart-heatmap") # svgClose() };
    var nc : Nat = 0;
    for (r in rows.vals()) { if (r.size() > nc) { nc := r.size() } };
    if (nc == 0) { return svgOpen(opts, "mv-chart-heatmap") # svgClose() };

    let rLabs = parseLabels(rowLabels);
    let cLabs = parseLabels(colLabels);

    // data range across every present cell
    let flat = Buffer.Buffer<Float>(nr * nc);
    for (r in rows.vals()) { for (v in r.vals()) { flat.add(v) } };
    let arr = Buffer.toArray(flat);
    let vlo = switch (opts.yMin) { case (?m) { m }; case null { arrMin(arr) } };
    let vhiRaw = switch (opts.yMax) { case (?m) { m }; case null { arrMax(arr) } };
    let vhi = if (vhiRaw <= vlo) { vlo + 1.0 } else { vhiRaw };

    // plot rect, leaving room on the left for row labels and top for col labels
    let left = if (rLabs.size() > 0) { plotLeft() } else { plotLeft() - 28.0 };
    let right = plotRight(opts);
    let top = plotTop() + (if (cLabs.size() > 0) { 8.0 } else { 0.0 });
    let bot = plotBottom(opts) + 16.0; // heatmap has no x-axis ticks below
    let cw = (right - left) / Float.fromInt(nc);
    let ch = (bot - top) / Float.fromInt(nr);
    let baseColor = palette(opts, 0);

    let b = Buffer.Buffer<Text>(nr * nc + nr + nc + 8);
    b.add(svgOpen(opts, "mv-chart-heatmap"));

    // column labels along the top
    if (cLabs.size() > 0) {
      var c : Nat = 0;
      while (c < nc) {
        let cx = left + cw * (Float.fromInt(c) + 0.5);
        b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(cx)
          # "\" y=\"" # fmtNum(top - 4.0) # "\" text-anchor=\"middle\">"
          # esc(labelAt(cLabs, c)) # "</text>");
        c += 1;
      };
    };

    var ri : Nat = 0;
    while (ri < nr) {
      let row = rows[ri];
      let cy = top + ch * Float.fromInt(ri);
      // row label
      if (rLabs.size() > 0) {
        b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(left - 6.0)
          # "\" y=\"" # fmtNum(cy + ch / 2.0 + 4.0) # "\" text-anchor=\"end\">"
          # esc(labelAt(rLabs, ri)) # "</text>");
      };
      var ci : Nat = 0;
      while (ci < nc) {
        let cx = left + cw * Float.fromInt(ci);
        if (ci < row.size()) {
          let v = row[ci];
          var frac = (v - vlo) / (vhi - vlo);
          if (frac < 0.0) { frac := 0.0 };
          if (frac > 1.0) { frac := 1.0 };
          let fill = heatBlend(baseColor, frac);
          let rl = labelAt(rLabs, ri);
          let cl = labelAt(cLabs, ci);
          let where = if (rl != "" or cl != "") {
            (if (rl != "") { rl } else { "r" # Nat.toText(ri + 1) }) # " / "
              # (if (cl != "") { cl } else { "c" # Nat.toText(ci + 1) }) # ": "
          } else { "" };
          b.add("<rect class=\"mv-chart-heat-cell\" x=\"" # fmtNum(cx + 1.0) # "\" y=\"" # fmtNum(cy + 1.0)
            # "\" width=\"" # fmtNum(maxF(cw - 2.0, 0.0)) # "\" height=\"" # fmtNum(maxF(ch - 2.0, 0.0))
            # "\" rx=\"2\" fill=\"" # esc(fill) # "\">"
            # "<title>" # esc(where # fmtNum(v) # opts.unit) # "</title></rect>");
        };
        ci += 1;
      };
      ri += 1;
    };

    // legend: a min..max color bar
    if (opts.showLegend) {
      let lw : Float = 120.0;
      let lh : Float = 8.0;
      let lx = left;
      let ly = Float.fromInt(Int.abs(opts.height)) - 12.0;
      let steps : Nat = 12;
      var k : Nat = 0;
      b.add("<g class=\"mv-chart-legend mv-chart-heat-legend\">");
      while (k < steps) {
        let frac = Float.fromInt(k) / Float.fromInt(steps - 1);
        let sx = lx + lw * Float.fromInt(k) / Float.fromInt(steps);
        b.add("<rect class=\"mv-chart-heat-swatch\" x=\"" # fmtNum(sx) # "\" y=\"" # fmtNum(ly - lh)
          # "\" width=\"" # fmtNum(lw / Float.fromInt(steps) + 0.5) # "\" height=\"" # fmtNum(lh)
          # "\" fill=\"" # esc(heatBlend(baseColor, frac)) # "\"/>");
        k += 1;
      };
      b.add("<text class=\"mv-chart-legend-label\" x=\"" # fmtNum(lx - 4.0) # "\" y=\"" # fmtNum(ly)
        # "\" text-anchor=\"end\">" # esc(fmtNum(vlo) # opts.unit) # "</text>");
      b.add("<text class=\"mv-chart-legend-label\" x=\"" # fmtNum(lx + lw + 4.0) # "\" y=\"" # fmtNum(ly)
        # "\" text-anchor=\"start\">" # esc(fmtNum(vhi) # opts.unit) # "</text>");
      b.add("</g>");
    };

    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse a ';'-separated, ','-celled numeric matrix into rows of floats.
  func heatRows(spec : Text) : [[Float]] {
    let out = Buffer.Buffer<[Float]>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = Text.trimStart(Text.trimEnd(seg, #char ' '), #char ' ');
      if (s != "") { out.add(parseFloats(s)) };
    };
    Buffer.toArray(out);
  };

  // Parse "#rrggbb" -> (r,g,b) as floats; falls back to a mid blue.
  // SHARED by Heatmap + HexbinChart (heatBlend). Define ONCE.
  func heatHex(hex : Text) : (Float, Float, Float) {
    let cs = Text.toArray(hex);
    if (cs.size() < 7 or cs[0] != '#') { return (15.0, 108.0, 189.0) };
    let hv = func(c : Char) : Float {
      let n : Nat32 = Char.toNat32(c);
      if (n >= 48 and n <= 57) { Float.fromInt(Nat32.toNat(n - 48)) }       // 0-9
      else if (n >= 97 and n <= 102) { Float.fromInt(Nat32.toNat(n - 87)) } // a-f
      else if (n >= 65 and n <= 70) { Float.fromInt(Nat32.toNat(n - 55)) }  // A-F
      else { 0.0 };
    };
    let r = hv(cs[1]) * 16.0 + hv(cs[2]);
    let g = hv(cs[3]) * 16.0 + hv(cs[4]);
    let bl = hv(cs[5]) * 16.0 + hv(cs[6]);
    (r, g, bl);
  };

  // Blend white -> baseColor by frac (0=faint tint, 1=full color) -> "rgb(...)".
  // SHARED by Heatmap + HexbinChart. Define ONCE.
  func heatBlend(baseColor : Text, frac : Float) : Text {
    let (r, g, bl) = heatHex(baseColor);
    // start at a light tint (88% toward white) so low cells stay readable
    let t = 0.12 + frac * 0.88;
    let mix = func(ch : Float) : Nat {
      let v = 255.0 + (ch - 255.0) * t;
      let iv = Float.toInt(v + 0.5);
      if (iv < 0) { 0 } else if (iv > 255) { 255 } else { Int.abs(iv) };
    };
    "rgb(" # Nat.toText(mix(r)) # "," # Nat.toText(mix(g)) # "," # Nat.toText(mix(bl)) # ")";
  };

  // ===== HexbinChart =====
  // ---- HexbinChart: hexagonal density binning of xy points -----------------
  /// Bins scattered xy points into a flat-top hexagonal grid; each occupied hex
  /// is shaded by its count (denser = stronger color). Same data convention as
  /// ScatterChart: `points="x,y;x,y;.."`. Axes are numeric.
  /// NOTE: depends on the SHARED helpers heatHex/heatBlend defined in the
  /// Heatmap section above (do NOT redefine them here).
  public func hexbin(pointsSpec : Text, opts : O) : Text {
    let pts = parseXY(pointsSpec);
    let n = pts.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-hexbin") # svgClose() };

    let xs = Array.map<Point, Float>(pts, func(p) { p.0 });
    let ys = Array.map<Point, Float>(pts, func(p) { p.1 });
    let (xlo, xhi) = xDomainNice(arrMin(xs), arrMax(xs));
    let (ylo, yhi) = yDomain(opts, arrMin(ys), arrMax(ys), false);
    let xSc = linScale(xlo, xhi, plotLeft(), plotRight(opts));
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());

    let b = Buffer.Buffer<Text>(n + 16);
    b.add(svgOpen(opts, "mv-chart-hexbin"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 4)));
    b.add(axisBottomNumeric(opts, xSc, ticks(xlo, xhi, 4)));

    // Pointy-top hex grid in PIXEL space. radius -> horizontal/vertical spacing.
    let radius : Float = 18.0;
    let hexW = sqrt_(3.0) * radius;        // column spacing
    let hexV = 1.5 * radius;               // row spacing
    let x0 = plotLeft();
    let y0 = plotTop();

    // bin each point into (col,row); offset odd rows by half a column.
    let binsX = Buffer.Buffer<Int>(n);
    let binsY = Buffer.Buffer<Int>(n);
    let counts = Buffer.Buffer<Nat>(n);
    var maxCount : Nat = 1;
    for ((x, y) in pts.vals()) {
      let px = xSc(x);
      let py = ySc(y);
      let row = Float.toInt(Float.floor((py - y0) / hexV + 0.5));
      let rowOdd = (Int.abs(row) % 2) == 1;
      let shift = if (rowOdd) { hexW / 2.0 } else { 0.0 };
      let col = Float.toInt(Float.floor((px - x0 - shift) / hexW + 0.5));
      // find or add this bin
      var found = false;
      var bi : Nat = 0;
      while (bi < binsX.size() and not found) {
        if (binsX.get(bi) == col and binsY.get(bi) == row) {
          let c = counts.get(bi) + 1;
          counts.put(bi, c);
          if (c > maxCount) { maxCount := c };
          found := true;
        };
        bi += 1;
      };
      if (not found) { binsX.add(col); binsY.add(row); counts.add(1) };
    };

    let baseColor = palette(opts, 0);
    var i : Nat = 0;
    while (i < binsX.size()) {
      let col = binsX.get(i);
      let row = binsY.get(i);
      let c = counts.get(i);
      let rowOdd = (Int.abs(row) % 2) == 1;
      let shift = if (rowOdd) { hexW / 2.0 } else { 0.0 };
      let cx = x0 + shift + Float.fromInt(col) * hexW;
      let cy = y0 + Float.fromInt(row) * hexV;
      let frac = Float.fromInt(c) / Float.fromInt(maxCount);
      let fill = heatBlend(baseColor, frac);
      b.add("<polygon class=\"mv-chart-hex-cell\" points=\"" # hexPoints(cx, cy, radius * 0.92)
        # "\" fill=\"" # esc(fill) # "\">"
        # "<title>" # esc(Nat.toText(c) # (if (c == 1) { " point" } else { " points" })) # "</title></polygon>");
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Pointy-top hexagon point list centered at (cx,cy) with circumradius r.
  func hexPoints(cx : Float, cy : Float, r : Float) : Text {
    let b = Buffer.Buffer<(Float, Float)>(6);
    var k : Nat = 0;
    let twoPiL : Float = 6.283185307179586;
    while (k < 6) {
      // pointy-top: first vertex straight up, step 60 degrees
      let ang = twoPiL * (Float.fromInt(k) / 6.0);
      b.add((polarX(cx, r, ang), polarY(cy, r, ang)));
      k += 1;
    };
    polyPoints(Buffer.toArray(b));
  };

  // ===== ConnectedScatterChart =====
  // ---- ConnectedScatterChart: scatter joined by an ordered path ------------
  /// Scatter points drawn IN THE GIVEN ORDER and joined by a path, so the
  /// reader follows a trajectory (e.g. a metric over time across two
  /// dimensions). Same data convention as ScatterChart: `points="x,y;x,y;.."`.
  /// Optional `pointLabels` CSV labels each vertex (in order). Start point is
  /// hollow, end point is filled solid.
  public func connectedScatter(pointsSpec : Text, pointLabels : Text, opts : O) : Text {
    let pts = parseXY(pointsSpec);
    let n = pts.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-connscatter") # svgClose() };

    let labs = parseLabels(pointLabels);
    let xs = Array.map<Point, Float>(pts, func(p) { p.0 });
    let ys = Array.map<Point, Float>(pts, func(p) { p.1 });
    let (xlo, xhi) = xDomainNice(arrMin(xs), arrMax(xs));
    let (ylo, yhi) = yDomain(opts, arrMin(ys), arrMax(ys), false);
    let xSc = linScale(xlo, xhi, plotLeft(), plotRight(opts));
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());

    let b = Buffer.Buffer<Text>(n + 12);
    b.add(svgOpen(opts, "mv-chart-connscatter"));
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 4)));
    b.add(axisBottomNumeric(opts, xSc, ticks(xlo, xhi, 4)));

    let color = palette(opts, 0);
    // ordered connecting polyline
    let pix = Buffer.Buffer<(Float, Float)>(n);
    for ((x, y) in pts.vals()) { pix.add((xSc(x), ySc(y))) };
    let pixArr = Buffer.toArray(pix);
    b.add("<polyline class=\"mv-chart-connscatter-path\" points=\"" # polyPoints(pixArr)
      # "\" fill=\"none\" stroke=\"" # esc(color) # "\"/>");

    // vertices: start hollow, end solid, middles standard
    let lastIdx : Nat = if (pixArr.size() == 0) { 0 } else { pixArr.size() - 1 };
    var i : Nat = 0;
    while (i < pixArr.size()) {
      let (cx, cy) = pixArr[i];
      let (x, y) = pts[i];
      let isStart = (i == 0);
      let isEnd = (i == lastIdx);
      let cls = if (isStart) { "mv-chart-connscatter-start" }
                else if (isEnd) { "mv-chart-connscatter-end" }
                else { "mv-chart-connscatter-mid" };
      let fill = if (isStart) { "var(--colorNeutralBackground1)" } else { color };
      let pl = labelAt(labs, i);
      let order = "#" # Nat.toText(i + 1);
      let tip = (if (pl != "") { pl # " " } else { "" }) # order # " ("
        # fmtNum(x) # ", " # fmtNum(y) # opts.unit # ")";
      b.add("<circle class=\"mv-chart-point " # cls # "\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"" # (if (isStart or isEnd) { "5" } else { "4" }) # "\" fill=\"" # esc(fill)
        # "\" stroke=\"" # esc(color) # "\">"
        # "<title>" # esc(tip) # "</title></circle>");
      if (pl != "") {
        b.add("<text class=\"mv-chart-tick mv-chart-connscatter-lbl\" x=\"" # fmtNum(cx + 6.0)
          # "\" y=\"" # fmtNum(cy - 6.0) # "\" text-anchor=\"start\">" # esc(pl) # "</text>");
      };
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== QuadrantChart =====
  // ---- QuadrantChart: scatter split into 4 quadrants by mid-lines ----------
  /// Scatter with two crossing reference lines splitting the plot into four
  /// quadrants, each tinted and named. Data is "label,x,y" per ';' segment:
  ///   "Alpha,8,9;Beta,3,7;Gamma,6,2"
  /// `axisLabels` = "xAxis,yAxis" (the dimension names). `quadLabels` names the
  /// four quadrants in order TR,TL,BL,BR: "Stars,Question,Dogs,Cash". The split
  /// is at the x/y data midpoints. Each point has a <title> tooltip.
  public func quadrant(dataSpec : Text, axisLabels : Text, quadLabels : Text, opts : O) : Text {
    let rows = quadParse(dataSpec);
    let n = rows.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-quadrant") # svgClose() };

    let axes = parseLabels(axisLabels);
    let quads = parseLabels(quadLabels);
    let xs = Array.map<(Text, Float, Float), Float>(rows, func(r) { r.1 });
    let ys = Array.map<(Text, Float, Float), Float>(rows, func(r) { r.2 });
    let (xlo, xhi) = xDomainNice(arrMin(xs), arrMax(xs));
    let (ylo, yhi) = yDomain(opts, arrMin(ys), arrMax(ys), false);
    let xSc = linScale(xlo, xhi, plotLeft(), plotRight(opts));
    let ySc = linScale(ylo, yhi, plotBottom(opts), plotTop());

    // split points (value space) -> midpoints of the data range
    let xMid = (arrMin(xs) + arrMax(xs)) / 2.0;
    let yMid = (arrMin(ys) + arrMax(ys)) / 2.0;
    let xMidPx = xSc(xMid);
    let yMidPx = ySc(yMid);
    let L = plotLeft(); let R = plotRight(opts);
    let T = plotTop(); let Bm = plotBottom(opts);

    let b = Buffer.Buffer<Text>(n + 16);
    b.add(svgOpen(opts, "mv-chart-quadrant"));

    // four faint quadrant background tints (TR,TL,BL,BR using palette 1/4/7/3)
    b.add("<rect class=\"mv-chart-quad-bg\" x=\"" # fmtNum(xMidPx) # "\" y=\"" # fmtNum(T)
      # "\" width=\"" # fmtNum(maxF(R - xMidPx, 0.0)) # "\" height=\"" # fmtNum(maxF(yMidPx - T, 0.0))
      # "\" fill=\"" # esc(palette(opts, 1)) # "\" fill-opacity=\"0.07\"/>"); // TR
    b.add("<rect class=\"mv-chart-quad-bg\" x=\"" # fmtNum(L) # "\" y=\"" # fmtNum(T)
      # "\" width=\"" # fmtNum(maxF(xMidPx - L, 0.0)) # "\" height=\"" # fmtNum(maxF(yMidPx - T, 0.0))
      # "\" fill=\"" # esc(palette(opts, 4)) # "\" fill-opacity=\"0.07\"/>"); // TL
    b.add("<rect class=\"mv-chart-quad-bg\" x=\"" # fmtNum(L) # "\" y=\"" # fmtNum(yMidPx)
      # "\" width=\"" # fmtNum(maxF(xMidPx - L, 0.0)) # "\" height=\"" # fmtNum(maxF(Bm - yMidPx, 0.0))
      # "\" fill=\"" # esc(palette(opts, 7)) # "\" fill-opacity=\"0.07\"/>"); // BL
    b.add("<rect class=\"mv-chart-quad-bg\" x=\"" # fmtNum(xMidPx) # "\" y=\"" # fmtNum(yMidPx)
      # "\" width=\"" # fmtNum(maxF(R - xMidPx, 0.0)) # "\" height=\"" # fmtNum(maxF(Bm - yMidPx, 0.0))
      # "\" fill=\"" # esc(palette(opts, 3)) # "\" fill-opacity=\"0.07\"/>"); // BR

    // axes + grid
    b.add(axisLeft(opts, ySc, ticks(ylo, yhi, 4)));
    b.add(axisBottomNumeric(opts, xSc, ticks(xlo, xhi, 4)));

    // crossing mid-lines
    b.add("<line class=\"mv-chart-quad-mid\" x1=\"" # fmtNum(xMidPx) # "\" y1=\"" # fmtNum(T)
      # "\" x2=\"" # fmtNum(xMidPx) # "\" y2=\"" # fmtNum(Bm) # "\"/>");
    b.add("<line class=\"mv-chart-quad-mid\" x1=\"" # fmtNum(L) # "\" y1=\"" # fmtNum(yMidPx)
      # "\" x2=\"" # fmtNum(R) # "\" y2=\"" # fmtNum(yMidPx) # "\"/>");

    // quadrant corner labels (TR,TL,BL,BR)
    if (quads.size() > 0) {
      let pad : Float = 6.0;
      b.add(quadLabel(labelAt(quads, 0), R - pad, T + 14.0, "end"));   // TR
      b.add(quadLabel(labelAt(quads, 1), L + pad, T + 14.0, "start")); // TL
      b.add(quadLabel(labelAt(quads, 2), L + pad, Bm - pad, "start")); // BL
      b.add(quadLabel(labelAt(quads, 3), R - pad, Bm - pad, "end"));   // BR
    };

    // axis dimension names
    if (axes.size() > 0) {
      b.add("<text class=\"mv-chart-tick mv-chart-quad-axislbl\" x=\"" # fmtNum((L + R) / 2.0)
        # "\" y=\"" # fmtNum(Bm + 30.0) # "\" text-anchor=\"middle\">" # esc(labelAt(axes, 0)) # "</text>");
      if (axes.size() > 1) {
        let lyc = (T + Bm) / 2.0;
        b.add("<text class=\"mv-chart-tick mv-chart-quad-axislbl\" x=\"" # fmtNum(L - 40.0)
          # "\" y=\"" # fmtNum(lyc) # "\" text-anchor=\"middle\" transform=\"rotate(-90 "
          # fmtNum(L - 40.0) # " " # fmtNum(lyc) # ")\">" # esc(labelAt(axes, 1)) # "</text>");
      };
    };

    // points
    let color = palette(opts, 0);
    var i : Nat = 0;
    while (i < n) {
      let (nm, x, y) = rows[i];
      let cx = xSc(x);
      let cy = ySc(y);
      b.add("<circle class=\"mv-chart-point mv-chart-quad-pt\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"5\" fill=\"" # esc(color) # "\">"
        # "<title>" # esc((if (nm != "") { nm # " " } else { "" }) # "(" # fmtNum(x) # ", " # fmtNum(y) # opts.unit # ")")
        # "</title></circle>");
      if (nm != "") {
        b.add("<text class=\"mv-chart-tick mv-chart-quad-ptlbl\" x=\"" # fmtNum(cx + 7.0)
          # "\" y=\"" # fmtNum(cy + 3.0) # "\" text-anchor=\"start\">" # esc(nm) # "</text>");
      };
      i += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // Parse "label,x,y;label,x,y" -> [(label,x,y)]. A leading non-numeric token is
  // the label; if the segment is just "x,y" the label is "".
  func quadParse(spec : Text) : [(Text, Float, Float)] {
    let out = Buffer.Buffer<(Text, Float, Float)>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = Text.trimStart(Text.trimEnd(seg, #char ' '), #char ' ');
      if (s != "") {
        let parts = Iter.toArray(Text.split(s, #char ','));
        if (parts.size() >= 3) {
          let nm = Text.trimStart(Text.trimEnd(parts[0], #char ' '), #char ' ');
          switch (toFloat(parts[1]), toFloat(parts[2])) {
            case (?x, ?y) { out.add((nm, x, y)) };
            case _ {};
          };
        } else if (parts.size() == 2) {
          switch (toFloat(parts[0]), toFloat(parts[1])) {
            case (?x, ?y) { out.add(("", x, y)) };
            case _ {};
          };
        };
      };
    };
    Buffer.toArray(out);
  };

  func quadLabel(t : Text, x : Float, y : Float, anchor : Text) : Text {
    if (t == "") { return "" };
    "<text class=\"mv-chart-quad-label\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(y)
      # "\" text-anchor=\"" # anchor # "\">" # esc(t) # "</text>";
  };

  // ===== SankeyDiagram =====
  // ---- SankeyDiagram: two-column flow ---------------------------------------
  /// Flows between a SOURCE column and a TARGET column. Data: one
  /// "Source>Target:value" link per ';' segment, e.g.
  /// `<SankeyDiagram links="Visit>Signup:120;Visit>Bounce:80;Signup>Paid:40" />`.
  /// Left nodes = all distinct sources, right nodes = all distinct targets; a
  /// name that is both a source and a target appears once on EACH side. Node
  /// heights are proportional to their throughput; ribbons are proportional to
  /// the link value. Native <title> tooltips on nodes + links.
  type SankeyLink = { src : Text; dst : Text; value : Float };

  func sankeyParse(spec : Text) : [SankeyLink] {
    let out = Buffer.Buffer<SankeyLink>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        // split "src>dst:value"
        let arrow = Iter.toArray(Text.split(s, #char '>'));
        if (arrow.size() >= 2) {
          let src = trim(arrow[0]);
          // rejoin remainder after first '>'
          var rest = arrow[1];
          var k = 2;
          while (k < arrow.size()) { rest := rest # ">" # arrow[k]; k += 1 };
          let colon = Iter.toArray(Text.split(rest, #char ':'));
          if (colon.size() >= 2) {
            let dst = trim(colon[0]);
            switch (toFloat(colon[1])) {
              case (?v) { if (v > 0.0 and src != "" and dst != "") { out.add({ src; dst; value = v }) } };
              case null {};
            };
          };
        };
      };
    };
    Buffer.toArray(out);
  };

  // distinct names in first-seen order, drawn from a selector over links
  func sankeyDistinct(links : [SankeyLink], pickSrc : Bool) : [Text] {
    let out = Buffer.Buffer<Text>(8);
    for (l in links.vals()) {
      let name = if (pickSrc) { l.src } else { l.dst };
      var found = false;
      for (e in out.vals()) { if (e == name) { found := true } };
      if (not found) { out.add(name) };
    };
    Buffer.toArray(out);
  };

  func sankeyIndexOf(xs : [Text], name : Text) : Int {
    var i : Nat = 0;
    while (i < xs.size()) { if (xs[i] == name) { return i }; i += 1 };
    -1;
  };

  // sum of link values flowing OUT of (pickSrc) / INTO a node name
  func sankeyThroughput(links : [SankeyLink], name : Text, pickSrc : Bool) : Float {
    var s : Float = 0.0;
    for (l in links.vals()) {
      let m = if (pickSrc) { l.src } else { l.dst };
      if (m == name) { s += l.value };
    };
    s;
  };

  public func sankey(links : Text, opts : O) : Text {
    let ls = sankeyParse(links);
    let b = Buffer.Buffer<Text>(ls.size() * 2 + 8);
    b.add(svgOpen(opts, "mv-chart-sankey"));
    if (ls.size() == 0) {
      let (ecx, ecy) = ((plotLeft() + plotRight(opts)) / 2.0, (plotTop() + plotBottom(opts)) / 2.0);
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ecx) # "\" y=\"" # fmtNum(ecy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    let sources = sankeyDistinct(ls, true);
    let targets = sankeyDistinct(ls, false);
    let left = plotLeft();
    let right = plotRight(opts);
    let top = plotTop();
    let bot = plotBottom(opts);
    let nodeW : Float = 14.0;
    let gap : Float = 8.0; // vertical gap between stacked nodes

    // total throughput per side -> vertical scale (px per value unit)
    var srcTotal : Float = 0.0;
    for (n in sources.vals()) { srcTotal += sankeyThroughput(ls, n, true) };
    var dstTotal : Float = 0.0;
    for (n in targets.vals()) { dstTotal += sankeyThroughput(ls, n, false) };
    let total = maxF(srcTotal, dstTotal);
    let plotH = bot - top;
    let srcGaps = gap * Float.fromInt(if (sources.size() > 0) { sources.size() - 1 } else { 0 });
    let dstGaps = gap * Float.fromInt(if (targets.size() > 0) { targets.size() - 1 } else { 0 });
    let availH = plotH - maxF(srcGaps, dstGaps);
    let pxPer = if (total <= 0.0) { 0.0 } else { availH / total };

    // node top-y + running height tracking, by index, per side
    let srcY = Array.init<Float>(sources.size(), top);
    let srcH = Array.init<Float>(sources.size(), 0.0);
    let srcCursor = Array.init<Float>(sources.size(), 0.0); // consumed height as ribbons attach
    var sy : Float = top;
    var si : Nat = 0;
    while (si < sources.size()) {
      let h = sankeyThroughput(ls, sources[si], true) * pxPer;
      srcY[si] := sy;
      srcH[si] := h;
      srcCursor[si] := sy;
      sy += h + gap;
      si += 1;
    };
    let dstY = Array.init<Float>(targets.size(), top);
    let dstH = Array.init<Float>(targets.size(), 0.0);
    let dstCursor = Array.init<Float>(targets.size(), 0.0);
    var dy : Float = top;
    var di : Nat = 0;
    while (di < targets.size()) {
      let h = sankeyThroughput(ls, targets[di], false) * pxPer;
      dstY[di] := dy;
      dstH[di] := h;
      dstCursor[di] := dy;
      dy += h + gap;
      di += 1;
    };

    let leftX = left;
    let rightX = right - nodeW;

    // ribbons first (under the nodes)
    b.add("<g class=\"mv-chart-sankey-links\">");
    var li : Nat = 0;
    while (li < ls.size()) {
      let l = ls[li];
      let sIdx = sankeyIndexOf(sources, l.src);
      let tIdx = sankeyIndexOf(targets, l.dst);
      if (sIdx >= 0 and tIdx >= 0) {
        let s0 = Int.abs(sIdx);
        let t0 = Int.abs(tIdx);
        let lh = l.value * pxPer;
        let y0 = srcCursor[s0];
        let y1 = dstCursor[t0];
        srcCursor[s0] := y0 + lh;
        dstCursor[t0] := y1 + lh;
        let x0 = leftX + nodeW;
        let x1 = rightX;
        let xm = (x0 + x1) / 2.0;
        // ribbon as a closed path: top edge (cubic) + down + bottom edge (cubic back)
        let topEdge = "M " # fmtNum(x0) # "," # fmtNum(y0)
          # " C " # fmtNum(xm) # "," # fmtNum(y0) # " " # fmtNum(xm) # "," # fmtNum(y1) # " " # fmtNum(x1) # "," # fmtNum(y1);
        let botEdge = " L " # fmtNum(x1) # "," # fmtNum(y1 + lh)
          # " C " # fmtNum(xm) # "," # fmtNum(y1 + lh) # " " # fmtNum(xm) # "," # fmtNum(y0 + lh) # " " # fmtNum(x0) # "," # fmtNum(y0 + lh) # " Z";
        let color = palette(opts, s0);
        let tip = l.src # " \u{2192} " # l.dst # ": " # fmtNum(l.value) # opts.unit;
        b.add("<path class=\"mv-chart-sankey-link\" d=\"" # topEdge # botEdge # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></path>");
      };
      li += 1;
    };
    b.add("</g>");

    // nodes + labels
    b.add("<g class=\"mv-chart-sankey-nodes\">");
    var sj : Nat = 0;
    while (sj < sources.size()) {
      let h = srcH[sj];
      let color = palette(opts, sj);
      let tip = sources[sj] # ": " # fmtNum(sankeyThroughput(ls, sources[sj], true)) # opts.unit;
      b.add("<rect class=\"mv-chart-sankey-node\" x=\"" # fmtNum(leftX) # "\" y=\"" # fmtNum(srcY[sj])
        # "\" width=\"" # fmtNum(nodeW) # "\" height=\"" # fmtNum(if (h < 1.0) { 1.0 } else { h })
        # "\" rx=\"2\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></rect>");
      b.add("<text class=\"mv-chart-sankey-label\" x=\"" # fmtNum(leftX + nodeW + 4.0) # "\" y=\"" # fmtNum(srcY[sj] + h / 2.0 + 4.0)
        # "\" text-anchor=\"start\">" # esc(sources[sj]) # "</text>");
      sj += 1;
    };
    var dj : Nat = 0;
    while (dj < targets.size()) {
      let h = dstH[dj];
      let color = palette(opts, dj + sources.size());
      let tip = targets[dj] # ": " # fmtNum(sankeyThroughput(ls, targets[dj], false)) # opts.unit;
      b.add("<rect class=\"mv-chart-sankey-node\" x=\"" # fmtNum(rightX) # "\" y=\"" # fmtNum(dstY[dj])
        # "\" width=\"" # fmtNum(nodeW) # "\" height=\"" # fmtNum(if (h < 1.0) { 1.0 } else { h })
        # "\" rx=\"2\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></rect>");
      b.add("<text class=\"mv-chart-sankey-label\" x=\"" # fmtNum(rightX - 4.0) # "\" y=\"" # fmtNum(dstY[dj] + h / 2.0 + 4.0)
        # "\" text-anchor=\"end\">" # esc(targets[dj]) # "</text>");
      dj += 1;
    };
    b.add("</g>");
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== ChordDiagram =====
  // ---- ChordDiagram: circular relationships ---------------------------------
  /// Circular node arcs with ribbons between them, from a square matrix.
  /// Data: rows separated by ';', cells (Floats) by ','; an OPTIONAL labels CSV
  /// names the nodes (else "N1, N2 ..."). matrix[i][j] = weight from i to j.
  /// `<ChordDiagram matrix="0,5,8;5,0,3;8,3,0" labels="A,B,C" />`.
  /// Each node owns an outer arc sized by its total degree; ribbons connect
  /// node pairs (symmetric pairs are merged). Native <title> tooltips.
  func chordMatrix(spec : Text) : [[Float]] {
    let rows = Buffer.Buffer<[Float]>(4);
    for (rowSeg in Text.split(spec, #char ';')) {
      let r = trim(rowSeg);
      if (r != "") { rows.add(parseFloats(r)) };
    };
    Buffer.toArray(rows);
  };

  public func chord(matrix : Text, labelsCsv : Text, opts : O) : Text {
    let m = chordMatrix(matrix);
    let labs = parseLabels(labelsCsv);
    let n = m.size();
    let b = Buffer.Buffer<Text>(n * n + 12);
    b.add(svgOpen(opts, "mv-chart-chord"));
    let (cx, cy) = centerXY(opts);
    let maxR = minF(cx, cy) - 18.0;
    let outerR = if (maxR < 12.0) { 12.0 } else { maxR };
    let innerR = outerR - 12.0; // arc band thickness

    if (n == 0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };

    // degree of each node = sum of its row + its column (incoming+outgoing),
    // excluding self-loops counted once.
    let degree = Array.init<Float>(n, 0.0);
    var grand : Float = 0.0;
    var i0 : Nat = 0;
    while (i0 < n) {
      var d : Float = 0.0;
      let row = m[i0];
      var j0 : Nat = 0;
      while (j0 < n) {
        let outv = if (j0 < row.size()) { row[j0] } else { 0.0 };
        let inv = if (i0 < m[j0].size()) { m[j0][i0] } else { 0.0 };
        d += maxF(outv, 0.0) + maxF(inv, 0.0);
        j0 += 1;
      };
      degree[i0] := d;
      grand += d;
      i0 += 1;
    };
    if (grand <= 0.0) {
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };

    // gap fraction between node arcs
    let padFrac : Float = 0.012; // per node
    let totalPad = padFrac * Float.fromInt(n);
    let usable = 1.0 - totalPad;

    // node arc start fractions (around the circle, 0..1)
    let arcStart = Array.init<Float>(n, 0.0);
    let arcEnd = Array.init<Float>(n, 0.0);
    var acc : Float = 0.0;
    var ni : Nat = 0;
    while (ni < n) {
      let frac = degree[ni] / grand * usable;
      arcStart[ni] := acc;
      arcEnd[ni] := acc + frac;
      acc += frac + padFrac;
      ni += 1;
    };

    // per-node running cursor used to lay ribbon endpoints inside the arc
    let cursor = Array.init<Float>(n, 0.0);
    var ci : Nat = 0;
    while (ci < n) { cursor[ci] := arcStart[ci]; ci += 1 };

    // ribbons: for each unordered pair (i,j) with i<=j combine m[i][j]+m[j][i]
    b.add("<g class=\"mv-chart-chord-links\">");
    var i : Nat = 0;
    while (i < n) {
      var j : Nat = i;
      while (j < n) {
        let aij = if (j < m[i].size()) { maxF(m[i][j], 0.0) } else { 0.0 };
        let aji = if (i < m[j].size()) { maxF(m[j][i], 0.0) } else { 0.0 };
        let w = if (i == j) { aij } else { aij + aji };
        if (w > 0.0) {
          // fraction of the circle each endpoint consumes on its own node arc
          let fi = w / grand * usable;
          let s0i = cursor[i]; let e0i = s0i + fi;
          cursor[i] := e0i;
          if (i == j) {
            // self loop: a small arc back onto itself
            let a0 = s0i * twoPi; let a1 = e0i * twoPi;
            let p0x = polarX(cx, innerR, a0); let p0y = polarY(cy, innerR, a0);
            let p1x = polarX(cx, innerR, a1); let p1y = polarY(cy, innerR, a1);
            let color = palette(opts, i);
            let tip = chordName(labs, i) # " \u{2194} " # chordName(labs, i) # ": " # fmtNum(w) # opts.unit;
            b.add("<path class=\"mv-chart-chord-ribbon\" d=\"M " # fmtNum(p0x) # "," # fmtNum(p0y)
              # " Q " # fmtNum(cx) # "," # fmtNum(cy) # " " # fmtNum(p1x) # "," # fmtNum(p1y)
              # "\" fill=\"none\" stroke=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></path>");
          } else {
            let fj = w / grand * usable;
            let s0j = cursor[j]; let e0j = s0j + fj;
            cursor[j] := e0j;
            // endpoints on the inner radius
            let ai0 = s0i * twoPi; let ai1 = e0i * twoPi;
            let aj0 = s0j * twoPi; let aj1 = e0j * twoPi;
            let ix0 = polarX(cx, innerR, ai0); let iy0 = polarY(cy, innerR, ai0);
            let ix1 = polarX(cx, innerR, ai1); let iy1 = polarY(cy, innerR, ai1);
            let jx0 = polarX(cx, innerR, aj0); let jy0 = polarY(cy, innerR, aj0);
            let jx1 = polarX(cx, innerR, aj1); let jy1 = polarY(cy, innerR, aj1);
            // ribbon: arc i, quad to arc j, arc j, quad back to i, through center
            let large_i = if (e0i - s0i > 0.5) { "1" } else { "0" };
            let large_j = if (e0j - s0j > 0.5) { "1" } else { "0" };
            let d = "M " # fmtNum(ix0) # "," # fmtNum(iy0)
              # " A " # fmtNum(innerR) # "," # fmtNum(innerR) # " 0 " # large_i # " 1 " # fmtNum(ix1) # "," # fmtNum(iy1)
              # " Q " # fmtNum(cx) # "," # fmtNum(cy) # " " # fmtNum(jx0) # "," # fmtNum(jy0)
              # " A " # fmtNum(innerR) # "," # fmtNum(innerR) # " 0 " # large_j # " 1 " # fmtNum(jx1) # "," # fmtNum(jy1)
              # " Q " # fmtNum(cx) # "," # fmtNum(cy) # " " # fmtNum(ix0) # "," # fmtNum(iy0) # " Z";
            let color = palette(opts, i);
            let tip = chordName(labs, i) # " \u{2194} " # chordName(labs, j) # ": " # fmtNum(w) # opts.unit;
            b.add("<path class=\"mv-chart-chord-ribbon\" d=\"" # d # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></path>");
          };
        };
        j += 1;
      };
      i += 1;
    };
    b.add("</g>");

    // node arcs + labels (on top)
    b.add("<g class=\"mv-chart-chord-nodes\">");
    var k : Nat = 0;
    while (k < n) {
      let color = palette(opts, k);
      if (arcEnd[k] > arcStart[k]) {
        let tip = chordName(labs, k) # ": " # fmtNum(degree[k]) # opts.unit;
        b.add("<path class=\"mv-chart-chord-arc\" fill=\"" # esc(color) # "\" d=\""
          # arcPath(cx, cy, outerR, innerR, arcStart[k], arcEnd[k]) # "\"><title>" # esc(tip) # "</title></path>");
        let mid = (arcStart[k] + arcEnd[k]) / 2.0 * twoPi;
        let lx = polarX(cx, outerR + 10.0, mid);
        let ly = polarY(cy, outerR + 10.0, mid);
        let anchor = if (lx > cx + 1.0) { "start" } else if (lx < cx - 1.0) { "end" } else { "middle" };
        b.add("<text class=\"mv-chart-chord-label\" x=\"" # fmtNum(lx) # "\" y=\"" # fmtNum(ly + 3.0)
          # "\" text-anchor=\"" # anchor # "\">" # esc(chordName(labs, k)) # "</text>");
      };
      k += 1;
    };
    b.add("</g>");
    b.add(svgClose());
    Text.join("", b.vals());
  };

  func chordName(labs : [Text], i : Nat) : Text {
    if (i < labs.size() and labs[i] != "") { labs[i] } else { "N" # Nat.toText(i + 1) };
  };

  // ===== ArcDiagram =====
  // ---- ArcDiagram: nodes on a line, arcs for edges --------------------------
  /// Nodes laid along a horizontal baseline; each edge is a semicircular arc
  /// hopping above the line. Data: edges "A>B" per ';' segment (an optional
  /// ":weight" sets arc thickness). Node order = first-seen across edges, or
  /// override with a `nodes` CSV. `<ArcDiagram edges="A>B;B>C;A>C;C>D" />`.
  /// Node size reflects its degree (edge count). Native <title> tooltips.
  type ArcEdge = { a : Text; b : Text; w : Float };

  func arcParse(spec : Text) : [ArcEdge] {
    let out = Buffer.Buffer<ArcEdge>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        // optional ":weight" suffix
        var core = s;
        var w : Float = 1.0;
        let colon = Iter.toArray(Text.split(s, #char ':'));
        if (colon.size() >= 2) {
          core := trim(colon[0]);
          switch (toFloat(colon[1])) { case (?v) { if (v > 0.0) { w := v } }; case null {} };
        };
        let arrow = Iter.toArray(Text.split(core, #char '>'));
        if (arrow.size() >= 2) {
          let a = trim(arrow[0]);
          let bn = trim(arrow[1]);
          if (a != "" and bn != "") { out.add({ a; b = bn; w }) };
        };
      };
    };
    Buffer.toArray(out);
  };

  func arcNodes(edges : [ArcEdge], nodesCsv : Text) : [Text] {
    if (trim(nodesCsv) != "") { return parseLabels(nodesCsv) };
    let out = Buffer.Buffer<Text>(8);
    func push(name : Text) {
      var found = false;
      for (e in out.vals()) { if (e == name) { found := true } };
      if (not found) { out.add(name) };
    };
    for (e in edges.vals()) { push(e.a); push(e.b) };
    Buffer.toArray(out);
  };

  func arcIndexOf(xs : [Text], name : Text) : Int {
    var i : Nat = 0;
    while (i < xs.size()) { if (xs[i] == name) { return i }; i += 1 };
    -1;
  };

  func arcDegree(edges : [ArcEdge], name : Text) : Nat {
    var d : Nat = 0;
    for (e in edges.vals()) { if (e.a == name or e.b == name) { d += 1 } };
    d;
  };

  public func arc(edges : Text, nodesCsv : Text, opts : O) : Text {
    let es = arcParse(edges);
    let nodes = arcNodes(es, nodesCsv);
    let n = nodes.size();
    let b = Buffer.Buffer<Text>(es.size() + n + 8);
    b.add(svgOpen(opts, "mv-chart-arc"));
    if (n == 0) {
      let (ecx, ecy) = ((plotLeft() + plotRight(opts)) / 2.0, (plotTop() + plotBottom(opts)) / 2.0);
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ecx) # "\" y=\"" # fmtNum(ecy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    let left = plotLeft();
    let right = plotRight(opts);
    // baseline near the bottom, leaving room above for arcs + labels below
    let baseY = plotBottom(opts) - 4.0;
    let xs = Array.init<Float>(n, left);
    if (n == 1) { xs[0] := (left + right) / 2.0 }
    else {
      let step = (right - left) / Float.fromInt(n - 1);
      var i : Nat = 0;
      while (i < n) { xs[i] := left + step * Float.fromInt(i); i += 1 };
    };
    let maxArcH = baseY - plotTop() - 6.0;

    // arcs (under nodes)
    b.add("<g class=\"mv-chart-arc-edges\">");
    for (e in es.vals()) {
      let ia = arcIndexOf(nodes, e.a);
      let ib = arcIndexOf(nodes, e.b);
      if (ia >= 0 and ib >= 0 and ia != ib) {
        let xa = xs[Int.abs(ia)];
        let xb = xs[Int.abs(ib)];
        let x0 = minF(xa, xb);
        let x1 = maxF(xa, xb);
        let span = x1 - x0;
        // semicircle radius = half the span, but cap by available height
        var r = span / 2.0;
        if (r * 2.0 > maxArcH) { r := maxArcH / 2.0 };
        // an upper semicircle from (x0,baseY) to (x1,baseY)
        let d = "M " # fmtNum(x0) # "," # fmtNum(baseY)
          # " A " # fmtNum(span / 2.0) # "," # fmtNum(r) # " 0 0 1 " # fmtNum(x1) # "," # fmtNum(baseY);
        let color = palette(opts, Int.abs(ia));
        let sw = if (e.w < 1.0) { 1.0 } else { e.w };
        let tip = e.a # " \u{2014} " # e.b # (if (e.w != 1.0) { ": " # fmtNum(e.w) # opts.unit } else { "" });
        b.add("<path class=\"mv-chart-arc-edge\" d=\"" # d # "\" fill=\"none\" stroke=\"" # esc(color)
          # "\" stroke-width=\"" # fmtNum(sw) # "\"><title>" # esc(tip) # "</title></path>");
      };
    };
    b.add("</g>");

    // baseline
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(left) # "\" y1=\"" # fmtNum(baseY)
        # "\" x2=\"" # fmtNum(right) # "\" y2=\"" # fmtNum(baseY) # "\"/>");
    };

    // nodes + labels
    b.add("<g class=\"mv-chart-arc-nodes\">");
    var i : Nat = 0;
    while (i < n) {
      let deg = arcDegree(es, nodes[i]);
      let r = 3.0 + Float.fromInt(deg) * 1.2;
      let rr = if (r > 9.0) { 9.0 } else { r };
      let color = palette(opts, i);
      let tip = nodes[i] # ": " # Nat.toText(deg) # (if (deg == 1) { " link" } else { " links" });
      b.add("<circle class=\"mv-chart-arc-node\" cx=\"" # fmtNum(xs[i]) # "\" cy=\"" # fmtNum(baseY)
        # "\" r=\"" # fmtNum(rr) # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
      b.add("<text class=\"mv-chart-arc-label\" x=\"" # fmtNum(xs[i]) # "\" y=\"" # fmtNum(baseY + 16.0)
        # "\" text-anchor=\"middle\">" # esc(nodes[i]) # "</text>");
      i += 1;
    };
    b.add("</g>");
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== Dendrogram =====
  // ---- Dendrogram: top-down tree -------------------------------------------
  /// A top-down hierarchy from "parent>child" edges (one per ';' segment). The
  /// root is the node that never appears as a child. Leaves spread evenly across
  /// the bottom; internal nodes sit centered above their children with elbow
  /// connectors. `<Dendrogram edges="Root>A;Root>B;A>A1;A>A2;B>B1" />`.
  /// Native <title> tooltips name each node + its child count.
  type DendroEdge = { parent : Text; child : Text };

  func dendroParse(spec : Text) : [DendroEdge] {
    let out = Buffer.Buffer<DendroEdge>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        let arrow = Iter.toArray(Text.split(s, #char '>'));
        if (arrow.size() >= 2) {
          let p = trim(arrow[0]);
          var ch = arrow[1];
          var k = 2;
          while (k < arrow.size()) { ch := ch # ">" # arrow[k]; k += 1 };
          let c = trim(ch);
          if (p != "" and c != "") { out.add({ parent = p; child = c }) };
        };
      };
    };
    Buffer.toArray(out);
  };

  // distinct node names, parents-first then children, first-seen order
  func dendroNodes(edges : [DendroEdge]) : [Text] {
    let out = Buffer.Buffer<Text>(8);
    func push(name : Text) {
      var found = false;
      for (e in out.vals()) { if (e == name) { found := true } };
      if (not found) { out.add(name) };
    };
    for (e in edges.vals()) { push(e.parent); push(e.child) };
    Buffer.toArray(out);
  };

  func dendroChildren(edges : [DendroEdge], parent : Text) : [Text] {
    let out = Buffer.Buffer<Text>(4);
    for (e in edges.vals()) { if (e.parent == parent) { out.add(e.child) } };
    Buffer.toArray(out);
  };

  func dendroIsChild(edges : [DendroEdge], name : Text) : Bool {
    for (e in edges.vals()) { if (e.child == name) { return true } };
    false;
  };

  // depth of a node = longest path from root (computed by walking parents).
  func dendroDepth(edges : [DendroEdge], name : Text, guard : Nat) : Nat {
    if (guard == 0) { return 0 };
    var d : Nat = 0;
    for (e in edges.vals()) {
      if (e.child == name) {
        let pd = dendroDepth(edges, e.parent, guard - 1) + 1;
        if (pd > d) { d := pd };
      };
    };
    d;
  };

  // count leaves under a node (a node with no children counts as 1 leaf).
  func dendroLeafCount(edges : [DendroEdge], name : Text, guard : Nat) : Nat {
    if (guard == 0) { return 1 };
    let kids = dendroChildren(edges, name);
    if (kids.size() == 0) { return 1 };
    var s : Nat = 0;
    for (k in kids.vals()) { s += dendroLeafCount(edges, k, guard - 1) };
    s;
  };

  public func dendrogram(edges : Text, opts : O) : Text {
    let es = dendroParse(edges);
    let nodes = dendroNodes(es);
    let n = nodes.size();
    let b = Buffer.Buffer<Text>(n * 3 + 8);
    b.add(svgOpen(opts, "mv-chart-dendrogram"));
    if (n == 0) {
      let (ecx, ecy) = ((plotLeft() + plotRight(opts)) / 2.0, (plotTop() + plotBottom(opts)) / 2.0);
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ecx) # "\" y=\"" # fmtNum(ecy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    let guard = n + 1;
    // root = first node that is never a child
    var root : Text = nodes[0];
    var foundRoot = false;
    for (nm in nodes.vals()) {
      if (not foundRoot and not dendroIsChild(es, nm)) { root := nm; foundRoot := true };
    };

    // max depth -> y bands
    var maxD : Nat = 0;
    for (nm in nodes.vals()) { let d = dendroDepth(es, nm, guard); if (d > maxD) { maxD := d } };
    let left = plotLeft();
    let right = plotRight(opts);
    let top = plotTop() + 4.0;
    let bot = plotBottom(opts);
    let levelH = if (maxD == 0) { 0.0 } else { (bot - top) / Float.fromInt(maxD) };

    // leaf x assignment: walk leaves left->right in tree order, then internal
    // nodes get centered over their children.
    let totalLeaves = dendroLeafCount(es, root, guard);
    let leafGap = if (totalLeaves <= 1) { 0.0 } else { (right - left) / Float.fromInt(totalLeaves - 1) };

    // compute x for each node by index, using a mutable leaf cursor
    let xPos = Array.init<Float>(n, (left + right) / 2.0);
    let yPos = Array.init<Float>(n, top);
    let placed = Array.init<Bool>(n, false);
    let leafCursor = Array.init<Float>(1, 0.0); // mutable counter (#leaves placed)

    // index helper
    func idxOf(name : Text) : Nat {
      var i : Nat = 0;
      while (i < n) { if (nodes[i] == name) { return i }; i += 1 };
      0;
    };

    // recursive layout via an explicit worklist is awkward in Motoko without
    // closures-over-mutable; use a bounded recursive helper returning the x.
    func layout(name : Text, g : Nat) : Float {
      let idx = idxOf(name);
      let depth = dendroDepth(es, name, guard);
      yPos[idx] := top + levelH * Float.fromInt(depth);
      let kids = dendroChildren(es, name);
      if (kids.size() == 0 or g == 0) {
        let x = left + leafGap * leafCursor[0];
        leafCursor[0] += 1.0;
        xPos[idx] := x;
        placed[idx] := true;
        return x;
      };
      var sumX : Float = 0.0;
      var cnt : Float = 0.0;
      for (k in kids.vals()) { sumX += layout(k, g - 1); cnt += 1.0 };
      let x = if (cnt > 0.0) { sumX / cnt } else { (left + right) / 2.0 };
      xPos[idx] := x;
      placed[idx] := true;
      x;
    };
    ignore layout(root, guard);
    // any unplaced (disconnected) nodes -> stack them as extra leaves
    var u : Nat = 0;
    while (u < n) {
      if (not placed[u]) {
        let depth = dendroDepth(es, nodes[u], guard);
        yPos[u] := top + levelH * Float.fromInt(depth);
        xPos[u] := left + leafGap * leafCursor[0];
        leafCursor[0] += 1.0;
        placed[u] := true;
      };
      u += 1;
    };

    // connectors (elbow: parent down to mid, across, down to child)
    b.add("<g class=\"mv-chart-dendro-links\">");
    for (e in es.vals()) {
      let pi = idxOf(e.parent);
      let ck = idxOf(e.child);
      let px = xPos[pi]; let py = yPos[pi];
      let cxp = xPos[ck]; let cyp = yPos[ck];
      let midY = (py + cyp) / 2.0;
      let d = "M " # fmtNum(px) # "," # fmtNum(py)
        # " V " # fmtNum(midY)
        # " H " # fmtNum(cxp)
        # " V " # fmtNum(cyp);
      b.add("<path class=\"mv-chart-dendro-link\" d=\"" # d # "\" fill=\"none\"/>");
    };
    b.add("</g>");

    // nodes + labels
    b.add("<g class=\"mv-chart-dendro-nodes\">");
    var i : Nat = 0;
    while (i < n) {
      let kids = dendroChildren(es, nodes[i]);
      let isLeaf = kids.size() == 0;
      let color = palette(opts, dendroDepth(es, nodes[i], guard));
      let tip = nodes[i] # (if (isLeaf) { " (leaf)" } else { ": " # Nat.toText(kids.size()) # " children" });
      b.add("<circle class=\"mv-chart-dendro-node\" cx=\"" # fmtNum(xPos[i]) # "\" cy=\"" # fmtNum(yPos[i])
        # "\" r=\"" # (if (isLeaf) { "3.5" } else { "4.5" }) # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
      let lblY = if (isLeaf) { yPos[i] + 14.0 } else { yPos[i] - 8.0 };
      b.add("<text class=\"mv-chart-dendro-label\" x=\"" # fmtNum(xPos[i]) # "\" y=\"" # fmtNum(lblY)
        # "\" text-anchor=\"middle\">" # esc(nodes[i]) # "</text>");
      i += 1;
    };
    b.add("</g>");
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== VennDiagram =====
  // ---- VennDiagram: 2 or 3 overlapping sets ---------------------------------
  /// Two or three labelled overlapping circles. Data is a set-sizes spec where
  /// each ';' segment is "Key:size"; the Key is either a single set id (A, B, C)
  /// or an intersection of ids (AB, BC, ABC). Sizes scale circle radii (by the
  /// set's TOTAL size = own + shared) and tune overlap. Labels CSV names A,B,C.
  /// `<VennDiagram sets="A:60;B:40;AB:15" labels="Users,Buyers" />`  (2-set)
  /// `<VennDiagram sets="A:50;B:40;C:30;AB:12;AC:9;BC:8;ABC:4" labels="X,Y,Z" />`.
  /// Each region carries a native <title> tooltip with its size.
  func vennSize(spec : Text, key : Text) : Float {
    for (seg in Text.split(spec, #char ';')) {
      let s = trim(seg);
      if (s != "") {
        let kv = Iter.toArray(Text.split(s, #char ':'));
        if (kv.size() >= 2 and trim(kv[0]) == key) {
          switch (toFloat(kv[1])) { case (?v) { return v }; case null { return 0.0 } };
        };
      };
    };
    0.0;
  };

  // total magnitude of a single set = its own-only size plus all intersections
  // that include it (so the circle reads proportionally).
  func vennTotal(spec : Text, id : Text, three : Bool) : Float {
    var t = vennSize(spec, id);
    if (id == "A") {
      t += vennSize(spec, "AB") + vennSize(spec, "AC");
      if (three) { t += vennSize(spec, "ABC") };
    } else if (id == "B") {
      t += vennSize(spec, "AB") + vennSize(spec, "BC");
      if (three) { t += vennSize(spec, "ABC") };
    } else if (id == "C") {
      t += vennSize(spec, "AC") + vennSize(spec, "BC");
      if (three) { t += vennSize(spec, "ABC") };
    };
    t;
  };

  func vennCircle(cx : Float, cy : Float, r : Float, color : Text, tip : Text) : Text {
    "<circle class=\"mv-chart-venn-circle\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
      # "\" r=\"" # fmtNum(r) # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>";
  };

  func vennLabel(x : Float, y : Float, t : Text) : Text {
    "<text class=\"mv-chart-venn-label\" x=\"" # fmtNum(x) # "\" y=\"" # fmtNum(y)
      # "\" text-anchor=\"middle\">" # esc(t) # "</text>";
  };

  public func venn(sets : Text, labelsCsv : Text, opts : O) : Text {
    let labs = parseLabels(labelsCsv);
    let three = vennTotal(sets, "C", true) > 0.0 or vennSize(sets, "C") > 0.0
      or vennSize(sets, "ABC") > 0.0 or vennSize(sets, "AC") > 0.0 or vennSize(sets, "BC") > 0.0;
    let b = Buffer.Buffer<Text>(12);
    b.add(svgOpen(opts, "mv-chart-venn"));
    let (cx, cy) = centerXY(opts);
    let span = minF(cx, cy);
    let baseR = (span - 12.0) * 0.55;
    let r0 = if (baseR < 16.0) { 16.0 } else { baseR };

    let tA = vennTotal(sets, "A", three);
    let tB = vennTotal(sets, "B", three);
    let tC = vennTotal(sets, "C", three);
    let tMax = maxF(maxF(tA, tB), maxF(tC, 1.0));
    // radius ~ sqrt(size) (area-proportional), floored so all stay visible
    func rOf(t : Float) : Float {
      if (t <= 0.0) { return r0 * 0.55 };
      let fr = sqrt_(t / tMax);
      r0 * (0.55 + 0.45 * fr);
    };
    let rA = rOf(tA);
    let rB = rOf(tB);
    let rC = rOf(tC);

    let nameA = if (labs.size() > 0 and labs[0] != "") { labs[0] } else { "A" };
    let nameB = if (labs.size() > 1 and labs[1] != "") { labs[1] } else { "B" };
    let nameC = if (labs.size() > 2 and labs[2] != "") { labs[2] } else { "C" };
    let cA = palette(opts, 0);
    let cB = palette(opts, 1);
    let cC = palette(opts, 2);

    if (not three) {
      // two circles, horizontally offset to overlap ~40%
      let off = (rA + rB) * 0.42;
      let ax = cx - off + (rB - rA) * 0.2;
      let bx = cx + off + (rB - rA) * 0.2;
      let ovl = vennSize(sets, "AB");
      b.add("<g class=\"mv-chart-venn-sets\">");
      b.add(vennCircle(ax, cy, rA, cA, nameA # ": " # fmtNum(tA) # opts.unit));
      b.add(vennCircle(bx, cy, rB, cB, nameB # ": " # fmtNum(tB) # opts.unit));
      b.add("</g>");
      // labels
      b.add(vennLabel(ax - rA * 0.45, cy - rA * 0.5, nameA));
      b.add(vennLabel(bx + rB * 0.45, cy - rB * 0.5, nameB));
      // region size annotations
      b.add(vennLabel(ax - rA * 0.4, cy + 4.0, fmtNum(vennSize(sets, "A")) # opts.unit));
      b.add(vennLabel(bx + rB * 0.4, cy + 4.0, fmtNum(vennSize(sets, "B")) # opts.unit));
      b.add(vennLabel((ax + bx) / 2.0, cy + 4.0, fmtNum(ovl) # opts.unit));
    } else {
      // three circles arranged as a triangle (A top-left, B top-right, C bottom)
      let off = (rA + rB + rC) / 3.0 * 0.5;
      let ax = cx - off;       let ay = cy - off * 0.55;
      let bx = cx + off;       let by = cy - off * 0.55;
      let ccx = cx;            let ccy = cy + off * 0.7;
      b.add("<g class=\"mv-chart-venn-sets\">");
      b.add(vennCircle(ax, ay, rA, cA, nameA # ": " # fmtNum(tA) # opts.unit));
      b.add(vennCircle(bx, by, rB, cB, nameB # ": " # fmtNum(tB) # opts.unit));
      b.add(vennCircle(ccx, ccy, rC, cC, nameC # ": " # fmtNum(tC) # opts.unit));
      b.add("</g>");
      b.add(vennLabel(ax - rA * 0.5, ay - rA * 0.4, nameA));
      b.add(vennLabel(bx + rB * 0.5, by - rB * 0.4, nameB));
      b.add(vennLabel(ccx, ccy + rC * 0.7, nameC));
      // pairwise + triple intersection size annotations near the overlaps
      b.add(vennLabel((ax + bx) / 2.0, (ay + by) / 2.0 - 4.0, fmtNum(vennSize(sets, "AB")) # opts.unit));
      b.add(vennLabel((ax + ccx) / 2.0 - 6.0, (ay + ccy) / 2.0 + 6.0, fmtNum(vennSize(sets, "AC")) # opts.unit));
      b.add(vennLabel((bx + ccx) / 2.0 + 6.0, (by + ccy) / 2.0 + 6.0, fmtNum(vennSize(sets, "BC")) # opts.unit));
      b.add(vennLabel(cx, cy + 4.0, fmtNum(vennSize(sets, "ABC")) # opts.unit));
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== NightingaleChart =====
// ---- NightingaleChart (rose / polar-area) -------------------------------
/// Florence Nightingale's polar-area / rose diagram: every wedge spans the
/// SAME angle (a full circle / n), and the wedge RADIUS is proportional to its
/// value (radius = outerR * value / niceCeil(max)). A value's visual weight is
/// thus its radius (per the brief), unlike a pie where it is the angle. Faint
/// value rings when showGrid; native <title> tooltip per wedge.
/// `<NightingaleChart values="42,30,55,20,38" labels="Jan,Feb,Mar,Apr,May" />`.
public func nightingale(values : Text, labels : Text, opts : O) : Text {
  let vs = parseFloats(values);
  let labs = parseLabels(labels);
  let b = Buffer.Buffer<Text>(vs.size() * 2 + 8);
  b.add(svgOpen(opts, "mv-chart-rose"));
  let (cx, cy) = centerXY(opts);
  let maxR0 = minF(cx, cy) - 16.0;
  let outerR = if (maxR0 < 8.0) { 8.0 } else { maxR0 };
  let n = vs.size();
  if (n == 0) {
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  let rawMax = arrMax(vs);
  let hi = switch (opts.yMax) { case (?m) { m }; case null { if (rawMax <= 0.0) { 1.0 } else { niceCeil(rawMax) } } };
  let denom = if (hi == 0.0) { 1.0 } else { hi };
  // faint concentric value rings + outer guide
  if (opts.showGrid) {
    b.add("<g class=\"mv-chart-rose-grid\">");
    let rings : Nat = 4;
    var r : Nat = 1;
    while (r <= rings) {
      let rr = outerR * Float.fromInt(r) / Float.fromInt(rings);
      b.add("<circle class=\"mv-chart-grid\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy) # "\" r=\"" # fmtNum(rr) # "\"/>");
      r += 1;
    };
    b.add("</g>");
  };
  b.add("<g class=\"mv-chart-rose-slices\">");
  let legendEntries = Buffer.Buffer<(Text, Text)>(n);
  var i : Nat = 0;
  while (i < n) {
    let v = vs[i];
    let startFrac = Float.fromInt(i) / Float.fromInt(n);
    let endFrac = Float.fromInt(i + 1) / Float.fromInt(n);
    let rr = roseRadius(v, denom, outerR);
    let color = palette(opts, i);
    let name = labelAt(labs, i);
    let tip = (if (name != "") { name # ": " } else { "" }) # fmtNum(v) # opts.unit;
    if (rr > 0.0) {
      b.add("<path class=\"mv-chart-rose-slice\" fill=\"" # esc(color)
        # "\" d=\"" # arcPath(cx, cy, rr, 0.0, startFrac, endFrac) # "\"><title>" # esc(tip) # "</title></path>");
    };
    legendEntries.add((if (name != "") { name } else { fmtNum(v) # opts.unit }, color));
    i += 1;
  };
  b.add("</g>");
  b.add(legend(opts, Buffer.toArray(legendEntries)));
  b.add(svgClose());
  Text.join("", b.vals());
};
// rose wedge radius: linear in value, clamped to [0, outerR].
func roseRadius(v : Float, denom : Float, outerR : Float) : Float {
  var fr = v / denom;
  if (fr < 0.0) { fr := 0.0 };
  if (fr > 1.0) { fr := 1.0 };
  outerR * fr;
};

  // ===== RadialHistogram =====
// ---- RadialHistogram (binned counts as polar bars) ----------------------
/// Bins raw observations into `bins` equal-width buckets (Sturges if bins<=0,
/// same model as Histogram) and draws each bucket as a polar bar: equal angular
/// width, radius proportional to the bucket COUNT. A circular axis of bin-edge
/// values when showAxes. Native <title> tooltip per bar with the [lo,hi) range
/// and count.
/// `<RadialHistogram values="1,2,2,3,3,3,4,4,5,6,6,7,8,9,9,10" bins="6" />`.
public func radialHistogram(values : Text, bins : Text, opts : O) : Text {
  let raw = parseFloats(values);
  let b = Buffer.Buffer<Text>(16);
  b.add(svgOpen(opts, "mv-chart-radhist"));
  let (cx, cy) = centerXY(opts);
  let maxR0 = minF(cx, cy) - 18.0;
  let outerMost = if (maxR0 < 8.0) { 8.0 } else { maxR0 };
  let n = raw.size();
  if (n == 0) {
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  let dataMin = arrMin(raw);
  let dataMax = arrMax(raw);
  let requested = switch (toFloat(bins)) { case (?f) { Int.abs(Float.toInt(f)) }; case null { 0 } };
  var nbins : Nat = if (requested > 0) { requested } else {
    let s = Float.log(Float.fromInt(n)) / Float.log(2.0);
    let c = Int.abs(Float.toInt(Float.ceil(s))) + 1;
    if (c < 1) { 1 } else { c };
  };
  if (nbins > 36) { nbins := 36 };
  if (nbins < 1) { nbins := 1 };
  let span = if (dataMax - dataMin <= 0.0) { 1.0 } else { dataMax - dataMin };
  let binW = span / Float.fromInt(nbins);
  let counts = radhCount(raw, dataMin, binW, nbins);
  var cmax : Nat = 0;
  for (c in counts.vals()) { if (c > cmax) { cmax := c } };
  let hi = switch (opts.yMax) { case (?m) { m }; case null { if (cmax == 0) { 1.0 } else { niceCeil(Float.fromInt(cmax)) } } };
  let denom = if (hi == 0.0) { 1.0 } else { hi };
  let innerHole = outerMost * 0.18;
  let radSpan = outerMost - innerHole;
  // faint count rings + circular outer guide
  if (opts.showGrid) {
    b.add("<g class=\"mv-chart-radhist-grid\">");
    let rings : Nat = 4;
    var r : Nat = 1;
    while (r <= rings) {
      let rr = innerHole + radSpan * Float.fromInt(r) / Float.fromInt(rings);
      b.add("<circle class=\"mv-chart-grid\" cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy) # "\" r=\"" # fmtNum(rr) # "\"/>");
      r += 1;
    };
    b.add("</g>");
  };
  b.add("<g class=\"mv-chart-radhist-bars\">");
  let color = palette(opts, 0);
  var i : Nat = 0;
  while (i < nbins) {
    let startFrac = Float.fromInt(i) / Float.fromInt(nbins);
    let endFrac = (Float.fromInt(i) + 0.86) / Float.fromInt(nbins); // small angular gap
    let cnt = counts[i];
    var fr = Float.fromInt(cnt) / denom;
    if (fr < 0.0) { fr := 0.0 };
    if (fr > 1.0) { fr := 1.0 };
    let rOuter = innerHole + radSpan * fr;
    let lo = dataMin + binW * Float.fromInt(i);
    let bhi = lo + binW;
    let tip = "[" # fmtNum(lo) # opts.unit # ", " # fmtNum(bhi) # opts.unit # "): " # Nat.toText(cnt);
    if (cnt > 0) {
      b.add("<path class=\"mv-chart-radhist-bar\" fill=\"" # esc(color)
        # "\" d=\"" # arcPath(cx, cy, rOuter, innerHole, startFrac, endFrac) # "\"><title>" # esc(tip) # "</title></path>");
    };
    // bin-edge labels around the rim
    if (opts.showAxes) {
      let ang = startFrac * 6.283185307179586;
      let lx = polarX(cx, outerMost + 10.0, ang);
      let ly = polarY(cy, outerMost + 10.0, ang);
      let anchor = if (lx > cx + 1.0) { "start" } else if (lx < cx - 1.0) { "end" } else { "middle" };
      b.add("<text class=\"mv-chart-tick mv-chart-radhist-edge\" x=\"" # fmtNum(lx) # "\" y=\"" # fmtNum(ly + 3.0)
        # "\" text-anchor=\"" # anchor # "\">" # esc(fmtNum(lo) # opts.unit) # "</text>");
    };
    i += 1;
  };
  b.add("</g>");
  b.add(svgClose());
  Text.join("", b.vals());
};
// count observations into nbins equal-width buckets (last bucket inclusive).
func radhCount(raw : [Float], dataMin : Float, binW : Float, nbins : Nat) : [var Nat] {
  let counts = Array.init<Nat>(nbins, 0);
  for (v in raw.vals()) {
    var idx = Int.abs(Float.toInt(Float.floor((v - dataMin) / binW)));
    if (idx >= nbins) { idx := nbins - 1 };
    counts[idx] += 1;
  };
  counts;
};

  // ===== ParallelCoordinates =====
// ---- ParallelCoordinates -------------------------------------------------
/// One vertical axis per dimension; each series becomes a polyline crossing
/// every axis at its value (each axis independently scaled to its own data
/// min..max). `labels` names the dimensions (= the axes, left to right);
/// `series` gives one named row of values per dimension.
/// `<ParallelCoordinates series="Car A:130,8.5,1450,42;Car B:90,12,1100,55"
///                       labels="HP,0-60s,Weight,MPG" />`.
public func parallelCoords(series : Text, labels : Text, opts : O) : Text {
  let rows = parseSeries(series);
  let dims = parseLabels(labels);
  let b = Buffer.Buffer<Text>(rows.size() * 2 + 12);
  b.add(svgOpen(opts, "mv-chart-parcoord"));
  // number of axes = max(label count, widest row)
  var nAxes : Nat = dims.size();
  for ((_, vals) in rows.vals()) { if (vals.size() > nAxes) { nAxes := vals.size() } };
  if (nAxes == 0 or rows.size() == 0) {
    let cx = Float.fromInt(Int.abs(opts.width)) / 2.0;
    let cy = Float.fromInt(Int.abs(opts.height)) / 2.0;
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  let left = plotLeft();
  let right = plotRight(opts);
  let top = plotTop();
  let bot = plotBottom(opts);
  // x position of each axis
  let axX = Array.tabulate<Float>(nAxes, func(d) {
    if (nAxes == 1) { (left + right) / 2.0 }
    else { left + (right - left) * Float.fromInt(d) / Float.fromInt(nAxes - 1) };
  });
  // per-axis (independent) min/max -> a y scale that maps value -> pixel
  let axLo = Array.init<Float>(nAxes, 0.0);
  let axHi = Array.init<Float>(nAxes, 1.0);
  var d : Nat = 0;
  while (d < nAxes) {
    let col = pcoordColumn(rows, d);
    var lo = if (col.size() > 0) { arrMin(col) } else { 0.0 };
    var hi = if (col.size() > 0) { arrMax(col) } else { 1.0 };
    if (hi <= lo) { hi := lo + 1.0 };
    axLo[d] := lo; axHi[d] := hi;
    d += 1;
  };
  // draw axes + per-axis min/max ticks + dimension labels
  if (opts.showAxes) {
    b.add("<g class=\"mv-chart-parcoord-axes\">");
    var a : Nat = 0;
    while (a < nAxes) {
      let x = axX[a];
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(x) # "\" y1=\"" # fmtNum(top)
        # "\" x2=\"" # fmtNum(x) # "\" y2=\"" # fmtNum(bot) # "\"/>");
      // top = axis max, bottom = axis min
      b.add("<text class=\"mv-chart-tick\" x=\"" # fmtNum(x + 4.0) # "\" y=\"" # fmtNum(top + 8.0)
        # "\" text-anchor=\"start\">" # esc(fmtNum(axHi[a]) # opts.unit) # "</text>");
      b.add("<text class=\"mv-chart-tick\" x=\"" # fmtNum(x + 4.0) # "\" y=\"" # fmtNum(bot - 2.0)
        # "\" text-anchor=\"start\">" # esc(fmtNum(axLo[a]) # opts.unit) # "</text>");
      let anchor = if (a == 0) { "start" } else if (a == nAxes - 1) { "end" } else { "middle" };
      b.add("<text class=\"mv-chart-tick mv-chart-xtick mv-chart-parcoord-dim\" x=\"" # fmtNum(x)
        # "\" y=\"" # fmtNum(bot + 16.0) # "\" text-anchor=\"" # anchor # "\">" # esc(labelAt(dims, a)) # "</text>");
      a += 1;
    };
    b.add("</g>");
  };
  // one polyline per series, crossing each axis at its scaled value
  b.add("<g class=\"mv-chart-parcoord-lines\">");
  let legendEntries = Buffer.Buffer<(Text, Text)>(rows.size());
  var si : Nat = 0;
  for ((name, vals) in rows.vals()) {
    let color = palette(opts, si);
    let pts = Buffer.Buffer<(Float, Float)>(nAxes);
    var a : Nat = 0;
    while (a < nAxes) {
      if (a < vals.size()) {
        let ySc = linScale(axLo[a], axHi[a], bot, top); // min at bottom, max at top
        pts.add((axX[a], ySc(vals[a])));
      };
      a += 1;
    };
    let ptsArr = Buffer.toArray(pts);
    let dispName = if (name != "") { name } else { "Series " # Nat.toText(si + 1) };
    b.add("<polyline class=\"mv-chart-parcoord-line\" fill=\"none\" stroke=\"" # esc(color)
      # "\" points=\"" # polyPoints(ptsArr) # "\"><title>" # esc(dispName) # "</title></polyline>");
    // vertex dots with per-dimension tooltips
    var k : Nat = 0;
    while (k < ptsArr.size()) {
      let (px, py) = ptsArr[k];
      let v = if (k < vals.size()) { vals[k] } else { 0.0 };
      let tip = dispName # " \u{b7} " # labelAt(dims, k) # ": " # fmtNum(v) # opts.unit;
      b.add("<circle class=\"mv-chart-parcoord-dot\" cx=\"" # fmtNum(px) # "\" cy=\"" # fmtNum(py)
        # "\" r=\"2.5\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
      k += 1;
    };
    legendEntries.add((dispName, color));
    si += 1;
  };
  b.add("</g>");
  b.add(legend(opts, Buffer.toArray(legendEntries)));
  b.add(svgClose());
  Text.join("", b.vals());
};
// gather every series' value at dimension index d (skipping rows too short).
func pcoordColumn(rows : [Series], d : Nat) : [Float] {
  let out = Buffer.Buffer<Float>(rows.size());
  for ((_, vals) in rows.vals()) { if (d < vals.size()) { out.add(vals[d]) } };
  Buffer.toArray(out);
};

  // ===== SmallMultiples =====
// ---- SmallMultiples (trellis grid of mini charts) -----------------------
/// A responsive grid of tiny charts, one cell per series, sharing a common y
/// scale (so cells are visually comparable). `kind` chooses the mark: "bar"
/// (default) draws mini columns, "line" draws a mini sparkline+area. `labels`
/// is the shared x category list (used only for tooltips). Each cell shows the
/// series name and has per-point native <title> tooltips.
/// `<SmallMultiples series="North:10,14,9,16;South:6,8,7,11;East:12,15,13,18"
///                  labels="Q1,Q2,Q3,Q4" kind="line" />`.
public func smallMultiples(series : Text, labels : Text, kind : Text, opts : O) : Text {
  let rows = parseSeries(series);
  let xlabs = parseLabels(labels);
  let ns = rows.size();
  let b = Buffer.Buffer<Text>(ns * 4 + 8);
  b.add(svgOpen(opts, "mv-chart-smallmult"));
  if (ns == 0) {
    let cx = Float.fromInt(Int.abs(opts.width)) / 2.0;
    let cy = Float.fromInt(Int.abs(opts.height)) / 2.0;
    b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy) # "\" text-anchor=\"middle\">No data</text>");
    b.add(svgClose());
    return Text.join("", b.vals());
  };
  let isLine = kind == "line" or kind == "Line" or kind == "spark";
  // shared y range across every series (comparable cells)
  let allVals = Buffer.Buffer<Float>(ns * 4);
  for ((_, vals) in rows.vals()) { for (v in vals.vals()) { allVals.add(v) } };
  let (ylo, yhi) = yDomain(opts, arrMin(Buffer.toArray(allVals)), arrMax(Buffer.toArray(allVals)), true);
  // grid geometry: pick a near-square column count
  let cols = smultCols(ns);
  let rowsN = (ns + cols - 1) / cols;
  let gridL = plotLeft() - 36.0; // mini charts need less left padding
  let gridR = plotRight(opts);
  let gridT = plotTop();
  let gridB = plotBottom(opts);
  let cellW = (gridR - gridL) / Float.fromInt(cols);
  let cellH = (gridB - gridT) / Float.fromInt(rowsN);
  let padIn : Float = 6.0;
  let labelH : Float = 14.0;
  var i : Nat = 0;
  for ((name, vals) in rows.vals()) {
    let cc = i % cols;
    let cr = i / cols;
    let cx0 = gridL + cellW * Float.fromInt(cc);
    let cy0 = gridT + cellH * Float.fromInt(cr);
    let innerL = cx0 + padIn;
    let innerR = cx0 + cellW - padIn;
    let innerT = cy0 + labelH;
    let innerB = cy0 + cellH - padIn;
    let color = palette(opts, i);
    b.add("<g class=\"mv-chart-smallmult-cell\">");
    // cell title
    let dispName = if (name != "") { name } else { "Series " # Nat.toText(i + 1) };
    b.add("<text class=\"mv-chart-smallmult-name\" x=\"" # fmtNum(cx0 + 2.0) # "\" y=\"" # fmtNum(cy0 + 10.0)
      # "\" text-anchor=\"start\">" # esc(dispName) # "</text>");
    let m = vals.size();
    if (m > 0) {
      let ySc = linScale(ylo, yhi, innerB, innerT);
      let baseY = ySc(if (ylo < 0.0) { 0.0 } else { ylo });
      if (isLine) {
        // mini area + line
        let pts = Array.tabulate<(Float, Float)>(m, func(j) {
          let x = if (m == 1) { (innerL + innerR) / 2.0 }
                  else { innerL + (innerR - innerL) * Float.fromInt(j) / Float.fromInt(m - 1) };
          (x, ySc(vals[j]));
        });
        b.add("<path class=\"mv-chart-smallmult-area\" fill=\"" # esc(color) # "\" d=\"" # areaPath(pts, baseY) # "\"/>");
        b.add("<polyline class=\"mv-chart-smallmult-line\" fill=\"none\" stroke=\"" # esc(color)
          # "\" points=\"" # polyPoints(pts) # "\"/>");
        var j : Nat = 0;
        while (j < m) {
          let (px, py) = pts[j];
          let tip = dispName # " \u{b7} " # labelAt(xlabs, j) # ": " # fmtNum(vals[j]) # opts.unit;
          b.add("<circle class=\"mv-chart-smallmult-dot\" cx=\"" # fmtNum(px) # "\" cy=\"" # fmtNum(py)
            # "\" r=\"2\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
          j += 1;
        };
      } else {
        // mini columns
        let band = (innerR - innerL) / Float.fromInt(m);
        let colW = band * 0.66;
        var j : Nat = 0;
        while (j < m) {
          let v = vals[j];
          let yv = ySc(v);
          let y0 = minF(baseY, yv);
          let h = Float.abs(yv - baseY);
          let bx = innerL + band * Float.fromInt(j) + (band - colW) / 2.0;
          let tip = dispName # " \u{b7} " # labelAt(xlabs, j) # ": " # fmtNum(v) # opts.unit;
          b.add("<rect class=\"mv-chart-smallmult-bar\" x=\"" # fmtNum(bx) # "\" y=\"" # fmtNum(y0)
            # "\" width=\"" # fmtNum(colW) # "\" height=\"" # fmtNum(h)
            # "\" rx=\"1\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></rect>");
          j += 1;
        };
      };
      // faint baseline per cell
      b.add("<line class=\"mv-chart-smallmult-base\" x1=\"" # fmtNum(innerL) # "\" y1=\"" # fmtNum(baseY)
        # "\" x2=\"" # fmtNum(innerR) # "\" y2=\"" # fmtNum(baseY) # "\"/>");
    };
    b.add("</g>");
    i += 1;
  };
  b.add(svgClose());
  Text.join("", b.vals());
};
// near-square column count for n cells (ceil(sqrt(n))).
func smultCols(n : Nat) : Nat {
  if (n <= 1) { return 1 };
  let r = sqrt_(Float.fromInt(n));
  var c = Int.abs(Float.toInt(Float.ceil(r)));
  if (c < 1) { c := 1 };
  Int.abs(c);
};

  // ===== CircularTreemap =====
  // ---- CircularTreemap: circle-packing of "label:value" items ---------------
  /// `<CircularTreemap data="Search:42;Direct:30;Social:18;Email:10" />`.
  /// Each item is a circle whose AREA is proportional to its value; circles are
  /// packed left-to-right / top-to-bottom into rows that fit the plot, biggest
  /// first. A simple, deterministic row-pack (no physics) keeps it pure & stable.
  public func circularTreemap(data : Text, opts : O) : Text {
    let pairs = parseSeries(data);
    let cpItems = Buffer.Buffer<(Text, Float)>(pairs.size());
    var cpTotal : Float = 0.0;
    for ((nm, vs) in pairs.vals()) {
      let v = if (vs.size() > 0 and vs[0] > 0.0) { vs[0] } else { 0.0 };
      if (v > 0.0) { cpItems.add((nm, v)); cpTotal += v };
    };
    let b = Buffer.Buffer<Text>(cpItems.size() * 2 + 6);
    b.add(svgOpen(opts, "mv-chart-circpack"));
    let arr = Buffer.toArray(cpItems);
    if (arr.size() == 0 or cpTotal <= 0.0) {
      let (ecx, ecy) = centerXY(opts);
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ecx) # "\" y=\"" # fmtNum(ecy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    // sort descending by value (insertion sort over an index array; small N).
    let order = Array.init<Nat>(arr.size(), 0);
    var z : Nat = 0; while (z < arr.size()) { order[z] := z; z += 1 };
    var a0 : Nat = 1;
    while (a0 < arr.size()) {
      let key = order[a0];
      var jj : Int = a0 - 1;
      while (jj >= 0 and arr[order[Int.abs(jj)]].1 < arr[key].1) {
        order[Int.abs(jj) + 1] := order[Int.abs(jj)];
        jj -= 1;
      };
      order[Int.abs(jj) + 1] := key;
      a0 += 1;
    };
    // plot rect.
    let x0 = plotLeft() - 28.0;
    let y0 = plotTop();
    let x1 = plotRight(opts);
    let y1 = if (opts.showLegend) { plotBottom(opts) + 6.0 } else { plotBottom(opts) + 14.0 };
    let plotW = x1 - x0;
    let plotH = y1 - y0;
    // Choose a radius scale so the largest circle is a sensible fraction of the
    // plot and total circle area ~ fills it. r = k * sqrt(value). Pick k by
    // matching the biggest item's diameter to a cap of the short side.
    let vmax = arr[order[0]].1;
    let shortSide = minF(plotW, plotH);
    let kCap = (shortSide * 0.44) / sqrt_(vmax);   // biggest circle ~ 0.44*short
    // area-fit k: sum(pi r^2) = packing*plotArea  =>  k = sqrt(pack*W*H/(pi*sumV))
    let pack : Float = 0.62;
    let kArea = sqrt_(pack * plotW * plotH / (3.141592653589793 * cpTotal));
    let cpGap : Float = 4.0;
    // shrink k until the row-pack fits the plot HEIGHT (pure, bounded loop).
    var k = minF(kCap, kArea);
    var guard : Nat = 0;
    while (guard < 40 and cpPackedHeight(arr, order, k, x0, x1, cpGap) > plotH) {
      k := k * 0.9;
      guard += 1;
    };
    // greedy row packing: walk items biggest-first, place along a row until the
    // next circle would overflow plotW, then drop to a new row.
    var penX = x0;
    var rowTop = y0;
    var rowMaxD : Float = 0.0;
    b.add("<g class=\"mv-chart-circpack-circles\">");
    var i : Nat = 0;
    while (i < arr.size()) {
      let oi = order[i];
      let (nm, v) = arr[oi];
      let r = maxF(cpCircleR(v, k), 3.0);
      let d = r * 2.0;
      // wrap to a new row if this circle would exceed the right edge.
      if (penX + d > x1 + 0.5 and penX > x0 + 0.5) {
        rowTop += rowMaxD + cpGap;
        penX := x0;
        rowMaxD := 0.0;
      };
      let cx = penX + r;
      let cy = rowTop + r;
      let color = palette(opts, oi);
      let pct = v / cpTotal * 100.0;
      let tip = (if (nm != "") { nm # ": " } else { "" }) # fmtNum(v) # opts.unit # " (" # fmtNum(pct) # "%)";
      b.add("<g class=\"mv-chart-circpack-node\">");
      b.add("<circle cx=\"" # fmtNum(cx) # "\" cy=\"" # fmtNum(cy)
        # "\" r=\"" # fmtNum(r) # "\" fill=\"" # esc(color) # "\"><title>" # esc(tip) # "</title></circle>");
      // inline label when the circle is large enough.
      if (r > 18.0 and nm != "") {
        b.add("<text class=\"mv-chart-circpack-label\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy + 1.0) # "\" text-anchor=\"middle\">" # esc(nm) # "</text>");
        if (r > 30.0) {
          b.add("<text class=\"mv-chart-circpack-val\" x=\"" # fmtNum(cx) # "\" y=\"" # fmtNum(cy + 14.0) # "\" text-anchor=\"middle\">" # esc(fmtNum(v) # opts.unit) # "</text>");
        };
      };
      b.add("</g>");
      penX += d + cpGap;
      if (d > rowMaxD) { rowMaxD := d };
      i += 1;
    };
    b.add("</g>");
    let entries = Array.tabulate<(Text, Text)>(arr.size(), func(j) {
      let (nm, _) = arr[j]; (if (nm != "") { nm } else { "Item " # Nat.toText(j + 1) }, palette(opts, j));
    });
    b.add(legend(opts, entries));
    b.add(svgClose());
    Text.join("", b.vals());
  };
  // area-proportional radius for a value (r = k*sqrt(v)).
  func cpCircleR(v : Float, k : Float) : Float { if (v <= 0.0) { 0.0 } else { k * sqrt_(v) } };
  // Simulate the greedy row pack at scale k; return total stacked height so the
  // caller can shrink k until everything fits vertically.
  func cpPackedHeight(arr : [(Text, Float)], order : [var Nat], k : Float, x0 : Float, x1 : Float, gap : Float) : Float {
    var penX = x0;
    var rowTop : Float = 0.0;
    var rowMaxD : Float = 0.0;
    var i : Nat = 0;
    while (i < arr.size()) {
      let v = arr[order[i]].1;
      let r = maxF(cpCircleR(v, k), 3.0);
      let d = r * 2.0;
      if (penX + d > x1 + 0.5 and penX > x0 + 0.5) {
        rowTop += rowMaxD + gap;
        penX := x0;
        rowMaxD := 0.0;
      };
      penX += d + gap;
      if (d > rowMaxD) { rowMaxD := d };
      i += 1;
    };
    rowTop + rowMaxD; // last row's height included
  };

  // ===== HorizonChart =====
  // ---- HorizonChart: banded folded area (compact time series) ---------------
  /// `<HorizonChart values="3,7,5,9,4,8,12,6,10,2" labels=".." />`.
  /// The series is sliced into `bands` equal value-bands; each band is drawn as
  /// a filled area in an increasingly saturated tint, all collapsed onto one
  /// short lane so the silhouette reads like a folded area chart. Negative
  /// values fold UP in a contrasting hue. Fixed at 3 bands for readability.
  public func horizon(valuesCsv : Text, labelsCsv : Text, opts : O) : Text {
    let vals = parseFloats(valuesCsv);
    let n = vals.size();
    if (n == 0) { return svgOpen(opts, "mv-chart-horizon") # svgClose() };
    let labels = parseLabels(labelsCsv);

    let dmax = maxF(arrMax(vals), 0.0);
    let dmin = minF(arrMin(vals), 0.0);
    let posPeak = maxF(dmax, 0.0001);
    let negPeak = maxF(-dmin, 0.0);
    let bands : Nat = 3;                      // fixed, readable banding
    let bandStep = posPeak / Float.fromInt(bands);

    // one short lane occupying the plot height.
    let laneTop = plotTop() + 4.0;
    let laneBot = plotBottom(opts);
    let centers = horizonCenters(opts, n);
    let baseColor = palette(opts, 0);
    let negColor = palette(opts, 2);

    let b = Buffer.Buffer<Text>(n + bands + 8);
    b.add(svgOpen(opts, "mv-chart-horizon"));
    // baseline + bottom labels.
    if (opts.showAxes) {
      b.add("<line class=\"mv-chart-axis\" x1=\"" # fmtNum(plotLeft()) # "\" y1=\"" # fmtNum(laneBot)
        # "\" x2=\"" # fmtNum(plotRight(opts)) # "\" y2=\"" # fmtNum(laneBot) # "\"/>");
    };
    if (labels.size() > 0) { b.add(axisBottom(opts, labels, centers)) };

    // POSITIVE bands: band j shows the portion of the value above j*step, each
    // mapped to the FULL lane height and stacked by opacity (classic horizon).
    var j : Nat = 0;
    while (j < bands) {
      let lo = bandStep * Float.fromInt(j);
      // a per-band scale: value `lo`..`lo+step` -> laneBot..laneTop.
      let ySc = linScale(lo, lo + bandStep, laneBot, laneTop);
      let pts = Buffer.Buffer<(Float, Float)>(n);
      var i : Nat = 0;
      while (i < n) {
        let raw = if (vals[i] > 0.0) { vals[i] } else { 0.0 };
        let clamped = if (raw < lo) { lo } else if (raw > lo + bandStep) { lo + bandStep } else { raw };
        pts.add((centers[i], ySc(clamped)));
        i += 1;
      };
      let ptsArr = Buffer.toArray(pts);
      let d = areaPath(ptsArr, laneBot);
      // deepest band = strongest opacity.
      let op = horizonOpacity(j, bands);
      b.add("<path class=\"mv-chart-horizon-band\" d=\"" # d # "\" fill=\"" # esc(baseColor)
        # "\" fill-opacity=\"" # fmtNum(op) # "\"/>");
      j += 1;
    };
    // NEGATIVE fold (single contrasting band) if there is any negative data.
    if (negPeak > 0.0) {
      let ySc = linScale(0.0, negPeak, laneBot, laneTop);
      let pts = Buffer.Buffer<(Float, Float)>(n);
      var i : Nat = 0;
      while (i < n) {
        let mag = if (vals[i] < 0.0) { -vals[i] } else { 0.0 };
        pts.add((centers[i], ySc(mag)));
        i += 1;
      };
      let d = areaPath(Buffer.toArray(pts), laneBot);
      b.add("<path class=\"mv-chart-horizon-band mv-chart-horizon-neg\" d=\"" # d # "\" fill=\"" # esc(negColor)
        # "\" fill-opacity=\"0.85\"/>");
    };
    // hover points carrying the true value (geometry is otherwise lossy).
    var hi : Nat = 0;
    while (hi < n) {
      let lbl = labelAt(labels, hi);
      let tip = (if (lbl != "") { lbl # ": " } else { "" }) # fmtNum(vals[hi]) # opts.unit;
      b.add("<circle class=\"mv-chart-horizon-pt\" cx=\"" # fmtNum(centers[hi]) # "\" cy=\"" # fmtNum(laneBot - 4.0)
        # "\" r=\"6\" fill=\"transparent\"><title>" # esc(tip) # "</title></circle>");
      hi += 1;
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };
  // even x centers across the plot (single point centered).
  func horizonCenters(opts : O, n : Nat) : [Float] {
    let x0 = plotLeft(); let x1 = plotRight(opts);
    if (n == 0) { return [] };
    if (n == 1) { return [(x0 + x1) / 2.0] };
    let step = (x1 - x0) / Float.fromInt(n - 1);
    Array.tabulate<Float>(n, func(i) { x0 + step * Float.fromInt(i) });
  };
  // opacity ramp: band 0 faintest .. last band fullest.
  func horizonOpacity(j : Nat, bands : Nat) : Float {
    let denom = if (bands == 0) { 1.0 } else { Float.fromInt(bands) };
    0.30 + 0.70 * (Float.fromInt(j + 1) / denom);
  };

  // ===== BumpAreaChart =====
  // ---- BumpAreaChart: stacked ranking areas (streamgraph-style) -------------
  /// `<BumpAreaChart series="Alice:30,40,38,50;Bob:20,25,35,30;Carol:10,15,12,22"
  ///                 labels="W1,W2,W3,W4" />`. Each series' VALUE (a magnitude,
  /// e.g. votes/score) is stacked into a smooth band; band order is re-sorted at
  /// the first time slot so the largest sits at the bottom — a "bump"-flavoured
  /// stacked area that emphasises shifting share over time.
  public func bumpArea(spec : Text, labelsCsv : Text, opts : O) : Text {
    let ss = parseSeries(spec);
    let ns = ss.size();
    if (ns == 0) { return svgOpen(opts, "mv-chart-bumparea") # svgClose() };
    var n : Nat = 0;
    for ((_, vs) in ss.vals()) { if (vs.size() > n) { n := vs.size() } };
    if (n == 0) { return svgOpen(opts, "mv-chart-bumparea") # svgClose() };
    let labels = parseLabels(labelsCsv);

    // per-x stacked total -> y domain (0..max total).
    let totals = Array.init<Float>(n, 0.0);
    var s0 : Nat = 0;
    while (s0 < ns) {
      let (_, vs) = ss[s0];
      var i : Nat = 0;
      while (i < n) { let v = if (i < vs.size() and vs[i] > 0.0) { vs[i] } else { 0.0 }; totals[i] += v; i += 1 };
      s0 += 1;
    };
    var maxTotal : Float = 0.0;
    for (t in totals.vals()) { if (t > maxTotal) { maxTotal := t } };
    if (maxTotal <= 0.0) { maxTotal := 1.0 };
    let ySc = linScale(0.0, maxTotal, plotBottom(opts), plotTop());
    let centers = baCenters(opts, n);

    // draw order: sort series by their FIRST-slot value descending so the
    // biggest band anchors the bottom of the stack (bump emphasis).
    let baOrder = Array.init<Nat>(ns, 0);
    var z : Nat = 0; while (z < ns) { baOrder[z] := z; z += 1 };
    var a0 : Nat = 1;
    while (a0 < ns) {
      let key = baOrder[a0];
      var jj : Int = a0 - 1;
      while (jj >= 0 and baFirst(ss[baOrder[Int.abs(jj)]]) < baFirst(ss[key])) {
        baOrder[Int.abs(jj) + 1] := baOrder[Int.abs(jj)];
        jj -= 1;
      };
      baOrder[Int.abs(jj) + 1] := key;
      a0 += 1;
    };

    let b = Buffer.Buffer<Text>(ns * 2 + 8);
    b.add(svgOpen(opts, "mv-chart-bumparea"));
    if (labels.size() > 0) { b.add(axisBottom(opts, labels, centers)) };

    // running stacked baseline per x (value units, from 0 up).
    let running = Array.init<Float>(n, 0.0);
    var oi : Nat = 0;
    while (oi < ns) {
      let si = baOrder[oi];
      let (name0, vs) = ss[si];
      let color = palette(opts, si);
      let top = Buffer.Buffer<(Float, Float)>(n);
      let bot = Buffer.Buffer<(Float, Float)>(n);
      var i : Nat = 0;
      while (i < n) {
        let v = if (i < vs.size() and vs[i] > 0.0) { vs[i] } else { 0.0 };
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
        // smooth top forward, smooth bottom backward, close into one band.
        let topD = baSmooth(topArr, true);
        let revB = Buffer.Buffer<(Float, Float)>(botArr.size());
        var jr : Int = botArr.size() - 1;
        while (jr >= 0) { revB.add(botArr[Int.abs(jr)]); jr -= 1 };
        let botD = baSmooth(Buffer.toArray(revB), false);
        let d = topD # " " # botD # " Z";
        let nm = if (name0 == "") { "Series " # Nat.toText(si + 1) } else { name0 };
        // tooltip reports first->last for a quick trend read.
        let firstV = if (vs.size() > 0) { vs[0] } else { 0.0 };
        let lastV = if (vs.size() > 0) { vs[vs.size() - 1] } else { 0.0 };
        let tip = nm # ": " # fmtNum(firstV) # opts.unit # " \u{2192} " # fmtNum(lastV) # opts.unit;
        b.add("<path class=\"mv-chart-bumparea-band\" d=\"" # d # "\" fill=\"" # esc(color)
          # "\"><title>" # esc(tip) # "</title></path>");
      };
      oi += 1;
    };
    let entries = Array.tabulate<(Text, Text)>(ns, func(i) {
      ((if (ss[i].0 == "") { "Series " # Nat.toText(i + 1) } else { ss[i].0 }), palette(opts, i));
    });
    b.add(legend(opts, entries));
    b.add(svgClose());
    Text.join("", b.vals());
  };
  func baCenters(opts : O, n : Nat) : [Float] {
    let x0 = plotLeft(); let x1 = plotRight(opts);
    if (n == 0) { return [] };
    if (n == 1) { return [(x0 + x1) / 2.0] };
    let step = (x1 - x0) / Float.fromInt(n - 1);
    Array.tabulate<Float>(n, func(i) { x0 + step * Float.fromInt(i) });
  };
  func baFirst(s : Series) : Float { let (_, vs) = s; if (vs.size() > 0) { vs[0] } else { 0.0 } };
  // Catmull-Rom smoothing; lead=true starts with M (top edge), false with L
  // (continues into the reversed bottom edge so the band stays one shape).
  func baSmooth(pts : [(Float, Float)], lead : Bool) : Text {
    let n = pts.size();
    if (n == 0) { return "" };
    let (x0, y0) = pts[0];
    let head = if (lead) { "M " } else { "L " };
    if (n < 3) {
      let b = Buffer.Buffer<Text>(n + 1);
      b.add(head # fmtNum(x0) # "," # fmtNum(y0));
      var i : Nat = 1;
      while (i < n) { let (x, y) = pts[i]; b.add("L " # fmtNum(x) # "," # fmtNum(y)); i += 1 };
      return Text.join(" ", b.vals());
    };
    let b = Buffer.Buffer<Text>(n + 1);
    b.add(head # fmtNum(x0) # "," # fmtNum(y0));
    var i : Nat = 0;
    while (i + 1 < n) {
      let p0 = if (i == 0) { pts[0] } else { pts[i - 1] };
      let p1 = pts[i]; let p2 = pts[i + 1];
      let p3 = if (i + 2 < n) { pts[i + 2] } else { pts[i + 1] };
      let (p0x, p0y) = p0; let (p1x, p1y) = p1;
      let (p2x, p2y) = p2; let (p3x, p3y) = p3;
      let c1x = p1x + (p2x - p0x) / 6.0; let c1y = p1y + (p2y - p0y) / 6.0;
      let c2x = p2x - (p3x - p1x) / 6.0; let c2y = p2y - (p3y - p1y) / 6.0;
      b.add("C " # fmtNum(c1x) # "," # fmtNum(c1y) # " "
        # fmtNum(c2x) # "," # fmtNum(c2y) # " " # fmtNum(p2x) # "," # fmtNum(p2y));
      i += 1;
    };
    Text.join(" ", b.vals());
  };

  // ===== WordCloud =====
  // ---- WordCloud: weighted words laid out in rows -------------------------
  /// `<WordCloud words="motoko:40;canister:32;icp:28;wasm:20;ui:12" />`.
  /// Each "word:weight" becomes a <text> whose font-size is proportional to its
  /// weight; words are flowed into centered rows (greedy line-wrap) and colored
  /// from the categorical palette. Heaviest words first.
  public func wordCloud(words : Text, opts : O) : Text {
    let pairs = parseSeries(words);
    let wcItems = Buffer.Buffer<(Text, Float)>(pairs.size());
    for ((nm, vs) in pairs.vals()) {
      let w = if (vs.size() > 0 and vs[0] > 0.0) { vs[0] } else { 0.0 };
      if (nm != "" and w > 0.0) { wcItems.add((nm, w)) };
    };
    let b = Buffer.Buffer<Text>(wcItems.size() + 6);
    b.add(svgOpen(opts, "mv-chart-wordcloud"));
    let arr = Buffer.toArray(wcItems);
    if (arr.size() == 0) {
      let (ecx, ecy) = centerXY(opts);
      b.add("<text class=\"mv-chart-empty\" x=\"" # fmtNum(ecx) # "\" y=\"" # fmtNum(ecy) # "\" text-anchor=\"middle\">No data</text>");
      b.add(svgClose());
      return Text.join("", b.vals());
    };
    // sort descending by weight.
    let order = Array.init<Nat>(arr.size(), 0);
    var z : Nat = 0; while (z < arr.size()) { order[z] := z; z += 1 };
    var a0 : Nat = 1;
    while (a0 < arr.size()) {
      let key = order[a0];
      var jj : Int = a0 - 1;
      while (jj >= 0 and arr[order[Int.abs(jj)]].1 < arr[key].1) {
        order[Int.abs(jj) + 1] := order[Int.abs(jj)];
        jj -= 1;
      };
      order[Int.abs(jj) + 1] := key;
      a0 += 1;
    };
    let wmax = arr[order[0]].1;
    var wmin = wmax;
    for ((_, w) in arr.vals()) { if (w < wmin) { wmin := w } };
    let x0 = plotLeft() - 28.0;
    let x1 = plotRight(opts);
    let plotW = x1 - x0;
    let yTop = plotTop() + 6.0;
    let yBot = plotBottom(opts) + 14.0;
    // font-size range scaled to the plot.
    let fsMin : Float = 12.0;
    let fsMax : Float = maxF(20.0, minF(48.0, (yBot - yTop) * 0.30));
    func wcFont(w : Float) : Float {
      if (wmax <= wmin) { (fsMin + fsMax) / 2.0 }
      else { fsMin + (w - wmin) / (wmax - wmin) * (fsMax - fsMin) };
    };
    // estimate a word's rendered width (~0.58em per char + padding).
    func wcWidth(word : Text, fs : Float) : Float { Float.fromInt(word.size()) * fs * 0.58 + fs * 0.5 };

    // greedy line-wrap into rows, centering each row; gap of 10px between words.
    let wcWordGap : Float = 10.0;
    let rowGap : Float = 8.0;
    var rowY = yTop;
    var i : Nat = 0;
    b.add("<g class=\"mv-chart-wordcloud-words\">");
    while (i < arr.size()) {
      // 1) find rowEnd: grow the row while the next word still fits in plotW.
      var rowEnd = i;          // exclusive end of this row [i, rowEnd)
      var rowW : Float = 0.0;  // accumulated width of placed words + gaps
      var rowMaxFs : Float = 0.0;
      var fitting = true;
      while (rowEnd < arr.size() and fitting) {
        let (word, w) = arr[order[rowEnd]];
        let fs = wcFont(w);
        let ww = wcWidth(word, fs);
        let add = ww + (if (rowEnd > i) { wcWordGap } else { 0.0 });
        if (rowEnd > i and rowW + add > plotW) {
          fitting := false;      // next word overflows -> close the row
        } else {
          rowW += add;
          if (fs > rowMaxFs) { rowMaxFs := fs };
          rowEnd += 1;
        };
      };
      // 2) place this row centered on the lane.
      var penX = x0 + maxF(plotW - rowW, 0.0) / 2.0;
      let baseY = rowY + rowMaxFs * 0.8;
      var r = i;
      while (r < rowEnd) {
        let oi = order[r];
        let (word, w) = arr[oi];
        let fs = wcFont(w);
        let color = palette(opts, oi);
        let ww = wcWidth(word, fs);
        b.add("<text class=\"mv-chart-wordcloud-word\" x=\"" # fmtNum(penX + ww / 2.0) # "\" y=\"" # fmtNum(baseY)
          # "\" text-anchor=\"middle\" font-size=\"" # fmtNum(fs) # "\" fill=\"" # esc(color) # "\">"
          # esc(word) # "<title>" # esc(word # ": " # fmtNum(w) # opts.unit) # "</title></text>");
        penX += ww + wcWordGap;
        r += 1;
      };
      rowY += rowMaxFs + rowGap;
      i := rowEnd;
    };
    b.add("</g>");
    b.add(svgClose());
    Text.join("", b.vals());
  };

  // ===== MatrixChart =====
  // ---- MatrixChart: square value glyphs sized + shaded by magnitude --------
  /// A row-major numeric matrix (same data convention as Heatmap) drawn as a
  /// grid of SQUARE glyphs whose SIZE and SHADE both encode magnitude.
  ///   matrix="12,30,5;8,22,40;3,9,18"  rowLabels="A,B,C"  colLabels="Q1,Q2,Q3"
  /// Larger/darker square = bigger value. Each glyph carries a <title> tooltip.
  public func matrix(matrixSpec : Text, rowLabels : Text, colLabels : Text, opts : O) : Text {
    let rows = mxRows(matrixSpec);
    let nr = rows.size();
    if (nr == 0) { return svgOpen(opts, "mv-chart-matrix") # svgClose() };
    var nc : Nat = 0;
    for (r in rows.vals()) { if (r.size() > nc) { nc := r.size() } };
    if (nc == 0) { return svgOpen(opts, "mv-chart-matrix") # svgClose() };

    let rLabs = parseLabels(rowLabels);
    let cLabs = parseLabels(colLabels);

    let flat = Buffer.Buffer<Float>(nr * nc);
    for (r in rows.vals()) { for (v in r.vals()) { flat.add(v) } };
    let allv = Buffer.toArray(flat);
    let vlo = switch (opts.yMin) { case (?m) { m }; case null { minF(arrMin(allv), 0.0) } };
    let vhiRaw = switch (opts.yMax) { case (?m) { m }; case null { arrMax(allv) } };
    let vhi = if (vhiRaw <= vlo) { vlo + 1.0 } else { vhiRaw };

    let left = if (rLabs.size() > 0) { plotLeft() } else { plotLeft() - 28.0 };
    let right = plotRight(opts);
    let top = plotTop() + (if (cLabs.size() > 0) { 8.0 } else { 0.0 });
    let bot = plotBottom(opts) + 16.0;
    let cw = (right - left) / Float.fromInt(nc);
    let ch = (bot - top) / Float.fromInt(nr);
    let cell = minF(cw, ch);
    let baseColor = palette(opts, 0);

    let b = Buffer.Buffer<Text>(nr * nc + nr + nc + 8);
    b.add(svgOpen(opts, "mv-chart-matrix"));

    // column labels along the top.
    if (cLabs.size() > 0) {
      var c : Nat = 0;
      while (c < nc) {
        let cx = left + cw * (Float.fromInt(c) + 0.5);
        b.add("<text class=\"mv-chart-tick mv-chart-xtick\" x=\"" # fmtNum(cx)
          # "\" y=\"" # fmtNum(top - 4.0) # "\" text-anchor=\"middle\">"
          # esc(labelAt(cLabs, c)) # "</text>");
        c += 1;
      };
    };

    b.add("<g class=\"mv-chart-matrix-cells\">");
    var ri : Nat = 0;
    while (ri < nr) {
      let row = rows[ri];
      let cyCell = top + ch * Float.fromInt(ri);
      if (rLabs.size() > 0) {
        b.add("<text class=\"mv-chart-tick mv-chart-ytick\" x=\"" # fmtNum(left - 6.0)
          # "\" y=\"" # fmtNum(cyCell + ch / 2.0 + 4.0) # "\" text-anchor=\"end\">"
          # esc(labelAt(rLabs, ri)) # "</text>");
      };
      var ci : Nat = 0;
      while (ci < nc) {
        let cxCell = left + cw * Float.fromInt(ci);
        if (ci < row.size()) {
          let v = row[ci];
          var frac = (v - vlo) / (vhi - vlo);
          if (frac < 0.0) { frac := 0.0 };
          if (frac > 1.0) { frac := 1.0 };
          // glyph size: 30%..96% of the cell square, area-ish via sqrt(frac).
          let sz = cell * (0.30 + 0.66 * sqrt_(frac));
          let gx = cxCell + (cw - sz) / 2.0;
          let gy = cyCell + (ch - sz) / 2.0;
          let fill = mxBlend(baseColor, frac);
          let rl = labelAt(rLabs, ri);
          let cl = labelAt(cLabs, ci);
          let where = if (rl != "" or cl != "") {
            (if (rl != "") { rl } else { "r" # Nat.toText(ri + 1) }) # " / "
              # (if (cl != "") { cl } else { "c" # Nat.toText(ci + 1) }) # ": "
          } else { "" };
          b.add("<rect class=\"mv-chart-matrix-cell\" x=\"" # fmtNum(gx) # "\" y=\"" # fmtNum(gy)
            # "\" width=\"" # fmtNum(maxF(sz, 1.0)) # "\" height=\"" # fmtNum(maxF(sz, 1.0))
            # "\" rx=\"2\" fill=\"" # esc(fill) # "\">"
            # "<title>" # esc(where # fmtNum(v) # opts.unit) # "</title></rect>");
        };
        ci += 1;
      };
      ri += 1;
    };
    b.add("</g>");

    // min..max color legend bar (mirrors Heatmap).
    if (opts.showLegend) {
      let lw : Float = 120.0; let lh : Float = 8.0;
      let lx = left; let ly = Float.fromInt(Int.abs(opts.height)) - 12.0;
      let steps : Nat = 12;
      var k : Nat = 0;
      b.add("<g class=\"mv-chart-legend mv-chart-matrix-legend\">");
      while (k < steps) {
        let frac = Float.fromInt(k) / Float.fromInt(steps - 1);
        let sx = lx + lw * Float.fromInt(k) / Float.fromInt(steps);
        b.add("<rect class=\"mv-chart-matrix-swatch\" x=\"" # fmtNum(sx) # "\" y=\"" # fmtNum(ly - lh)
          # "\" width=\"" # fmtNum(lw / Float.fromInt(steps) + 0.5) # "\" height=\"" # fmtNum(lh)
          # "\" fill=\"" # esc(mxBlend(baseColor, frac)) # "\"/>");
        k += 1;
      };
      b.add("<text class=\"mv-chart-legend-label\" x=\"" # fmtNum(lx - 4.0) # "\" y=\"" # fmtNum(ly)
        # "\" text-anchor=\"end\">" # esc(fmtNum(vlo) # opts.unit) # "</text>");
      b.add("<text class=\"mv-chart-legend-label\" x=\"" # fmtNum(lx + lw + 4.0) # "\" y=\"" # fmtNum(ly)
        # "\" text-anchor=\"start\">" # esc(fmtNum(vhi) # opts.unit) # "</text>");
      b.add("</g>");
    };
    b.add(svgClose());
    Text.join("", b.vals());
  };
  // Parse a ';'-rowed, ','-celled numeric matrix.
  func mxRows(spec : Text) : [[Float]] {
    let out = Buffer.Buffer<[Float]>(8);
    for (seg in Text.split(spec, #char ';')) {
      let s = Text.trimStart(Text.trimEnd(seg, #char ' '), #char ' ');
      if (s != "") { out.add(parseFloats(s)) };
    };
    Buffer.toArray(out);
  };
  // white -> baseColor blend by frac (own copy; namespaced to avoid collision
  // with heatBlend/heatHex which are private to the Heatmap section).
  func mxBlend(baseColor : Text, frac : Float) : Text {
    let cs = Text.toArray(baseColor);
    var r : Float = 15.0; var g : Float = 108.0; var bl : Float = 189.0;
    if (cs.size() >= 7 and cs[0] == '#') {
      let hv = func(c : Char) : Float {
        let nn : Nat32 = Char.toNat32(c);
        if (nn >= 48 and nn <= 57) { Float.fromInt(Nat32.toNat(nn - 48)) }
        else if (nn >= 97 and nn <= 102) { Float.fromInt(Nat32.toNat(nn - 87)) }
        else if (nn >= 65 and nn <= 70) { Float.fromInt(Nat32.toNat(nn - 55)) }
        else { 0.0 };
      };
      r := hv(cs[1]) * 16.0 + hv(cs[2]);
      g := hv(cs[3]) * 16.0 + hv(cs[4]);
      bl := hv(cs[5]) * 16.0 + hv(cs[6]);
    };
    let t = 0.15 + frac * 0.85;
    let mix = func(ch : Float) : Nat {
      let v = 255.0 + (ch - 255.0) * t;
      let iv = Float.toInt(v + 0.5);
      if (iv < 0) { 0 } else if (iv > 255) { 255 } else { Int.abs(iv) };
    };
    "rgb(" # Nat.toText(mix(r)) # "," # Nat.toText(mix(g)) # "," # Nat.toText(mix(bl)) # ")";
  };

  // ===== TableChart =====
  // ---- TableChart: a clean HTML data table (NOT svg) -----------------------
  /// `<TableChart values="10,20;5,8;3,9" rowLabels="Q1,Q2,Q3" colLabels="Sales,Costs" />`.
  /// `values` is a ';'-rowed, ','-celled matrix; each row gets a rowLabel (left
  /// header column) and each column a colLabel (top header row). Returns styled
  /// HTML `<table class="mv-chart-table">` (NOT an SVG) so it flows as normal
  /// document content. opts.title -> <caption>; opts.unit suffixes each value.
  public func table(values : Text, rowLabels : Text, colLabels : Text, opts : O) : Text {
    let rows = mxRows(values);  // shares MatrixChart's row parser
    let nr = rows.size();
    let rLabs = parseLabels(rowLabels);
    let cLabs = parseLabels(colLabels);
    var nc : Nat = 0;
    for (r in rows.vals()) { if (r.size() > nc) { nc := r.size() } };
    if (nc < cLabs.size()) { nc := cLabs.size() };

    let b = Buffer.Buffer<Text>(nr * nc + nr + 8);
    b.add("<table class=\"mv-chart-table\">");
    if (opts.title != "") {
      b.add("<caption class=\"mv-chart-table-caption\">" # esc(opts.title) # "</caption>");
    };
    if (nr == 0 and nc == 0) {
      b.add("<tbody><tr><td class=\"mv-chart-table-empty\">No data</td></tr></tbody>");
      b.add("</table>");
      return Text.join("", b.vals());
    };
    // header row: a corner cell (if there are row labels) + column labels.
    let hasRowHead = rLabs.size() > 0;
    if (cLabs.size() > 0) {
      b.add("<thead><tr>");
      if (hasRowHead) { b.add("<th class=\"mv-chart-table-corner\" scope=\"col\"></th>") };
      var c : Nat = 0;
      while (c < nc) {
        b.add("<th class=\"mv-chart-table-colhead\" scope=\"col\">" # esc(labelAt(cLabs, c)) # "</th>");
        c += 1;
      };
      b.add("</tr></thead>");
    };
    b.add("<tbody>");
    var ri : Nat = 0;
    while (ri < nr) {
      let row = rows[ri];
      b.add("<tr>");
      if (hasRowHead) {
        b.add("<th class=\"mv-chart-table-rowhead\" scope=\"row\">" # esc(labelAt(rLabs, ri)) # "</th>");
      };
      var ci : Nat = 0;
      while (ci < nc) {
        let cellTxt = if (ci < row.size()) { fmtNum(row[ci]) # opts.unit } else { "" };
        b.add("<td class=\"mv-chart-table-cell\">" # esc(cellTxt) # "</td>");
        ci += 1;
      };
      b.add("</tr>");
      ri += 1;
    };
    b.add("</tbody>");
    b.add("</table>");
    Text.join("", b.vals());
  };
  // NOTE: TableChart reuses mxRows (defined in the MatrixChart section). Keep
  // the two sections adjacent so mxRows is in scope, or move mxRows up to the
  // shared-helper area if MatrixChart is dropped.

};
