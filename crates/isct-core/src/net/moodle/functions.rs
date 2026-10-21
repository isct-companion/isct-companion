//! The closed, read-only list of Moodle functions this app calls.
//!
//! The school's mobile service also grants write functions (assignment
//! submission, `mod_ttws_*` evaluations, …). Both transports take a
//! [`MoodleFunction`], never a function name string, so a write cannot be
//! expressed anywhere in the client.

/// One read-only Moodle external function.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MoodleFunction {
    SiteInfo,
    EnrolledCoursesByTimeline,
    CourseContents,
    /// AJAX only: the mobile service does not include it, so the token
    /// transport reads `CourseContents` instead.
    CourseformatState,
    ActionEventsByTimesort,
    CalendarUpcomingView,
    CalendarMonthlyView,
    AssignGetAssignments,
    AssignGetSubmissionStatus,
    QuizGetQuizzesByCourses,
    QuizGetUserAttempts,
    QuizGetUserBestGrade,
    ForumGetForumsByCourses,
    ForumGetForumDiscussions,
    PageGetPagesByCourses,
    ResourceGetResourcesByCourses,
    FolderGetFoldersByCourses,
    UrlGetUrlsByCourses,
}

impl MoodleFunction {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SiteInfo => "core_webservice_get_site_info",
            Self::EnrolledCoursesByTimeline => {
                "core_course_get_enrolled_courses_by_timeline_classification"
            }
            Self::CourseContents => "core_course_get_contents",
            Self::CourseformatState => "core_courseformat_get_state",
            Self::ActionEventsByTimesort => "core_calendar_get_action_events_by_timesort",
            Self::CalendarUpcomingView => "core_calendar_get_calendar_upcoming_view",
            Self::CalendarMonthlyView => "core_calendar_get_calendar_monthly_view",
            Self::AssignGetAssignments => "mod_assign_get_assignments",
            Self::AssignGetSubmissionStatus => "mod_assign_get_submission_status",
            Self::QuizGetQuizzesByCourses => "mod_quiz_get_quizzes_by_courses",
            Self::QuizGetUserAttempts => "mod_quiz_get_user_attempts",
            Self::QuizGetUserBestGrade => "mod_quiz_get_user_best_grade",
            Self::ForumGetForumsByCourses => "mod_forum_get_forums_by_courses",
            Self::ForumGetForumDiscussions => "mod_forum_get_forum_discussions",
            Self::PageGetPagesByCourses => "mod_page_get_pages_by_courses",
            Self::ResourceGetResourcesByCourses => "mod_resource_get_resources_by_courses",
            Self::FolderGetFoldersByCourses => "mod_folder_get_folders_by_courses",
            Self::UrlGetUrlsByCourses => "mod_url_get_urls_by_courses",
        }
    }

    /// Offered by the `moodle_mobile_app` service (REST + token).
    pub const fn via_rest(self) -> bool {
        !matches!(self, Self::CourseformatState)
    }

    /// Marked `ajax => true` and therefore callable with a session cookie and
    /// sesskey. Measured 2026-10-20: the module getters and
    /// `core_course_get_contents` answer `servicenotavailable` there.
    pub const fn via_ajax(self) -> bool {
        matches!(
            self,
            Self::EnrolledCoursesByTimeline
                | Self::CourseformatState
                | Self::ActionEventsByTimesort
                | Self::CalendarUpcomingView
                | Self::CalendarMonthlyView
        )
    }
}
