import IsctCore
import SwiftUI
import UniformTypeIdentifiers

extension View {
    func handInTarget(_ item: DeadlineItem) -> some View {
        handInTarget(
            assignment: item.assignmentId, title: item.title,
            availability: item.handInAvailability
        )
    }

    @ViewBuilder
    func handInTarget(assignment: String, title: String, availability: HandInAvailability) -> some View {
        if availability == .prepareDraft {
            modifier(HandInTargetModifier(choice: SharedInbox.Choice(id: assignment, title: title, course: nil, due: "")))
        } else {
            self
        }
    }
}

private struct HandInTargetModifier: ViewModifier {
    let choice: SharedInbox.Choice
    @Environment(InboxModel.self) private var inbox
    @State private var targeted = false

    func body(content: Content) -> some View {
        content
            .background(targeted ? Theme.accentSoft : Color.clear)
            .onDrop(of: [.fileURL, .data], isTargeted: $targeted) { providers in
                inbox.receive(providers, choice: choice)
            }
    }
}
