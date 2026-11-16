import IsctCore
import SwiftUI

/// The years Core accepts, newest first.
func examYears(_ limits: ExamLimits) -> [UInt32] {
    guard limits.latestYear >= limits.earliestYear else { return [] }
    return Array((limits.earliestYear...limits.latestYear).reversed())
}

/// Asks for documents of one subject; every field but the subject is optional.
struct ExamRequestForm: View {
    let subject: ExamSubjectPage
    let choices: ExamChoices
    let busy: Bool
    let problem: String?
    let submit: (ExamRequestDraft) async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var classId: UInt32?
    @State private var year: UInt32?
    @State private var occasion: ExamOccasion?
    @State private var role: ExamRole?
    @State private var description = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if subject.rows.count > 1 {
                        Picker(t("examShare.class"), selection: $classId) {
                            Text(t("examShare.allClasses")).tag(nil as UInt32?)
                            ForEach(subject.rows, id: \.classIds) { row in
                                if let id = row.classIds.first {
                                    Text((row.labels + row.teachers.map(\.name) + [row.taughtIn])
                                        .filter { !$0.isEmpty }.joined(separator: " · ")).tag(Optional(id))
                                }
                            }
                        }
                    }
                    Picker(t("examShare.year"), selection: $year) {
                        Text(t("examShare.allYears")).tag(nil as UInt32?)
                        ForEach(examYears(choices.limits), id: \.self) { Text(verbatim: String($0)).tag(Optional($0)) }
                    }
                    Picker(t("examShare.occasion"), selection: $occasion) {
                        Text(t("examShare.allOccasions")).tag(nil as ExamOccasion?)
                        ForEach(choices.requestOccasions, id: \.self) { Text($0.label).tag(Optional($0)) }
                    }
                    Picker(t("examShare.upload.role"), selection: $role) {
                        Text(t("examShare.wantedRole")).tag(nil as ExamRole?)
                        ForEach(choices.requestRoles, id: \.self) { Text($0.label).tag(Optional($0)) }
                    }
                } header: {
                    Text(subject.subject.name)
                }
                Section(t("examShare.description")) {
                    TextField(t("examShare.description"), text: $description, axis: .vertical).lineLimit(3...6)
                }
                if let problem {
                    Section { Text(problem).foregroundStyle(Theme.danger) }
                }
            }
            .themedList()
            .navigationTitle(t("examShare.newRequest"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy {
                        ProgressView()
                    } else {
                        Button(t("common.submit")) {
                            let draft = ExamRequestDraft(
                                subjectCode: subject.subject.code, classId: classId, academicYear: year,
                                occasion: occasion, wantedRole: role,
                                description: description.trimmingCharacters(in: .whitespacesAndNewlines))
                            Task { await submit(draft) }
                        }
                        .disabled(description.utf16.count > choices.limits.requestDescription)
                    }
                }
            }
        }
    }
}

struct ExamReportForm: View {
    let reasons: [ExamReportReason]
    let maxDetail: UInt32
    let busy: Bool
    let problem: String?
    let submit: (ExamReportReason, String) async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var reason: ExamReportReason
    @State private var detail = ""

    init(reasons: [ExamReportReason], maxDetail: UInt32, busy: Bool, problem: String?,
         submit: @escaping (ExamReportReason, String) async -> Void) {
        self.reasons = reasons
        self.maxDetail = maxDetail
        self.busy = busy
        self.problem = problem
        self.submit = submit
        _reason = State(initialValue: reasons.first ?? .other)
    }

    var body: some View {
        NavigationStack {
            Form {
                Picker(t("examShare.report"), selection: $reason) {
                    ForEach(reasons, id: \.self) { Text($0.label).tag($0) }
                }
                .pickerStyle(.inline)
                Section(t("examShare.description")) {
                    TextField(t("examShare.description"), text: $detail, axis: .vertical).lineLimit(3...6)
                }
                if let problem {
                    Section { Text(problem).foregroundStyle(Theme.danger) }
                }
            }
            .themedList()
            .navigationTitle(t("examShare.report"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.cancel")) { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy {
                        ProgressView()
                    } else {
                        Button(t("common.submit")) {
                            Task { await submit(reason, detail.trimmingCharacters(in: .whitespacesAndNewlines)) }
                        }
                        .disabled(detail.utf16.count > maxDetail)
                    }
                }
            }
        }
    }
}

// Labels for the values Core's `ExamChoices` offers. Each case names its
// key, so the locale checker sees every one.

extension ExamRole {
    var label: String {
        switch self {
        case .question: t("examShare.role.question")
        case .answer: t("examShare.role.answer")
        case .both: t("examShare.role.both")
        case .notes: t("examShare.role.notes")
        case .other: t("examShare.role.other")
        }
    }
}

extension ExamOccasion {
    var label: String {
        switch self {
        case .unknown: t("examShare.occasion.unknown")
        case .final: t("examShare.occasion.final")
        case .midterm: t("examShare.occasion.midterm")
        case .makeup: t("examShare.occasion.makeup")
        case .quiz: t("examShare.occasion.quiz")
        case .check: t("examShare.occasion.check")
        case .report: t("examShare.occasion.report")
        case .exercise: t("examShare.occasion.exercise")
        case .textbook: t("examShare.occasion.textbook")
        case .mock: t("examShare.occasion.mock")
        case .notApplicable: t("examShare.occasion.none")
        }
    }
}

extension ExamProvenance {
    var label: String {
        switch self {
        case .original: t("examShare.upload.original")
        case .reconstructed: t("examShare.upload.reconstructed")
        }
    }
}

extension ExamAnswerSource {
    var label: String {
        switch self {
        case .unknown: t("examShare.upload.unknown")
        case .official: t("examShare.upload.official")
        case .student: t("examShare.upload.student")
        }
    }
}

extension ExamReportReason {
    var label: String {
        switch self {
        case .copyright: t("examShare.reason.copyright")
        case .inappropriate: t("examShare.reason.inappropriate")
        case .wrongAnswer: t("examShare.reason.wrongAnswer")
        case .brokenFile: t("examShare.reason.brokenFile")
        case .other: t("examShare.reason.other")
        }
    }
}
