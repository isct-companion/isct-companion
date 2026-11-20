import SwiftUI

/// What waits to be handed in, as a stack of cards in the bottom-right
/// corner, like the screenshot thumbnail: the finger moves it at once. Drop it
/// on an assignment row or upload card (which floats while the stack is over
/// it) to put the files there and open that assignment; throw it to the trash
/// that appears top right to discard them; tap it for the upload panel. The
/// layer covers the page but only the stack takes touches.
struct InboxTray: View {
    /// Space kept free under the stack: the tab bar or the status bar.
    let bottom: CGFloat
    @Environment(InboxModel.self) private var inbox
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var drag: CGSize = .zero
    @State private var dragging = false
    @State private var overTrash = false
    @State private var discarded = 0

    private static let card = CGSize(width: 68, height: 88)

    var body: some View {
        GeometryReader { proxy in
            let frame = proxy.frame(in: .global)
            ZStack(alignment: .bottomTrailing) {
                Color.clear.allowsHitTesting(false)
                if dragging {
                    trash
                        .allowsHitTesting(false)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                        .padding(.top, Theme.space10)
                        .padding(.trailing, Theme.space10)
                        .transition(.scale.combined(with: .opacity))
                }
                if !inbox.items.isEmpty {
                    stack
                        .scaleEffect(dragging ? 0.9 : 1)
                        .offset(drag)
                        .padding(.trailing, Theme.space8)
                        .padding(.bottom, bottom)
                        .onTapGesture { inbox.presenting = true }
                        .gesture(gesture(in: frame))
                        .transition(.scale.combined(with: .opacity))
                        .accessibilityAddTraits(.isButton)
                        .accessibilityLabel(t("handIn.sharedFiles"))
                }
            }
            .animation(.smooth(duration: Theme.motionBase), value: dragging)
            .animation(.smooth(duration: Theme.motionBase), value: inbox.items.count)
        }
        .sensoryFeedback(.impact(weight: .medium), trigger: discarded)
    }

    /// The waiting files in order, each a card a little behind the next.
    private var stack: some View {
        let shown = Array(inbox.items.suffix(4))
        return ZStack(alignment: .bottomTrailing) {
            ForEach(Array(shown.enumerated()), id: \.element.id) { index, item in
                let depth = CGFloat(shown.count - 1 - index)
                if let url = item.url {
                    Thumbnail(url: url, corner: 10)
                        .frame(width: Self.card.width, height: Self.card.height)
                        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Theme.surface, lineWidth: 3))
                        .shadow(color: .black.opacity(0.22), radius: 6, y: 3)
                        .offset(x: -depth * 7, y: -depth * 7)
                }
            }
        }
        .padding([.top, .leading], CGFloat(max(shown.count - 1, 0)) * 7)
        .contentShape(Rectangle())
    }

    private var trash: some View {
        Image(systemName: overTrash ? "trash.fill" : "trash")
            .font(.title2.weight(.semibold))
            .foregroundStyle(overTrash ? Theme.accentInk : Theme.danger)
            .frame(width: 60, height: 60)
            .background(overTrash ? Theme.danger : Theme.surface, in: Circle())
            .shadow(color: .black.opacity(0.18), radius: 8, y: 3)
            .scaleEffect(overTrash ? 1.2 : 1)
            .symbolEffect(.bounce, value: discarded)
            .animation(.smooth(duration: Theme.motionBase), value: overTrash)
            .accessibilityHidden(true)
    }

    private func gesture(in frame: CGRect) -> some Gesture {
        DragGesture(minimumDistance: 4, coordinateSpace: .global)
            .onChanged { value in
                dragging = true
                drag = value.translation
                inbox.hover(at: value.location)
                // The trash sits in the window's top-right corner.
                let corner = CGRect(x: frame.maxX - 140, y: frame.minY, width: 140, height: 140)
                overTrash = inbox.hovered == nil && corner.contains(value.location)
            }
            .onEnded { _ in
                let target = inbox.hoveredTarget
                let trashed = overTrash
                inbox.hover(at: nil)
                overTrash = false
                if let target {
                    Task { await give(to: target) }
                } else if trashed {
                    discarded += 1
                    inbox.discardAll()
                }
                withAnimation(.smooth(duration: Theme.motionBase)) {
                    drag = .zero
                    dragging = false
                }
            }
    }

    private func give(to target: InboxModel.Target) async {
        guard await inbox.give(inbox.items, to: target.assignment, model) else { return }
        if target.opens {
            navigator.tab = .today
            navigator.open(AssignmentRoute(id: target.assignment))
        }
    }
}
