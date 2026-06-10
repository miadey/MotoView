//
//  Theme.swift — wires the Slice 7 BrandTokens into the native renderer.
//
//  The design-token cross-compiler (Slice 7) emits `BrandTokens.swift` (the
//  Fluent brand ramp) at `<app>/.mvbuild/native/BrandTokens.swift`. That file is
//  vendored here as `BrandTokens.swift`. This file is the THIN adapter that hands
//  those tokens to `NativeView` so the native UI uses the same brand colors as
//  the web build — closing the "one source, themed everywhere" loop.
//

#if canImport(SwiftUI)
import SwiftUI

/// The brand theme the renderer should use, resolved for the current color
/// scheme from the compiler-emitted `BrandTokens`. Public so an app can read
/// individual tokens (e.g. for a custom button style) and pass it into
/// `RenderEnv`.
public struct MotoViewTheme {
    /// The accent color used for links and primary actions, taken straight from
    /// the compiler's `colorBrandForegroundLink` token.
    public let accent: Color
    /// The primary brand background (e.g. for filled buttons).
    public let brandBackground: Color
    /// The brand foreground (e.g. heading accents).
    public let brandForeground: Color

    /// Build a theme from the Slice 7 tokens for the given scheme. `BrandTokens`
    /// is the generated type in `BrandTokens.swift`.
    public init(scheme: ColorScheme) {
        let t = BrandTokens.tokens(scheme)
        self.accent = t.colorBrandForegroundLink
        self.brandBackground = t.colorBrandBackground
        self.brandForeground = t.colorBrandForeground1
    }

    /// A default light theme for previews / non-scheme contexts.
    public static let light = MotoViewTheme(scheme: .light)
}

/// A SwiftUI wrapper that renders a MotoView IR forest with the brand theme
/// applied as the SwiftUI `.accentColor`, so `NativeView`'s link/button styling
/// picks up the compiler-emitted brand color automatically.
public struct ThemedForest: View {
    @Environment(\.colorScheme) private var scheme
    private let forest: [UINode]
    private let emit: (EventContext) -> Void

    public init(_ forest: [UINode], emit: @escaping (EventContext) -> Void = { _ in }) {
        self.forest = forest
        self.emit = emit
    }

    public var body: some View {
        let theme = MotoViewTheme(scheme: scheme)
        NativeForest(forest, env: RenderEnv(emit: emit))
            .accentColor(theme.accent)
    }
}
#endif
