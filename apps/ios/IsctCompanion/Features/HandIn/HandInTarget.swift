import SwiftUI

extension View {
    /// Makes the view a place to drop the corner stack for `assignment`; it
    /// floats while the stack is over it. `opens`: the drop opens the
    /// assignment (a row), or the page is already showing it (its card).
    func handInTarget(_ assignment: String, opens: Bool = true) -> some View {
        modifier(HandInTargetModifier(assignment: assignment, opens: opens))
    }
}

private struct HandInTargetModifier: ViewModifier {
    let assignment: String
    let opens: Bool
    @Environment(InboxModel.self) private var inbox
    @State private var token = UUID()

    func body(content: Content) -> some View {
        let over = inbox.hovered == token
        content
            .background {
                if over {
                    RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous)
                        .fill(Theme.accentSoft)
                }
            }
            .overlay {
                if over {
                    RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous)
                        .stroke(Theme.accent, lineWidth: 2)
                }
            }
            .scaleEffect(over ? 1.03 : 1)
            .shadow(color: .black.opacity(over ? 0.18 : 0), radius: 12, y: 4)
            .zIndex(over ? 1 : 0)
            .animation(.smooth(duration: Theme.motionBase), value: over)
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
                inbox.targets[token] = InboxModel.Target(assignment: assignment, frame: frame, opens: opens)
            }
            .onDisappear { inbox.targets[token] = nil }
    }
}
