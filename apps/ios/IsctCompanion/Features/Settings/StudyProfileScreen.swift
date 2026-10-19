import IsctCore
import SwiftUI

/// 学習プロフィール: department, admission year and degree programme, the
/// desktop's Study settings. The department is `settings.departmentId`; the
/// year and programme go through Core's `setStudyProfile`, as on desktop. Each
/// choice saves at once; a programme picked before the year waits for it.
struct StudyProfileScreen: View {
    @Environment(AppModel.self) private var model
    @State private var departments: [FinderDepartment] = []
    @State private var stored: AppModel.StudyProfile?
    @State private var year: Int32?
    @State private var stage: StudyStage = .unknown
    @State private var currentYear: Int32?
    @State private var error: String?

    private static var stages: [(StudyStage, String)] {
        [
            (.unknown, t("Not set")), (.undergraduate, t("Undergraduate")), (.master, t("Master")),
            (.professional, t("Professional degree")), (.doctoral, t("Doctoral")),
        ]
    }

    var body: some View {
        Form {
            Section {
                Picker(t("Department"), selection: departmentBinding) {
                    Text(t("Not set")).tag("")
                    ForEach(departments, id: \.id) { department in
                        Text(Localizer.shared.pick(department.name, content: model.records.settings) ?? department.id).tag(department.id)
                    }
                    // A stored id this list does not hold still shows.
                    if !departmentId.isEmpty, !departments.contains(where: { $0.id == departmentId }) {
                        Text(departmentId).tag(departmentId)
                    }
                }
                Picker(t("Admission year"), selection: Binding(get: { year }, set: { choose(year: $0, stage: stage) })) {
                    Text(t("Not set")).tag(Int32?.none)
                    ForEach(years, id: \.self) { Text(t("Entered {{year}}", ["year": String($0)])).tag(Int32?.some($0)) }
                }
                Picker(t("Degree stage"), selection: Binding(get: { stage }, set: { choose(year: year, stage: $0) })) {
                    ForEach(Self.stages, id: \.0) { Text($0.1).tag($0.0) }
                }
            } footer: {
                if let error {
                    Text(error).foregroundStyle(Theme.danger)
                } else {
                    Text(t("Used for course suggestions and the study guide. Syncs with the desktop app."))
                }
            }
        }
        .navigationTitle(t("Study profile"))
        .navigationBarTitleDisplayMode(.inline)
        .task { departments = await model.departments() }
        .task { currentYear = await model.academicYear(of: Date()) }
        .task(id: model.contentRevision) { await load() }
    }

    /// The stored department, matched like the desktop's `departmentById`
    /// (a plain id or a subscription-scoped "2026:cs").
    private var departmentId: String {
        let raw = model.records.settings["departmentId"] as? String ?? ""
        return departments.first { raw == $0.id || raw.hasSuffix(":\($0.id)") }?.id ?? raw
    }

    private var departmentBinding: Binding<String> {
        Binding(
            get: { departmentId },
            set: { id in
                Task {
                    await model.updateSettings { settings in
                        if id.isEmpty { settings.removeValue(forKey: "departmentId") } else { settings["departmentId"] = id }
                    }
                }
            })
    }

    /// The desktop's choices: the ten years up to next year, plus a stored one.
    private var years: [Int32] {
        let now = currentYear ?? Int32(Calendar.tokyo.component(.year, from: Date()))
        var list = (0..<10).map { now + 1 - Int32($0) }
        if let year, !list.contains(year) { list.append(year) }
        return list.sorted(by: >)
    }

    private func load() async {
        guard let profile = await model.studyProfile() else { return }
        stored = profile
        year = profile.admissionYear
        stage = profile.enrollments.last?.stage ?? .unknown
    }

    /// Builds the profile the desktop's save would: the programme replaces the
    /// latest enrollment's stage, and enrollments without a term start in the
    /// admission year's 1Q.
    private func choose(year: Int32?, stage: StudyStage) {
        self.year = year
        self.stage = stage
        error = nil
        var enrollments = stored?.enrollments ?? []
        if enrollments.isEmpty {
            if stage != .unknown {
                enrollments.append(StudyEnrollment(stage: stage, entryKind: .unknown, entryGrade: nil, admittedTerm: ""))
            }
        } else {
            enrollments[enrollments.count - 1].stage = stage
        }
        if enrollments.contains(where: { $0.admittedTerm.isEmpty }) {
            guard let year else {
                error = t("Choose an admission year.")
                return
            }
            for index in enrollments.indices where enrollments[index].admittedTerm.isEmpty {
                enrollments[index].admittedTerm = "\(year)-1Q"
            }
        }
        enrollments.sort { $0.admittedTerm < $1.admittedTerm }
        let profile = AppModel.StudyProfile(admissionYear: year, enrollments: enrollments)
        Task {
            do {
                try await model.setStudyProfile(profile)
                stored = profile
            } catch {
                self.error = AppModel.describe(error)
                await load()
            }
        }
    }
}
