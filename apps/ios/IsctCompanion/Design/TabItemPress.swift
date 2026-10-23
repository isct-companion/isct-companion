import SwiftUI

// A long press on a tab bar item raises a glass panel of rows above it: the
// ウェブ tab's site picker and the 時間割 tab's calendar export. The finger can
// slide to a row and lift to choose it, or lift first and tap.

/// The long-press state of one tab item, fed by `TabBarLongPress` and drawn
/// by `TabPressPanel` in the root.
@MainActor
@Observable
final class TabItemPress<Choice: Hashable> {
    /// The pressed item's frame (window coordinates) while the panel is up.
    private(set) var anchor: CGRect?
    /// Where the finger is while it stays down; nil once lifted.
    private(set) var finger: CGPoint?
    /// The finger left the item, so lifting away from a row dismisses.
    private var moved = false
    /// Bumped when the panel opens, for the haptic.
    private(set) var opened = 0
    /// The panel rows' frames (window coordinates).
    @ObservationIgnored var rows: [Choice: CGRect] = [:]

    var isOpen: Bool { anchor != nil }

    /// The row under the finger.
    var highlighted: Choice? {
        guard let finger else { return nil }
        return rows.first { $0.value.contains(finger) }?.key
    }

    /// Feeds one tab-bar event; returns the row chosen by lifting on it.
    func handle(_ event: TabBarLongPress.Event) -> Choice? {
        switch event {
        case let .began(item, at):
            rows = [:]
            anchor = item
            finger = at
            moved = false
            opened += 1
        case let .changed(point):
            finger = point
            if let anchor, !anchor.insetBy(dx: -8, dy: -8).contains(point) { moved = true }
        case let .ended(point):
            finger = point
            let chosen = highlighted
            finger = nil
            // Lifting on the item leaves the panel open for a tap.
            if chosen != nil || moved { dismiss() }
            return chosen
        case .cancelled:
            finger = nil
            dismiss()
        }
        return nil
    }

    func dismiss() {
        anchor = nil
        finger = nil
    }
}

/// The glass panel raised above a tab item by a long press. `rows` receives
/// the row under the finger, a frame reporter and the choose action.
struct TabPressPanel<Choice: Hashable, Rows: View>: View {
    let press: TabItemPress<Choice>
    let pick: (Choice) -> Void
    @ViewBuilder let rows: (_ highlighted: Choice?, _ report: @escaping (Choice, CGRect) -> Void,
                            _ pick: @escaping (Choice) -> Void) -> Rows

    private static var width: CGFloat { 240 }

    var body: some View {
        GeometryReader { proxy in
            if let anchor = press.anchor {
                let origin = proxy.frame(in: .global).origin
                let half = Self.width / 2
                let x = min(max(anchor.midX - origin.x, half + 12), proxy.size.width - half - 12)
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(0.08)
                        .contentShape(Rectangle())
                        .onTapGesture { withAnimation(.snappy) { press.dismiss() } }
                    VStack(spacing: 0) {
                        rows(press.highlighted, { press.rows[$0] = $1 }) { choice in
                            withAnimation(.snappy) { press.dismiss() }
                            pick(choice)
                        }
                    }
                    .padding(6)
                    .frame(width: Self.width)
                    .floatingGlass(in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                    .transition(.scale(scale: 0.85, anchor: .bottom).combined(with: .opacity))
                    // Bottom edge 10 pt above the tab item, centred on it.
                    .frame(height: max(0, anchor.minY - origin.y - 10), alignment: .bottom)
                    .offset(x: x - half)
                }
            }
        }
        .ignoresSafeArea()
        .animation(.snappy, value: press.isOpen)
        .sensoryFeedback(.impact(weight: .medium), trigger: press.opened)
        .sensoryFeedback(.selection, trigger: press.highlighted)
    }
}

/// One panel row: a button that reports its frame for the finger's hit test
/// and fills while under the finger or pressed. In a context menu (the iPad
/// sidebar) the menu draws the button itself and ignores the style.
struct TabPressRow<Choice: Hashable>: View {
    let choice: Choice
    let title: String
    var icon: String?
    var highlighted: Choice?
    var report: ((Choice, CGRect) -> Void)?
    let pick: (Choice) -> Void

    var body: some View {
        Button {
            pick(choice)
        } label: {
            if let icon {
                Label(title, systemImage: icon)
            } else {
                Text(title)
            }
        }
        .buttonStyle(TabPressRowStyle(highlighted: highlighted == choice))
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame in
            report?(choice, frame)
        }
    }
}

/// Title leading, symbol trailing in the accent.
private struct TabPressRowStyle: ButtonStyle {
    let highlighted: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .labelStyle(TrailingIconLabelStyle())
            .font(.body)
            .foregroundStyle(Theme.ink)
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                highlighted || configuration.isPressed ? Theme.line.opacity(0.9) : .clear,
                in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .contentShape(Rectangle())
    }
}

private struct TrailingIconLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack {
            configuration.title
            Spacer(minLength: 12)
            configuration.icon.foregroundStyle(Theme.accent)
        }
    }
}
