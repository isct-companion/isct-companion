import IsctCore
import SwiftUI

@MainActor @Observable
final class AppNotices {
    struct Feedback: Identifiable {
        let id = UUID()
        let text: String
    }

    struct Dialog: Identifiable {
        struct Action {
            let label: String
            var role: ButtonRole? = nil
            var preferred = false
            let run: @MainActor () -> Void
        }

        let id: String
        let title: String
        var message: String? = nil
        var announcementId: String? = nil
        let actions: [Action]
    }

    private(set) var feedback: Feedback?
    private(set) var requests: [Dialog] = []
    private(set) var flows: Set<UUID> = []

    func show(_ text: String) { feedback = Feedback(text: text) }
    func expire(_ id: UUID) { if feedback?.id == id { feedback = nil } }

    func confirm(title: String, message: String? = nil, action: String, destructive: Bool = false,
                 cancelLabel: String = t("common.cancel"), run: @escaping @MainActor () -> Void) {
        requests.append(Dialog(id: UUID().uuidString, title: title, message: message, actions: [
            .init(label: cancelLabel, role: .cancel, run: {}),
            .init(label: action, role: destructive ? .destructive : nil, preferred: !destructive, run: run),
        ]))
    }

    func takeRequest() -> Dialog? { requests.isEmpty ? nil : requests.removeFirst() }
    func setFlow(_ id: UUID, _ open: Bool) { if open { flows.insert(id) } else { flows.remove(id) } }
}

extension View {
    /// Sheet content and system presenters report their lifetime to the notice host.
    func appNoticeFlow(_ open: Bool = true) -> some View { modifier(NoticeFlow(open: open)) }
}

private struct NoticeFlow: ViewModifier {
    @Environment(AppModel.self) private var model
    @State private var id = UUID()
    let open: Bool

    func body(content: Content) -> some View {
        content
            .onAppear { model.notices.setFlow(id, open) }
            .onChange(of: open) { _, value in model.notices.setFlow(id, value) }
            .onDisappear { model.notices.setFlow(id, false) }
    }
}

struct AppNoticeLayer: ViewModifier {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    @AppStorage(Guide.welcomeKey) private var welcomed = false
    @State private var welcome = false
    @State private var dialog: AppNotices.Dialog?
    @State private var offered: Set<String> = []
    let blocked: Bool
    let connect: () -> Void

    private var busy: Bool { blocked || !model.notices.flows.isEmpty || welcome }
    private var interactive: Bool { scenePhase == .active }
    private var activeIds: [String] { model.announcements?.active.map(\.id) ?? [] }
    private var updateCandidate: AppRelease? {
        (model.records.settings["appUpdateChecks"] as? Bool ?? true) ? model.updateOffer : nil
    }
    private var pendingAnnouncements: [AnnouncementView] {
        model.announcementsReady ? model.announcements?.pending ?? [] : []
    }

    private struct Eligibility: Equatable {
        let interactive, busy, welcomed: Bool
        let undo: UUID?
        let current, user, update: String?
        let active, pending: [String]
    }

    private var eligibility: Eligibility {
        Eligibility(
            interactive: interactive, busy: busy, welcomed: welcomed, undo: model.undo?.id,
            current: dialog?.id, user: model.notices.requests.first?.id,
            update: updateCandidate?.version, active: activeIds, pending: pendingAnnouncements.map(\.id)
        )
    }

    func body(content: Content) -> some View {
        let showFeedback = interactive && !busy && dialog == nil
        content
            .sheet(isPresented: Binding(
                get: { welcome },
                set: { shown in
                    if !shown {
                        welcomed = true
                        Guide.welcomed = true
                    }
                    welcome = shown
                }
            )) { WelcomeSheet(connect: connect) }
            .alert(dialog?.title ?? "", isPresented: Binding(
                get: { dialog != nil },
                set: { if !$0 { dialog = nil } }
            ), presenting: dialog) { shown in
                ForEach(Array(shown.actions.enumerated()), id: \.offset) { _, action in
                    if action.preferred {
                        Button(action.label, role: action.role, action: action.run).keyboardShortcut(.defaultAction)
                    } else {
                        Button(action.label, role: action.role, action: action.run)
                    }
                }
            } message: { shown in
                if let message = shown.message { Text(message) }
            }
            .onChange(of: eligibility, initial: true) { _, _ in advance() }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if let undo = model.undo {
                    FeedbackCapsule(
                        interactive: showFeedback,
                        text: t("app.hidTitle", ["title": undo.title]),
                        undo: { Task { await model.undoHide(undo) } },
                        expire: { model.dismissUndo(undo) }
                    ).id(undo.id)
                } else if let feedback = model.notices.feedback {
                    FeedbackCapsule(
                        interactive: showFeedback,
                        text: feedback.text,
                        expire: { model.notices.expire(feedback.id) }
                    ).id(feedback.id)
                }
            }
    }

    private func advance() {
        if let id = dialog?.announcementId, !activeIds.contains(id) {
            dialog = nil
            return
        }
        guard interactive, dialog == nil, !busy else { return }
        if let requested = model.notices.takeRequest() {
            dialog = requested
            return
        }
        guard model.undo == nil else { return }
        if !welcomed {
            welcome = true
            return
        }
        if let release = updateCandidate, offered.insert("update:\(release.version)").inserted {
            let done = { model.closeUpdate(release.version) }
            var actions: [AppNotices.Dialog.Action] = [
                .init(label: t("common.later"), role: .cancel, run: done),
                .init(label: t("appUpdate.prompt.skip"), run: { Task { await model.skipUpdate(release.version) } }),
            ]
            if let url = URL(string: release.url) {
                actions.append(.init(label: t("appUpdate.openAppStore"), preferred: true, run: { done(); openURL(url) }))
            }
            dialog = .init(
                id: "update:\(release.version)",
                title: t("appUpdate.prompt.available", ["version": release.version]),
                actions: actions
            )
            return
        }
        guard let item = pendingAnnouncements.first(where: { !offered.contains("announcement:\($0.id)") })
        else { return }
        let id = "announcement:\(item.id)"
        offered.insert(id)
        let answer = { Task { await model.answerAnnouncement(item.id) } }
        var actions: [AppNotices.Dialog.Action] = [
            .init(label: t("announcements.acknowledge"), preferred: true, run: { _ = answer() }),
        ]
        if let link = item.link, let url = URL(string: link.url) {
            actions.append(.init(label: link.label, run: { _ = answer(); openURL(url) }))
        }
        dialog = .init(id: id, title: item.title, message: item.body, announcementId: item.id, actions: actions)
    }
}

private struct FeedbackCapsule: View {
    let interactive: Bool
    let text: String
    var undo: (() -> Void)? = nil
    let expire: () -> Void
    @State private var remaining = Duration.seconds(4)

    var body: some View {
        Group {
            if interactive {
                HStack {
                    Text(text).lineLimit(2)
                    if let undo {
                        Spacer()
                        Button(t("common.undo"), action: undo).fontWeight(.semibold)
                    }
                }
                .font(.subheadline)
                .foregroundStyle(Theme.ink)
                .tint(Theme.accent)
                .padding(.horizontal, Theme.space8)
                .padding(.vertical, Theme.space6)
                .floatingGlass(in: Capsule())
                .padding(.horizontal, Theme.gutter)
                .padding(.bottom, Theme.space4)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .task(id: interactive) {
            guard interactive else { return }
            let clock = ContinuousClock()
            let start = clock.now
            do {
                try await Task.sleep(for: remaining)
                guard !Task.isCancelled else { return }
                expire()
            } catch {
                remaining = max(.zero, remaining - start.duration(to: clock.now))
            }
        }
    }
}
