({ page, cmid, module }) => {
  if (!page.body?.classList.contains(`cmid-${cmid}`)) return null;
  if (module === "assign") {
    // Read through the classes Moodle puts on the status table cells.
    const table = page.querySelector(".submissionstatustable");
    if (!table) return null;
    const has = (name) => Boolean(table.querySelector(`td.${name}`));
    const submission = has("submissiongraded") ? "graded"
      : has("submissionstatussubmitted") ? "submitted" : "open";
    // Released feedback leads with the grade ("100.00 / 100.00", or a scale
    // item) as plain text. Its rows carry no classes; without a grade the
    // first row is the grader or a comment, which both hold markup.
    const first = page.querySelector(".feedback .feedbacktable table tr td");
    const text = first && !first.children.length
      ? first.textContent.replace(/\s+/g, " ").trim() : "";
    const grade = text && text !== "-" ? text : null;
    return { cmid, submission, grade, late: has("latesubmission") || has("overdue") };
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
    // The final grade is a sentence ("…最終評点は 100 / 100 です。"); only its
    // "grade / maximum" pair is read. Absent until the grade is released,
    // and without numbers while answers still need grading.
    const pair = finished && [...page.querySelectorAll("#feedback h3")]
      .map((heading) => heading.textContent.match(/(\d[\d.,]*)\s*\/\s*(\d[\d.,]*)/))
      .find(Boolean);
    const grade = pair ? `${pair[1]} / ${pair[2]}` : null;
    return { cmid, submission: grade ? "graded" : finished ? "submitted" : "open",
      grade, late: false };
  }
  return null;
}
