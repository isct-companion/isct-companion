({ page, cmid, module }) => {
  if (!page.body?.classList.contains(`cmid-${cmid}`)) return null;
  if (module === "assign") {
    // Read through the classes Moodle puts on the status table cells.
    const table = page.querySelector(".submissionstatustable");
    if (!table) return null;
    const has = (name) => Boolean(table.querySelector(`td.${name}`));
    const submission = has("submissiongraded") ? "graded"
      : has("submissionstatussubmitted") ? "submitted" : "open";
    return { cmid, submission, late: has("latesubmission") || has("overdue") };
  }
  if (module === "quiz") {
    // The attempt box is always rendered on the quiz view page; without it
    // the page is not one we understand.
    if (!page.querySelector(".quizattempt")) return null;
    // Each attempt is summarised as a table. Every attempt has its state and
    // start rows; only a finished one adds its completion time and duration,
    // so this stays independent of the interface language.
    const finished = [...page.querySelectorAll("table.quizreviewsummary")]
      .some((table) => table.querySelectorAll("tr").length >= 3);
    return { cmid, submission: finished ? "submitted" : "open", late: false };
  }
  return null;
}
