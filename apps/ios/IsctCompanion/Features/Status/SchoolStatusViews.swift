import Foundation
import IsctCore
import Pow
import SwiftUI

/// Words for Core's sync snapshot (`sync_engine`). Core decides what runs, what
/// waits, what failed and when it retries; these only name it.
enum SyncText {
    /// A source as Core names it in `SourceView.source`.
    static func source(_ source: String) -> String {
        switch source {
        case "lms": return "LMS"
        case "kyomu": return "教務Web"
        case "references": return t("Reference data")
        case "catalog": return t("Course catalog")
        default: return source
        }
    }

    /// Dataset names, keyed by the dataset id Core puts in `RunView.dataset`
    /// and in its labels' keys.
    private static func datasetName(_ key: String) -> String? {
        switch key {
        case "lms.enrolment": return t("Courses")
        case "lms.deadlines": return t("Deadlines")
        case "lms.course": return t("Course content")
        case "lms.assign": return t("Assignments")
        case "lms.history": return t("Past courses")
        case "kyomu.page": return t("教務Web pages")
        case "catalog.official": return t("Course catalog")
        case "reference": return t("Reference data")
        default: return nil
        }
    }

    static func dataset(_ dataset: String) -> String {
        datasetName(dataset) ?? dataset
    }

    /// A Core label: a dataset key reads as the dataset's name; any other key
    /// is a catalog key filled with Core's arguments.
    static func label(_ label: SyncLabel) -> String {
        let key = label.key.hasPrefix("sync.") ? String(label.key.dropFirst("sync.".count)) : label.key
        if let name = datasetName(key) ?? datasetName(label.key) { return name }
        let values = (try? JSONSerialization.jsonObject(with: Data(label.args.utf8))) as? [String: Any] ?? [:]
        return t(label.key, values)
    }

    static func held(_ held: Held) -> String {
        switch held {
        case .offline: return t("Waiting for a connection")
        case .cellular: return t("Waiting for Wi-Fi")
        case .powerSaver: return t("Paused for Low Power Mode")
        case .background: return t("Resumes when the app is in front")
        case let .backoff(until): return retry(at: until)
        case let .hostPaused(host, until): return t("{{host}} is paused until {{time}}", ["host": host, "time": Format.due(until)])
        case .signInRequired: return t("Sign in required")
        case .resetting: return t("Resetting")
        }
    }

    static func retry(at raw: String) -> String {
        t("Retry at {{time}}", ["time": Format.due(raw)])
    }

    /// "Could not update Deadlines · Retry at Today 14:05".
    static func failure(_ failure: SyncFailure, name: String) -> String {
        [t("Could not update {{source}}", ["source": name]), failure.retryAt.map(retry(at:))]
            .compactMap { $0 }.joined(separator: " · ")
    }

    /// What a Retry asks Core to read again for a failed run.
    static func scope(for run: RunView) -> SyncScope {
        if run.dataset.hasPrefix("lms.") {
            if let id = run.scope.flatMap(UInt64.init) { return .course(lmsCourseId: id) }
            return .site(site: .scienceLms)
        }
        if run.dataset.hasPrefix("kyomu.") { return .site(site: .kyomu) }
        if run.dataset == "reference" { return .references }
        if run.dataset.hasPrefix("catalog."), let year = run.scope.flatMap(UInt16.init) { return .catalog(year: year) }
        return .everything
    }

    /// What a Retry asks Core to read again for a failed source.
    static func scope(for source: SourceView) -> SyncScope {
        switch source.source {
        case "lms": return .site(site: .scienceLms)
        case "kyomu": return .site(site: .kyomu)
        case "references": return .references
        default: return .everything
        }
    }
}

/// What the header shows for Core's runs: a ring only when a run has a total,
/// a spinner for other work, an icon when everything waits or failed.
enum SyncIndicator: Equatable {
    case progress(Double, emphasized: Bool)
    case spinner(emphasized: Bool)
    case failed
    case held
}

/// The one thing that needs the person, for 今日's banner.
struct SyncProblem {
    let text: String
    let retry: SyncScope?
    let signIn: SchoolAuthProfile?
}

extension SyncSnapshot {
    /// Runs reading now: neither held nor failed.
    var activeRuns: [RunView] { runs.filter { $0.held == nil && $0.failure == nil } }

    /// Work the person asked for or is looking at, as opposed to background reads.
    var isBusy: Bool { resetting || activeRuns.contains { $0.priority != .background } }

    var indicator: SyncIndicator? {
        let active = activeRuns
        if let run = active.first(where: { $0.total != nil }), let total = run.total {
            return .progress(Double(min(run.done, total)) / Double(max(total, 1)), emphasized: run.priority != .background)
        }
        if resetting { return .spinner(emphasized: true) }
        if let run = active.first { return .spinner(emphasized: run.priority != .background) }
        if problem != nil { return .failed }
        if runs.contains(where: { $0.held != nil }) { return .held }
        return nil
    }

    var problem: SyncProblem? {
        let held = runs.compactMap(\.held) + sources.compactMap(\.held)
        for case let .signInRequired(profile) in held {
            return SyncProblem(text: t("Sign in required"), retry: nil, signIn: profile)
        }
        if let run = runs.first(where: { $0.failure != nil }), let failure = run.failure {
            return SyncProblem(
                text: SyncText.failure(failure, name: SyncText.label(run.label)), retry: SyncText.scope(for: run), signIn: nil)
        }
        if let source = sources.first(where: { $0.failure != nil }), let failure = source.failure {
            return SyncProblem(
                text: SyncText.failure(failure, name: SyncText.source(source.source)),
                retry: SyncText.scope(for: source), signIn: nil)
        }
        return nil
    }

    /// One line for the first-load card and the indicator's accessibility label.
    var line: String {
        if resetting { return t("Resetting") }
        if let run = activeRuns.first {
            return ([SyncText.label(run.label)] + [run.current.map(SyncText.label)].compactMap { $0 }).joined(separator: " · ")
        }
        if let problem { return problem.text }
        if let run = runs.first(where: { $0.held != nil }), let held = run.held {
            return "\(SyncText.label(run.label)) · \(SyncText.held(held))"
        }
        return t("Up to date")
    }
}

/// The sync ring beside the settings button (`docs/ux/mobile.md`, Sync
/// feedback): the shape of desktop's `PullRing`.
struct SyncRing: View {
    let progress: Double
    var emphasized = true

    var body: some View {
        ZStack {
            Circle().stroke(Theme.line, lineWidth: 3)
            Circle()
                .trim(from: 0, to: max(0.03, min(1, progress)))
                .stroke(emphasized ? Theme.accent : Theme.muted, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: 20, height: 20)
        .animation(.easeOut(duration: 0.3), value: progress)
    }
}

struct SyncIndicatorView: View {
    let indicator: SyncIndicator

    var body: some View {
        switch indicator {
        case let .progress(progress, emphasized):
            SyncRing(progress: progress, emphasized: emphasized)
        case let .spinner(emphasized):
            ProgressView().controlSize(.small).tint(emphasized ? Theme.accent : Theme.muted)
        case .failed:
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.danger)
        case .held:
            Image(systemName: "pause.circle").foregroundStyle(Theme.muted)
        }
    }
}

/// The indicator as a button that opens the connection sheet. When work the
/// person started finishes cleanly, a check pops in its place for a moment.
struct SyncRingButton: View {
    @Environment(AppModel.self) private var model
    @State private var showsSheet = false
    @State private var finished = 0
    @State private var showsDone = false

    var body: some View {
        ZStack {
            if let snapshot = model.syncSnapshot, let indicator = snapshot.indicator {
                Button {
                    showsSheet = true
                } label: {
                    SyncIndicatorView(indicator: indicator)
                        .frame(width: 40, height: 40)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel(snapshot.line)
            } else if showsDone {
                Image(systemName: "checkmark.circle.fill")
                    .font(.title3)
                    .foregroundStyle(Theme.ok)
                    .frame(width: 40, height: 40)
                    .transition(.movingParts.pop(Theme.ok))
                    .accessibilityLabel(t("Updated"))
            }
        }
        .animation(.snappy, value: showsDone)
        .sensoryFeedback(.success, trigger: finished)
        .sheet(isPresented: $showsSheet) { SyncStatusSheet() }
        .onChange(of: model.syncSnapshot?.isBusy) { old, new in
            guard old == true, new == false, model.syncSnapshot?.indicator == nil else { return }
            finished += 1
            showsDone = true
            Task {
                try? await Task.sleep(for: .seconds(1.6))
                showsDone = false
            }
        }
    }
}

/// The connection sheet: Core's sources, then its runs, as Core lists them.
struct SyncStatusSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                SyncSections()
            }
            .navigationTitle(t("School connection"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("Close")) { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

/// Sources with their last update, then the runs underway, for the sheet and
/// データ更新.
struct SyncSections: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let snapshot = model.syncSnapshot {
            if !snapshot.sources.isEmpty {
                Section(t("Last updated")) {
                    ForEach(snapshot.sources, id: \.source) { SyncSourceRow(source: $0) }
                }
            }
            if snapshot.resetting || !snapshot.runs.isEmpty {
                Section {
                    if snapshot.resetting {
                        HStack {
                            Text(t("Resetting"))
                            Spacer()
                            ProgressView().controlSize(.small)
                        }
                    }
                    ForEach(snapshot.runs, id: \.run) { SyncRunRow(run: $0) }
                }
            }
        }
    }
}

private struct SyncSourceRow: View {
    let source: SourceView
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(SyncText.source(source.source)).font(.body.weight(.semibold))
                Spacer()
                if source.running { ProgressView().controlSize(.small) }
            }
            Text(source.lastUpdated.map { t("Updated {{date}}", ["date": Format.due($0)]) } ?? t("Never updated"))
                .font(.subheadline)
                .foregroundStyle(Theme.muted)
            if let maintenance = source.maintenance {
                Text([t("{{source}} is under maintenance", ["source": SyncText.source(source.source)]), maintenance]
                    .filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.footnote)
                    .foregroundStyle(Theme.warn)
            } else if let failure = source.failure {
                FailureLine(text: SyncText.failure(failure, name: SyncText.source(source.source))) {
                    model.syncHint(.refresh(scope: SyncText.scope(for: source)))
                }
            } else if let held = source.held {
                Text(SyncText.held(held)).font(.footnote).foregroundStyle(Theme.muted)
            } else if source.session == "signed-out" {
                Text(t("Not connected")).font(.footnote).foregroundStyle(Theme.muted)
            }
        }
        .padding(.vertical, 2)
    }
}

/// One run: a bar only when Core knows its total, a spinner otherwise.
private struct SyncRunRow: View {
    let run: RunView
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(SyncText.label(run.label))
                Spacer()
                if let total = run.total {
                    Text(verbatim: "\(run.done)/\(total)")
                        .font(.footnote.monospacedDigit())
                        .foregroundStyle(Theme.muted)
                } else if run.held == nil, run.failure == nil {
                    ProgressView().controlSize(.small)
                }
            }
            if let total = run.total, run.failure == nil {
                ProgressView(value: Double(min(run.done, total)), total: Double(max(total, 1)))
                    .tint(run.priority == .background ? Theme.muted : Theme.accent)
            }
            if let current = run.current {
                Text(SyncText.label(current)).font(.footnote).foregroundStyle(Theme.muted).lineLimit(1)
            }
            if let failure = run.failure {
                FailureLine(text: SyncText.failure(failure, name: SyncText.dataset(run.dataset))) {
                    model.syncHint(.refresh(scope: SyncText.scope(for: run)))
                }
            } else if let held = run.held {
                Text(SyncText.held(held)).font(.footnote).foregroundStyle(Theme.muted)
            }
        }
        .padding(.vertical, 2)
    }
}

private struct FailureLine: View {
    let text: String
    let retry: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(text).font(.footnote).foregroundStyle(Theme.danger)
            Spacer()
            Button(t("Retry"), action: retry)
                .font(.footnote.weight(.semibold))
                .buttonStyle(.borderless)
        }
    }
}

/// "Needs you" (Sync feedback level 3): a failed read with Retry, or an
/// account to sign in to again.
struct SyncProblemBanner: View {
    let problem: SyncProblem
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.danger)
            Text(problem.text).font(.subheadline).foregroundStyle(Theme.ink)
            Spacer()
            if let profile = problem.signIn {
                NavigationLink(value: SettingsRoute.account(profile)) {
                    Text(t("Sign in")).font(.subheadline.weight(.semibold))
                }
            } else if let scope = problem.retry {
                Button(t("Retry")) { model.syncHint(.refresh(scope: scope)) }
                    .font(.subheadline.weight(.semibold))
            }
        }
        .padding(14)
        .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }
}

/// First launch after signing in: Today has nothing yet, so say what is
/// being read instead of showing a blank page.
struct FirstLoadCard: View {
    let snapshot: SyncSnapshot

    var body: some View {
        HStack(spacing: 14) {
            SyncIndicatorView(indicator: snapshot.indicator ?? .spinner(emphasized: true))
            VStack(alignment: .leading, spacing: 4) {
                Text(t("Loading school data")).font(.body.weight(.semibold)).foregroundStyle(Theme.ink)
                Text(snapshot.line).font(.footnote).foregroundStyle(Theme.muted).lineLimit(2)
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }
}
