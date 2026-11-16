import IsctCore
import SwiftUI

/// This quarter's enrolled courses, read from the device. A course opens its
/// subject only when tapped. The screen loads `list`; a `Section` inside a
/// `List` would run a `.task` for each of its rows.
struct CurrentCoursesSection: View {
    let list: ExamCourseList?
    let failed: Bool
    let retry: () -> Void
    let open: (ExamCourse) -> Void

    var body: some View {
        Section(t("examShare.currentCourses")) {
            if let list {
                if list.courses.isEmpty {
                    Text(t("examShare.noCurrentCourses")).foregroundStyle(Theme.muted)
                }
                ForEach(list.courses, id: \.courseId) { course in
                    Button { open(course) } label: {
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
                Button(t("common.retry"), action: retry)
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
    }
}
