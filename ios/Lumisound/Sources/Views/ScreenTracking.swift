import SwiftUI

// MARK: - Screen tracking for sheets and covers
//
// Tabs log `screen_entered` themselves (ContentView), but sheets and
// full-screen covers logged nothing, so field logs couldn't say which sheet
// a user was in when something happened. `lumiSheet` / `lumiFullScreenCover`
// are drop-in replacements for SwiftUI's: the presented view logs
// `screen_entered: <name>` and becomes the performance telemetry screen while
// it is up, and the screen underneath comes back when it closes. The name is
// the presented view's type ("UploadMusicView"), so call sites stay as they
// were.

@MainActor
enum ScreenTracker {
    private static var stack: [String] = []

    static func enter(_ name: String) {
        stack.append(name)
        PerformanceTelemetryService.shared.setScreen(name)
        appBreadcrumb("Opened \(name)")
        appLog("screen_entered: \(name)", category: "navigation")
    }

    static func leave(_ name: String) {
        if let i = stack.lastIndex(of: name) { stack.remove(at: i) }
        if let below = stack.last { PerformanceTelemetryService.shared.setScreen(below) }
    }

    /// The first meaningful type in a view's type name: wrappers SwiftUI adds
    /// (ModifiedContent, NavigationStack, Optional, …) are skipped.
    nonisolated static func name<V>(of _: V.Type, fallback: String) -> String {
        let wrappers: Set<String> = ["ModifiedContent", "Optional", "NavigationStack", "NavigationView", "AnyView",
                                     "Group", "TupleView", "VStack", "HStack", "ZStack", "ScrollView", "Form", "List",
                                     "NavigationPath", "EmptyView", "Text", "Color", "Swift", "SwiftUI"]
        var ident = ""
        for c in String(describing: V.self) + "<" {
            if c.isLetter || c.isNumber || c == "_" {
                ident.append(c)
                continue
            }
            if !ident.isEmpty, !ident.hasPrefix("_"), !wrappers.contains(ident), ident.first?.isUppercase == true {
                return ident
            }
            ident = ""
        }
        return fallback
    }
}

private struct TrackedScreen: ViewModifier {
    let name: String
    func body(content: Content) -> some View {
        content
            .onAppear { ScreenTracker.enter(name) }
            .onDisappear { ScreenTracker.leave(name) }
    }
}

extension View {
    /// Log this view as a screen while it is shown.
    func lumiScreen(_ name: String) -> some View {
        modifier(TrackedScreen(name: name))
    }

    func lumiSheet<Content: View>(isPresented: Binding<Bool>, onDismiss: (() -> Void)? = nil,
                                  file: String = #fileID, line: Int = #line,
                                  @ViewBuilder content: @escaping () -> Content) -> some View {
        let name = ScreenTracker.name(of: Content.self, fallback: "\(file):\(line)")
        return sheet(isPresented: isPresented, onDismiss: onDismiss) { content().lumiScreen(name) }
    }

    func lumiSheet<Item: Identifiable, Content: View>(item: Binding<Item?>, onDismiss: (() -> Void)? = nil,
                                                      file: String = #fileID, line: Int = #line,
                                                      @ViewBuilder content: @escaping (Item) -> Content) -> some View {
        let name = ScreenTracker.name(of: Content.self, fallback: "\(file):\(line)")
        return sheet(item: item, onDismiss: onDismiss) { content($0).lumiScreen(name) }
    }

    func lumiFullScreenCover<Content: View>(isPresented: Binding<Bool>, onDismiss: (() -> Void)? = nil,
                                            file: String = #fileID, line: Int = #line,
                                            @ViewBuilder content: @escaping () -> Content) -> some View {
        let name = ScreenTracker.name(of: Content.self, fallback: "\(file):\(line)")
        return fullScreenCover(isPresented: isPresented, onDismiss: onDismiss) { content().lumiScreen(name) }
    }
}
