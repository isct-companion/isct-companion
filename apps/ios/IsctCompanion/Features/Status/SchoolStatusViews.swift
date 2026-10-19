import IsctCore
import Pow
import SwiftUI

/// Words for Core's school status (`school_status.rs`). Core decides what is
/// shown and in which order; these only name it.
enum SchoolStatusText {
    static func source(_ site: SchoolSite) -> String {
        site == .kyomu ? "教務Web" : "LMS"
    }

    static func waiting(_ reason: WaitReason) -> String {
        switch reason {
        case .queued: return "順番を待っています"
        case .offline: return "接続を待っています"
        case .cellular: return "Wi-Fi を待っています"
        case .background: return "アプリを開くと再開します"
        case .schoolVisible: return "学校のページを閉じると再開します"
        case .suspended: return "一時停止中"
        case .other: return "待機中"
        }
    }

    /// A step with nothing reported yet reads as its label alone.
    static func step(_ step: PullStep) -> String {
        let label: String = switch step.kind {
        case .catalog: "履修中の授業を読み込み中"
        case .course: "授業を読み込み中"
        case .material: "資料を読み込み中"
        case .assignment: "課題を確認中"
        case .timetable: "時間割を準備中"
        case .saving: "変更を保存中"
        case .kyomu: "教務Webを読み込み中"
        }
        guard step.total > 0 || !step.courseTitle.isEmpty || !step.itemTitle.isEmpty else { return label }
        let context = [step.courseTitle, step.itemTitle].filter { !$0.isEmpty }.joined(separator: " · ")
        return [label, context, "\(step.completed)/\(step.total)"].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    static func problem(_ problem: SchoolProblem, site: SchoolSite) -> String {
        let name = source(site)
        switch problem {
        case let .maintenance(detail): return ["\(name) はメンテナンス中です", detail].joined(separator: " · ")
        case let .failed(detail): return ["\(name) を更新できませんでした", detail].compactMap { $0 }.joined(separator: " · ")
        case let .partial(detail): return ["\(name) の一部を読み込めませんでした", detail].compactMap { $0 }.joined(separator: " · ")
        }
    }

    static func signIn(_ state: SignInState) -> String {
        switch state {
        case .connected: return "ログイン済み"
        case let .step(step):
            let detail: String? = switch step {
            case .agree: "利用規約に同意中"
            case .identifier: "ID を入力中"
            case .password: "パスワードを確認中"
            case .method: "認証方法を選択中"
            case .otp: "ワンタイムパスワードを確認中"
            case .matrix: "マトリクスコードを確認中"
            case .other: nil
            }
            return ["ログイン中", detail].compactMap { $0 }.joined(separator: " · ")
        case .needsUser: return "操作が必要です"
        case let .failed(message): return message
        }
    }
}

extension SchoolHeadline {
    /// The ring's fill, when a pull is underway.
    func ringProgress(_ status: SchoolStatus) -> Double? {
        switch self {
        case let .busy(_, progress, _, _, _): return Double(progress) / 100
        case let .background(site, _):
            switch status.sources.first(where: { $0.site == site })?.activity {
            case let .running(_, progress, _)?: return Double(progress) / 100
            default: return 0.05
            }
        case .idle, .problem: return nil
        }
    }

    /// One line for the first-load hint and the ring's accessibility label.
    var line: String {
        switch self {
        case .idle: return "最新です"
        case let .busy(site, _, step, waiting, _):
            return "\(SchoolStatusText.source(site)) · \(waiting.map(SchoolStatusText.waiting) ?? SchoolStatusText.step(step))"
        case let .background(site, waiting):
            return "\(SchoolStatusText.source(site)) · \(waiting.map(SchoolStatusText.waiting) ?? "バックグラウンドで更新中")"
        case let .problem(site, problem): return SchoolStatusText.problem(problem, site: site)
        }
    }
}

/// The sync ring beside the settings button (`docs/ux/mobile.md`, Sync
/// feedback): the shape of desktop's `PullRing`. Hidden when nothing runs.
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

/// The ring as a button that opens the connection sheet. When a pull the
/// person started finishes cleanly, a check pops in its place for a moment.
struct SyncRingButton: View {
    @Environment(AppModel.self) private var model
    @State private var showsSheet = false
    @State private var finished = 0
    @State private var showsDone = false

    var body: some View {
        ZStack {
            if let status = model.schoolStatus, let progress = status.headline.ringProgress(status) {
                Button {
                    showsSheet = true
                } label: {
                    SyncRing(progress: progress, emphasized: status.headline.isBusy)
                        .frame(width: 40, height: 40)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel(status.headline.line)
            } else if showsDone {
                Image(systemName: "checkmark.circle.fill")
                    .font(.title3)
                    .foregroundStyle(Theme.ok)
                    .frame(width: 40, height: 40)
                    .transition(.movingParts.pop(Theme.ok))
                    .accessibilityLabel("更新しました")
            }
        }
        .animation(.snappy, value: showsDone)
        .sensoryFeedback(.success, trigger: finished)
        .sheet(isPresented: $showsSheet) { SchoolStatusSheet() }
        .onChange(of: model.schoolStatus?.headline) { old, new in
            guard old?.isBusy == true, case .idle? = new else { return }
            finished += 1
            showsDone = true
            Task {
                try? await Task.sleep(for: .seconds(1.6))
                showsDone = false
            }
        }
    }
}

extension SchoolHeadline {
    var isBusy: Bool {
        if case .busy = self { return true }
        return false
    }
}

/// The connection sheet: one row per source, as Core lists them.
struct SchoolStatusSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.schoolStatus?.sources ?? [], id: \.site) { source in
                    SourceRow(source: source)
                }
            }
            .navigationTitle("学校との接続")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("閉じる") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium])
    }
}

private struct SourceRow: View {
    let source: SchoolSourceStatus

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(SchoolStatusText.source(source.site)).font(.body.weight(.semibold))
                Spacer()
                if case let .running(_, progress, _) = source.activity {
                    SyncRing(progress: Double(progress) / 100)
                }
            }
            Text(activity).font(.subheadline).foregroundStyle(tint)
            if let signIn = source.signIn, signIn != .connected {
                Text(SchoolStatusText.signIn(signIn)).font(.footnote).foregroundStyle(Theme.muted)
            }
        }
        .padding(.vertical, 4)
    }

    private var activity: String {
        switch source.activity {
        case .idle:
            guard let last = source.lastUpdated else { return "まだ更新していません" }
            return "\(Format.due(last)) 更新"
        case let .waiting(reason, _): return SchoolStatusText.waiting(reason)
        case let .running(_, _, step): return SchoolStatusText.step(step)
        case let .problem(problem): return SchoolStatusText.problem(problem, site: source.site)
        }
    }

    private var tint: Color {
        if case .problem = source.activity { return Theme.danger }
        return Theme.muted
    }
}

/// "Needs you" (Sync feedback level 3): Core's problem, once nothing runs.
struct SchoolProblemBanner: View {
    let site: SchoolSite
    let problem: SchoolProblem
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: isMaintenance ? "wrench.and.screwdriver.fill" : "exclamationmark.circle.fill")
                .foregroundStyle(isMaintenance ? Theme.warn : Theme.danger)
            Text(SchoolStatusText.problem(problem, site: site)).font(.subheadline).foregroundStyle(Theme.ink)
            Spacer()
            if !isMaintenance {
                Button("再試行") { model.refreshFromSchool() }
                    .font(.subheadline.weight(.semibold))
            }
        }
        .padding(14)
        .background(
            isMaintenance ? Theme.warnSoft : Theme.dangerSoft,
            in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }

    private var isMaintenance: Bool {
        if case .maintenance = problem { return true }
        return false
    }
}

/// First launch after signing in: Today has nothing yet, so say what is
/// being read instead of showing a blank page.
struct FirstLoadCard: View {
    let status: SchoolStatus

    var body: some View {
        HStack(spacing: 14) {
            SyncRing(progress: status.headline.ringProgress(status) ?? 0.05)
            VStack(alignment: .leading, spacing: 4) {
                Text("学校のデータを読み込んでいます").font(.body.weight(.semibold)).foregroundStyle(Theme.ink)
                Text(status.headline.line).font(.footnote).foregroundStyle(Theme.muted).lineLimit(2)
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }
}
