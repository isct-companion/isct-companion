import IsctCore
import SwiftUI

/// This quarter's enrolled courses, read from the device. A course opens its
/// subject only when tapped.
struct CurrentCoursesSection: View {
    let open: (String) -> Void
    @Environment(AppModel.self) private var model
    @State private var list: ExamCourseList?
    @State private var failed = false
    @State private var retry = 0

    private struct Read: Equatable {
        let revision: String
        let retry: Int
    }

    var body: some View {
        Section(t("examShare.currentCourses")) {
            if let list {
                if list.courses.isEmpty {
                    Text(t("examShare.noCurrentCourses")).foregroundStyle(Theme.muted)
                }
                ForEach(list.courses, id: \.courseId) { course in
                    Button { open(course.courseId) } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(course.title).foregroundStyle(Theme.ink)
                            if !course.instructor.isEmpty {
                                Text(course.instructor).font(.subheadline).foregroundStyle(Theme.muted)
                            }
                            Text(course.subjectCodes.isEmpty
                                 ? t("examShare.noSubjectCode") : course.subjectCodes.joined(separator: " / "))
                                .font(.footnote.monospaced())
                                .foregroundStyle(Theme.muted)
                        }
                    }
                    .disabled(course.subjectCodes.isEmpty)
                }
            } else if failed {
                Button(t("common.retry")) { retry += 1 }
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
        .task(id: Read(revision: model.contentRevision, retry: retry)) {
            failed = false
            let answer = await model.examCourses()
            guard !Task.isCancelled else { return }
            list = answer
            failed = answer == nil
        }
    }
}
