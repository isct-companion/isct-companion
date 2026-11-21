import Foundation
import IsctCore
import Pow
import SwiftUI

/// Words for Core's sync snapshot (`plan`). Core decides what runs, what
/// waits, what failed and when it retries; these only name it.
enum SyncText {
    /// A source as Core names it in `SourceView.source`.
    static func source(_ source: String) -> String {
        switch source {
        case "lms": return "LMS"
        case "kyomu": return "教務Web"
        case "references": return t("syncStatus.dataset.referenceData")
        case "catalog": return t("syncStatus.dataset.courseCatalog")
        default: return source
        }
    }

    /// Dataset names, keyed by the dataset id Core puts in `RunView.dataset`
    /// and in its labels' keys.
    private static func datasetName(_ key: String) -> String? {
        switch key {
        case "lms.enrolment": return t("common.courses")
        case "lms.deadlines": return t("syncStatus.dataset.deadlines")
        case "lms.course": return t("syncStatus.dataset.courseContent")
        case "lms.assign": return t("common.assignments")
        case "lms.history": return t("syncStatus.dataset.pastCourses")
        case "kyomu.page": return t("syncStatus.dataset.kyomuPages")
        case "catalog.official": return t("syncStatus.dataset.courseCatalog")
        case "reference": return t("syncStatus.dataset.referenceData")
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
        return lookup(label.key, values) ?? label.key
    }

    static func held(_ held: Held) -> String {
        switch held {
        case .offline: return t("syncStatus.waitingForAConnection")
        case .cellular: return t("syncStatus.waitingForWifi")
        case .powerSaver: return t("syncStatus.pausedLowPowerMode")
        case .background: return t("syncStatus.resumesWhenTheAppIs")
        case let .backoff(until): return retry(at: until)
        case let .hostPaused(host, until): return t("syncStatus.hostIsPausedUntilTime", ["host": host, "time": Format.due(until)])
        case let .account(_, wait): return account(wait)
        case .resetting: return t("syncStatus.resetting")
        case .prerequisite: return t("syncStatus.waitingForTheDataIt")
        }
    }

    /// What an account's waiting reads wait for.
    static func account(_ wait: AccountWait) -> String {
        switch wait {
        case .person: return t("syncStatus.held.person")
        case let .paused(until): return t("schoolSession.state.unavailableRetry", ["time": Format.due(until)])
        }
    }

    static func retry(at raw: String) -> String {
        t("syncStatus.retryAtTime", ["time": Format.due(raw)])
    }

    /// "Could not update Deadlines · Retry at Today 14:05".
    static func failure(_ failure: SyncFailure, name: String) -> String {
        [t("syncStatus.couldNotUpdateSource", ["source": name]), label(failure.reason), failure.retryAt.map(retry(at:))]
            .compactMap { $0 }.joined(separator: " · ")
    }


}

/// Words for Core's school sign-in read model (`SchoolSessionSnapshot`). Core
/// decides the state, why an account is stuck and when it tries again; these
/// only name it. Core's `message` is a diagnostic and never shown.
enum SchoolSessionText {
    static func line(_ presentation: SignInPresentation, retryAt: String? = nil) -> String {
        let args = Format.date(retryAt).map { ["time": Format.time($0)] } ?? [:]
        return [t(presentation.stateKey, args), presentation.reasonKey.map { t($0) }]
            .compactMap { $0 }.joined(separator: " · ")
    }

    static func line(_ session: SchoolProfileSession) -> String {
        line(session.presentation, retryAt: session.retryAt)
    }

    /// The paused site of an account (LMS for Science Tokyo, 教務Web for Tokyo
    /// Tech), until when, and the school's own notice.
    static func maintenance(_ maintenance: SchoolMaintenance, profile: SchoolAuthProfile) -> String {
        let source = SyncText.source(profile == .science ? "lms" : "kyomu")
        let until = t("schoolSession.maintenance.until", ["site": source, "time": Format.due(maintenance.until)])
        return ([until] + [maintenance.notice].compactMap { $0 }.filter { !$0.isEmpty }).joined(separator: " · ")
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
    let retry: RetryPlan?
    let signIn: SchoolAuthProfile?
}

extension SyncSnapshot {
    var showsLoading: Bool {
        switch attention.primary {
        case .active, .resetting: true
        default: false
        }
    }

    var indicator: SyncIndicator? {
        switch attention.primary {
        case .resetting: return .spinner(emphasized: true)
        case .held: return .held
        case .problem: return .failed
        case let .active(id):
            guard let run = runs.first(where: { $0.run == id }) else { return nil }
            let emphasized = run.priority != .background
            if let total = run.total {
                return .progress(Double(min(run.done, total)) / Double(max(total, 1)), emphasized: emphasized)
            }
            return .spinner(emphasized: emphasized)
        case nil: return nil
        }
    }

    var problem: SyncProblem? {
        switch attention.problem {
        case let .account(profile, presentation, retryAt):
            let text = presentation.map { SchoolSessionText.line($0, retryAt: retryAt) } ?? SyncText.account(.person)
            return SyncProblem(text: text, retry: nil, signIn: profile)
        case let .runFailed(dataset, failure, retry):
            return SyncProblem(text: SyncText.failure(failure, name: SyncText.dataset(dataset)), retry: retry, signIn: nil)
        case let .sourceFailed(source, failure, retry):
            return SyncProblem(text: SyncText.failure(failure, name: SyncText.source(source)), retry: retry, signIn: nil)
        case nil: return nil
        }
    }

    var line: String {
        switch attention.primary {
        case .resetting: return t("syncStatus.resetting")
        case .problem: if let problem { return problem.text }
        case let .active(id):
            if let run = runs.first(where: { $0.run == id }) {
                return ([SyncText.label(run.label)] + [run.current.map(SyncText.label)].compactMap { $0 }).joined(separator: " · ")
            }
        case let .held(id):
            if let run = runs.first(where: { $0.run == id }), let held = run.held {
                return "\(SyncText.label(run.label)) · \(SyncText.held(held))"
            }
        case nil: break
        }
        return t("syncStatus.upToDate")
    }
}

/// The sync ring beside the settings button (slotledger
/// `approach/mobile-today-phases`): the shape of desktop's `PullRing`.
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
        .animation(.smooth(duration: Theme.motionSlow), value: progress)
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
                    .accessibilityLabel(t("syncStatus.updated"))
            }
        }
        .animation(.smooth(duration: Theme.motionBase), value: showsDone)
        .sheet(isPresented: $showsSheet) { SyncStatusSheet() }
        .onChange(of: model.syncSnapshot?.attention.busy) { old, new in
            guard old == true, new == false, model.syncSnapshot?.indicator == nil else { return }
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
                Group {
                    SyncSections()
                }
                .themedRows()
            }
            .themedList()
            .navigationTitle(t("common.schoolConnection"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("common.close")) { dismiss() }
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
                Section(t("syncStatus.lastUpdated")) {
                    ForEach(snapshot.sources, id: \.source) { SyncSourceRow(source: $0) }
                }
            }
            if snapshot.resetting || !snapshot.runs.isEmpty {
                Section {
                    if snapshot.resetting {
                        HStack {
                            Text(t("syncStatus.resetting"))
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

    var body: some View {
        VStack(alignment: .leading, spacing: Theme.space2) {
            HStack {
                Text(SyncText.source(source.source)).font(.body.weight(.semibold))
                Spacer()
                if source.running { ProgressView().controlSize(.small) }
            }
            Text(source.lastUpdated.map { t("syncStatus.updatedDate", ["date": Format.due($0)]) } ?? t("common.neverUpdated"))
                .font(.subheadline)
                .foregroundStyle(Theme.muted)
            if let maintenance = source.maintenance {
                Text([t("syncStatus.sourceIsUnderMaintenance", ["source": SyncText.source(source.source)]), maintenance]
                    .filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.footnote)
                    .foregroundStyle(Theme.warn)
            } else if let failure = source.failure {
                FailureLine(text: SyncText.failure(failure, name: SyncText.source(source.source)), retry: source.retry)
            } else if let held = source.held {
                Text(SyncText.held(held)).font(.footnote).foregroundStyle(Theme.muted)
            } else if source.session == .notConfigured {
                Text(t("common.notConnected")).font(.footnote).foregroundStyle(Theme.muted)
            }
        }
        .padding(.vertical, Theme.space1)
    }
}

/// One run: a bar only when Core knows its total, a spinner otherwise.
private struct SyncRunRow: View {
    let run: RunView

    var body: some View {
        VStack(alignment: .leading, spacing: Theme.space2) {
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
                FailureLine(text: SyncText.failure(failure, name: SyncText.dataset(run.dataset)), retry: run.retry)
                if let detail = failure.detail {
                    // Core's diagnostic text, selectable to quote in a report.
                    Text(verbatim: detail).font(.footnote).foregroundStyle(Theme.muted).textSelection(.enabled)
                }
            } else if let held = run.held {
                Text(SyncText.held(held)).font(.footnote).foregroundStyle(Theme.muted)
            }
        }
        .padding(.vertical, Theme.space1)
    }
}

private struct FailureLine: View {
    let text: String
    let retry: RetryPlan?
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(text).font(.footnote).foregroundStyle(Theme.danger)
            Spacer()
            if let retry {
                Button(t("common.retry")) { model.retrySync(retry) }
                    .font(.footnote.weight(.semibold))
                    .buttonStyle(.borderless)
            }
        }
    }
}

/// "Needs you" (Sync feedback level 3): a failed read with Retry, or an
/// account whose sign-in needs the person, opening its settings.
struct SyncProblemBanner: View {
    let problem: SyncProblem
    @Environment(AppModel.self) private var model
    @Environment(\.push) private var push

    var body: some View {
        HStack(alignment: .top, spacing: Theme.space5) {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.danger)
            Text(problem.text).font(.subheadline).foregroundStyle(Theme.ink)
            Spacer()
            if let profile = problem.signIn {
                Button {
                    push(SettingsRoute.account(profile))
                } label: {
                    Text(t("syncStatus.signIn")).font(.subheadline.weight(.semibold))
                }
            } else if let retry = problem.retry {
                Button(t("common.retry")) { model.retrySync(retry) }
                    .font(.subheadline.weight(.semibold))
            }
        }
        .padding(Theme.space7)
        .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }
}

/// First launch after signing in: Today has nothing yet, so say what is
/// being read instead of showing a blank page.
struct FirstLoadCard: View {
    let snapshot: SyncSnapshot

    var body: some View {
        HStack(spacing: Theme.space7) {
            SyncIndicatorView(indicator: snapshot.indicator ?? .spinner(emphasized: true))
            VStack(alignment: .leading, spacing: Theme.space2) {
                Text(t("syncStatus.loadingSchoolData")).font(.body.weight(.semibold)).foregroundStyle(Theme.ink)
                Text(snapshot.line).font(.footnote).foregroundStyle(Theme.muted).lineLimit(2)
            }
        }
        .padding(Theme.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }
}
