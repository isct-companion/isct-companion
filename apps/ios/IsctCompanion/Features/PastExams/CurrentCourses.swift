import IsctCore
import SwiftUI

/// This quarter's enrolled courses, read from the device; each pushes its
/// subject, which Core resolves only when opened. The screen loads `list`; a
/// `Section` inside a `List` would run a `.task` for each of its rows.
struct CurrentCoursesSection: View {
    let list: ExamCourseList?
    let failed: Bool
    let retry: () -> Void

    var body: some View {
        Section(t("examShare.currentCourses")) {
            if let list {
                if list.courses.isEmpty {
                    Text(t("examShare.noCurrentCourses")).foregroundStyle(Theme.muted)
                }
                ForEach(list.courses, id: \.courseId) { course in
                    if course.subjectCodes.isEmpty {
                        ExamListRow(title: course.title, meta: [t("examShare.noSubjectCode")])
                            .opacity(0.6)
                    } else {
                        NavigationLink(value: ExamSubjectRoute(source: .course(id: course.courseId), title: course.title)) {
                            ExamListRow(title: course.title, meta: [
                                course.subjectCodes.joined(separator: " / "), course.instructor,
                            ])
                        }
                    }
                }
            } else if failed {
                Button(t("common.retry"), action: retry)
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
    }
}
