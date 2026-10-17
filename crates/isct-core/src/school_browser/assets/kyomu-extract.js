(() => {
  const clean = (value) =>
    String(value || "")
      .normalize("NFKC")
      .replace(/\s+/g, " ")
      .trim();
  const text = (root, selector) =>
    clean(root.querySelector(selector)?.textContent);
  const sourceUrl = location.href;
  const allowed = new Map([
    [
      "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
      "registrations",
    ],
    [
      "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx",
      "declared",
    ],
    [
      "/Titech/Student/%E5%AD%A6%E7%94%9F%E6%83%85%E5%A0%B1/PID4_2.aspx",
      "profile",
    ],
    [
      "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
      "grades",
    ],
    [
      "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
      "requirements",
    ],
    ["/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx", "gpa"],
  ]);
  const kind = [...allowed].find(
    ([path]) => path.toLowerCase() === location.pathname.toLowerCase(),
  )?.[1];
  const result = (status, data = {}, message = "") => ({
    status,
    sourceUrl,
    kind: kind || "unknown",
    data,
    ...(message ? { message } : {}),
  });
  const maintenance = globalThis.__isctKyomuMaintenance?.(document);
  if (location.origin === "https://kyomu0.gakumu.titech.ac.jp" && maintenance)
    return result("maintenance", {}, maintenance);
  const loginForm =
    document.querySelector('form[action*="login" i]') ||
    document.querySelector('input[type="password"]')?.closest("form");
  const knownLoginHost = [
    "kyomu0.gakumu.titech.ac.jp",
    "portal.titech.ac.jp",
    "rp.nap.gsic.titech.ac.jp",
  ].includes(location.hostname);
  if (location.protocol === "https:" && knownLoginHost && loginForm)
    return result("login");
  if (
    location.protocol !== "https:" ||
    location.hostname !== "kyomu0.gakumu.titech.ac.jp" ||
    !kind
  )
    return result("error", {}, "The source page is not approved.");
  if (
    document.readyState !== "complete" ||
    document.querySelector('[aria-busy="true"], [data-loading="true"]')
  )
    return result("error", {}, "The school page is still loading.");

  // Only an explicit message in the page's error region proves that a page is
  // not offered to this account. A missing table or unfamiliar DOM is an error.
  const unavailableMessage = [
    ...document.querySelectorAll(
      "#ctl00_ContentPlaceHolder1_lblError, .errorMessage, .validation-summary-errors, [role=alert]",
    ),
  ]
    .map((node) => clean(node.textContent))
    .find((value) =>
      /(?:このページ|本機能|当機能).*(?:利用できません|対象外)/.test(value),
    );
  if (unavailableMessage) return result("unavailable", {}, unavailableMessage);

  const explicitEmpty = (root) =>
    Boolean(root && /対象データはありません/.test(clean(root.textContent)));

  // Expand a table to a rectangular text matrix. Values covered by rowspan or
  // colspan are repeated so semantic header lookup works for multi-row heads.
  const matrix = (table) => {
    const output = [];
    [...table.rows].forEach((row, rowIndex) => {
      output[rowIndex] ||= [];
      let column = 0;
      [...row.cells].forEach((cell) => {
        while (output[rowIndex][column] !== undefined) column += 1;
        const value = clean(cell.textContent);
        const rows = Math.max(1, Number(cell.rowSpan) || 1);
        const columns = Math.max(1, Number(cell.colSpan) || 1);
        for (let down = 0; down < rows; down += 1) {
          output[rowIndex + down] ||= [];
          for (let across = 0; across < columns; across += 1)
            output[rowIndex + down][column + across] = value;
        }
        column += columns;
      });
    });
    const width = output.reduce((size, row) => Math.max(size, row.length), 0);
    return output.map((row) =>
      Array.from({ length: width }, (_, index) => row[index] || ""),
    );
  };
  const tableData = (table, knownHeaderRows = 0) => {
    const rows = matrix(table);
    const inferredHeaders = [...table.rows].findIndex((row) =>
      row.querySelector("td"),
    );
    const headerRows =
      knownHeaderRows ||
      table.tHead?.rows.length ||
      (inferredHeaders > 0 ? inferredHeaders : 1);
    const header = rows.slice(0, headerRows);
    const width = rows.reduce((size, row) => Math.max(size, row.length), 0);
    const headers = Array.from({ length: width }, (_, column) => {
      const parts = header.map((row) => row[column]).filter(Boolean);
      return [...new Set(parts)].join(" / ");
    });
    return { headers, rows: rows.slice(headerRows) };
  };
  const column = (headers, label) =>
    headers.findIndex((header) => header.includes(label));
  const headingBefore = (table) => {
    const caption = clean(table.caption?.textContent);
    if (caption) return caption;
    const headings = [...document.querySelectorAll("h1,h2,h3,h4,h5")].filter(
      (heading) =>
        Boolean(
          heading.compareDocumentPosition(table) &
          Node.DOCUMENT_POSITION_FOLLOWING,
        ),
    );
    return clean(headings.at(-1)?.textContent);
  };
  const complete = (shape, data, empty = false) =>
    result("ok", { complete: true, shape, empty, ...data });
  const incomplete = (message) => result("error", {}, message);

  const requirement = (raw) => {
    const match = clean(raw).match(/^([A-Z])\s*\((◎|○|選択)\)$/);
    return match
      ? {
          category: match[1],
          mark:
            match[2] === "◎"
              ? "required"
              : match[2] === "○"
                ? "recommended"
                : "elective",
        }
      : { raw: clean(raw) };
  };
  const lines = (element) => {
    if (!element) return [];
    const copy = element.cloneNode(true);
    copy.querySelectorAll("br").forEach((br) => br.replaceWith("\n"));
    return copy.textContent.split("\n").map(clean).filter(Boolean);
  };
  const schedule = (rows) =>
    rows.flatMap((part) => {
      const match = part.match(
        /^([1-4])(?:\s*[~〜-]\s*([1-4]))?Q[:：]\s*([月火水木金土日])\s*(\d+)\s*[-–〜~]\s*(\d+)/,
      );
      if (!match) return [{ raw: part }];
      const from = Number(match[1]);
      const to = Number(match[2] || match[1]);
      const first = Number(match[4]);
      const last = Number(match[5]);
      const periods = Array.from(
        { length: last - first + 1 },
        (_, index) => first + index,
      );
      return Array.from({ length: to - from + 1 }, (_, index) => ({
        quarter: `${from + index}Q`,
        day: match[3],
        periods,
        room: clean(part.slice(match[0].length)).replace(/^\((.*)\)$/, "$1"),
      }));
    });

  if (kind === "profile") {
    const root = document.querySelector("#ctl00_ContentPlaceHolder1_divMain");
    if (!root) return incomplete("The academic profile page shape is unknown.");
    const heading = [...root.querySelectorAll("h2")].find((node) =>
      /【現在】/.test(node.textContent),
    );
    const next = heading?.nextElementSibling;
    const table = next?.matches("table") ? next : next?.querySelector("table");
    if (!table?.matches("table"))
      return incomplete("The current academic profile is missing.");
    // Only academic identity leaves the page. Names, student numbers, contacts
    // and other personal rows are deliberately excluded.
    const fields = Object.fromEntries(
      [...table.rows].map((row) => [
        clean(row.cells[0]?.textContent),
        clean(row.cells[1]?.textContent),
      ]),
    );
    if (
      !fields["所属"] ||
      !/^\d{4}\/\d{2}\/\d{2}$/.test(fields["入学年月日"] || "")
    )
      return incomplete("The academic profile fields are incomplete.");
    return complete("academic-profile", {
      department: fields["所属"],
      admittedOn: fields["入学年月日"].replaceAll("/", "-"),
      programme: clean(heading.textContent).replace("【現在】", "").trim(),
      entryCategory: fields["入学試験区分"] || "",
    });
  }

  if (kind === "registrations") {
    const root = document.querySelector("#divSinkokuStatus");
    if (!root) return incomplete("The registration page shape is unknown.");
    const timetable = document.querySelector("table#timetable");
    const listTables = [
      ...document.querySelectorAll('table[id^="kamokuUnit_"][id$="_gvKamoku"]'),
    ];
    if (!timetable || !listTables.length)
      return incomplete("The registration course area is missing.");
    const rawState = text(
      document,
      "#ctl00_ContentPlaceHolder1_ctl00_lblStatus2",
    );
    const stateMap = {
      一時保存: "saved",
      要エラーチェック: "needs-check",
      本登録可能: "registrable",
      承認待ち: "needs-approval",
      要承認申請: "needs-approval",
      "承認待ち / 要承認申請": "needs-approval",
      "承認待ち/要承認申請": "needs-approval",
      本登録不可: "rejected",
      本登録: "registered",
      本登録済: "registered",
    };
    const seen = new Set();
    const courses = [];
    for (const block of document.querySelectorAll(
      "table#timetable [data-jwc]",
    )) {
      const offeringId = clean(block.getAttribute("data-jwc"));
      if (!/^\d{9}$/.test(offeringId))
        return incomplete("A registration course identifier is invalid.");
      if (seen.has(offeringId)) {
        const course = courses.find(
          (course) => course.offeringId === offeringId,
        );
        for (const entry of schedule(
          lines(
            block.querySelector(".ttYoubi") || block.querySelector(".ttYoubi2"),
          ),
        )) {
          const twin = course.schedule.find(
            (row) =>
              row.quarter === entry.quarter &&
              row.day === entry.day &&
              JSON.stringify(row.periods) === JSON.stringify(entry.periods),
          );
          if (twin && entry.room) twin.room = entry.room;
        }
        continue;
      }
      seen.add(offeringId);
      const classes = [
        ...block.classList,
        ...(block.closest("td")?.classList || []),
      ];
      const courseState = classes.includes("timetableError")
        ? "rejected"
        : classes.some((name) =>
              [
                "timetableAlert1",
                "timetableAlert2",
                "timetableAlert3",
                "timetableAlert22",
              ].includes(name),
            )
          ? "needs-approval"
          : classes.includes("timetableUnchecked")
            ? "needs-check"
            : "registrable";
      const times = lines(
        block.querySelector(".ttYoubi") || block.querySelector(".ttYoubi2"),
      );
      courses.push({
        offeringId,
        title: text(block, ".ttKamokuName a"),
        code: text(block, ".ttSinkokuNo"),
        instructor: text(block, ".ttKyoin"),
        schedule: schedule(times),
        requirement: requirement(text(block, ".ttSuisho")),
        courseState,
        declaredOnly: false,
      });
    }
    for (const table of listTables) {
      const headers = [...table.querySelectorAll("tr:first-child th")].map(
        (cell) => clean(cell.textContent),
      );
      // An empty GridView renders only its empty-data row, without a header.
      if (!headers.length && table.rows.length === 1 && explicitEmpty(table))
        continue;
      if (
        !headers.some((header) => header.includes("授業担当教員")) ||
        !headers.some((header) => header.includes("備考"))
      )
        return incomplete("A registration list header is incomplete.");
      for (const row of [...table.querySelectorAll("tr")].slice(1)) {
        if (explicitEmpty(row)) continue;
        if (!row.querySelector("td") || !clean(row.textContent)) continue;
        const link = row.querySelector("a.kamokuGridKamokuName");
        if (!link) return incomplete("A registration list row is incomplete.");
        const offeringId =
          new URL(link.href, location.href).searchParams.get("jwc") || "";
        if (!/^\d{9}$/.test(offeringId))
          return incomplete("A registration list identifier is invalid.");
        if (seen.has(offeringId)) continue;
        seen.add(offeringId);
        const values = [...row.cells].map((cell) => clean(cell.textContent));
        const at = (label) =>
          values[headers.findIndex((header) => header.includes(label))] || "";
        const creditRaw = text(row, "span.resultUnitKamokuTaniSP");
        const credit = creditRaw.match(/\[\s*([\d.]+)-([\d.]+)-([\d.]+)\s*\]/);
        const rawSchedule = text(row, "td.tdYoubi");
        courses.push({
          offeringId,
          title: clean(link?.textContent),
          code: text(row, "span.resultUnitKamokuCodeSP"),
          instructor: at("授業担当教員"),
          schedule: schedule(lines(row.querySelector("td.tdYoubi"))),
          declaredOnly: rawSchedule === "申告専用",
          requirement: requirement(at("備考")),
          credits: credit
            ? {
                lecture: Number(credit[1]),
                exercise: Number(credit[2]),
                lab: Number(credit[3]),
              }
            : null,
        });
      }
    }
    const savedRaw = text(
      document,
      "#ctl00_ContentPlaceHolder1_ctl00_lblStatus3",
    );
    const savedMatch = savedRaw.match(
      /(\d{4})\/(\d{2})\/(\d{2})\s+(\d{2}):(\d{2}):(\d{2})/,
    );
    const savedAt = savedMatch
      ? `${savedMatch[1]}-${savedMatch[2]}-${savedMatch[3]}T${savedMatch[4]}:${savedMatch[5]}:${savedMatch[6]}+09:00`
      : null;
    if (
      courses.length === 0 &&
      !explicitEmpty(timetable) &&
      !listTables.some(explicitEmpty)
    )
      return incomplete("The empty registration list is not explicit.");
    return complete(
      "registration-page",
      {
        state: {
          state: stateMap[rawState] || "unknown",
          raw: rawState,
          savedAt,
        },
        courses,
        quarters:
          clean(document.querySelector("#rdoQAll")?.parentElement?.textContent)
            .match(/[1-4]/g)
            ?.map((quarter) => `${quarter}Q`) || [],
      },
      courses.length === 0,
    );
  }

  if (kind === "declared") {
    const tables = [
      ...document.querySelectorAll('table[id^="kamokuUnit_"][id$="_gvKamoku"]'),
    ];
    if (!tables.length)
      return incomplete("The declared-course page shape is unknown.");
    const courses = [];
    for (const table of tables) {
      const unit = table.id.replace(/_gvKamoku$/, "");
      const sumText = text(
        document,
        `#ctl00_ContentPlaceHolder1_${unit}_spanSum`,
      );
      const sum = Number((sumText.match(/([\d.]+)/) || [])[1]);
      for (const row of [...table.querySelectorAll("tr")].slice(1)) {
        if (explicitEmpty(row)) continue;
        if (!row.querySelector("td") || !clean(row.textContent)) continue;
        const link = row.querySelector("a.kamokuGridKamokuName");
        if (!link) return incomplete("A declared-course row is incomplete.");
        const offeringId =
          new URL(link.href, location.href).searchParams.get("jwc") || "";
        if (!/^\d{9}$/.test(offeringId))
          return incomplete("A declared-course identifier is invalid.");
        const range = text(row, "td.tdQ").match(/^([1-4])(?:\s*-\s*([1-4]))?$/);
        const credit = text(row, "span.resultUnitKamokuTaniSP").match(
          /\[\s*([\d.]+)-([\d.]+)-([\d.]+)\s*\]/,
        );
        courses.push({
          offeringId,
          unit,
          unitTotal: Number.isFinite(sum) ? sum : null,
          code: text(row, "span.resultUnitKamokuCodeSP"),
          title: clean(link.textContent),
          quarters: range
            ? Array.from(
                {
                  length: Number(range[2] || range[1]) - Number(range[1]) + 1,
                },
                (_, index) => Number(range[1]) + index,
              )
            : [],
          credits: credit
            ? {
                lecture: Number(credit[1]),
                exercise: Number(credit[2]),
                lab: Number(credit[3]),
              }
            : null,
        });
      }
    }
    if (!courses.length && !tables.some(explicitEmpty))
      return incomplete("The declared-course list is incomplete.");
    return complete(
      "declared-course-tables",
      { courses },
      courses.length === 0,
    );
  }

  if (kind === "grades") {
    const pageRoot = document.querySelector("#ctl00_ContentPlaceHolder1");
    const groups = [];
    const groupedTables = [
      ...document.querySelectorAll('h3[id$="_h3"], table[id$="_grid"]'),
    ];
    let category = "";
    let groupedCount = 0;
    let awaitingGroup = false;
    for (const node of groupedTables) {
      if (node.matches("h3")) {
        if (awaitingGroup)
          return incomplete("A grade group has no result table.");
        category = clean(node.textContent);
        awaitingGroup = true;
        continue;
      }
      const page = tableData(node);
      if (
        !category ||
        !awaitingGroup ||
        !page.headers.some((header) => header.includes("授業科目")) ||
        !page.headers.some((header) => header.includes("成績"))
      )
        return incomplete("A grouped grade table is incomplete.");
      groupedCount += 1;
      awaitingGroup = false;
      const rows = [];
      for (const row of [...node.querySelectorAll("tr")].slice(1)) {
        if (explicitEmpty(row)) continue;
        if (!row.querySelector("td") || !clean(row.textContent)) continue;
        const title = text(row, "td.resultUnitKamokuName");
        if (!title) return incomplete("A grouped grade row is incomplete.");
        rows.push({
          title,
          code:
            text(row, "td.resultUnitSuisho + td.mediaPC") ||
            text(row, "span.resultUnitKamokuCodeSP"),
          recommended: text(row, "td.resultUnitSuisho"),
          credits: text(row, "td.resultUnitTani"),
          effectiveCredits: row
            .querySelector("td.resultUnitTani")
            ?.getAttribute("data-sort")
            ?.trim()
            ? Number(
                row
                  .querySelector("td.resultUnitTani")
                  .getAttribute("data-sort"),
              )
            : undefined,
          result: text(row, "td.resultUnitSeiseki"),
          quarter: [...row.querySelectorAll('[class*="tdQuarter"]')]
            .map((cell) => clean(cell.textContent))
            .join(" "),
          completed: text(row, "td.resultUnitYG span.mediaPC"),
        });
      }
      if (!rows.length && !explicitEmpty(node))
        return incomplete("A grouped grade table has no explicit rows.");
      groups.push({ category, rows });
    }
    if (awaitingGroup) return incomplete("A grade group has no result table.");
    if (groupedCount) {
      return complete(
        "undergraduate-grouped",
        { groups },
        groups.every((group) => group.rows.length === 0),
      );
    }

    const flatGroups = [];
    for (const table of document.querySelectorAll("table")) {
      if (table.querySelector("table")) continue;
      const page = tableData(table);
      const titleAt = column(page.headers, "授業科目名");
      const instructorAt = column(page.headers, "授業担当教員");
      const creditsAt = column(page.headers, "単位");
      const resultAt = column(page.headers, "成績");
      const completedAt = column(page.headers, "修得時期");
      const columns = [titleAt, instructorAt, creditsAt, resultAt, completedAt];
      const recognizedColumns = columns.filter((at) => at >= 0).length;
      if (recognizedColumns === 0) continue;
      if (
        recognizedColumns !== columns.length ||
        new Set(columns).size !== columns.length
      )
        return incomplete("A flat grade table is incomplete.");
      const label = headingBefore(table);
      const section = label.includes("大学院科目")
        ? "graduate"
        : label.includes("学部科目")
          ? "undergraduate"
          : "";
      if (!section)
        return incomplete(
          "A flat grade table has no recognized section label.",
        );
      const rows = page.rows
        .filter(
          (row) =>
            row.some(Boolean) && !explicitEmpty({ textContent: row.join(" ") }),
        )
        .map((row) => ({
          title: row[titleAt] || "",
          instructor: row[instructorAt] || "",
          credits: row[creditsAt] || "",
          result: row[resultAt] || "",
          completed: row[completedAt] || "",
        }));
      if (rows.some((row) => !row.title))
        return incomplete("A flat grade row has no course title.");
      if (!rows.length && !explicitEmpty(table))
        return incomplete("A flat grade table has no explicit rows.");
      flatGroups.push({ section, category: label, rows });
    }
    const sections = new Set(flatGroups.map((group) => group.section));
    if (
      flatGroups.length === 2 &&
      sections.has("graduate") &&
      sections.has("undergraduate")
    )
      return complete(
        "graduate-flat",
        { groups: flatGroups },
        flatGroups.every((group) => group.rows.length === 0),
      );
    if (explicitEmpty(pageRoot))
      return complete("graduate-flat", { groups: [] }, true);
    return incomplete("The grade page shape is unknown.");
  }

  if (kind === "requirements") {
    const root = document.querySelector("div.tableSet02Tbl");
    if (!root) return incomplete("The requirement page shape is unknown.");
    const tables = [];
    for (const table of root.querySelectorAll("table")) {
      if (!table.matches("table.tblYouken"))
        return incomplete("An unknown requirement table is present.");
      // The live table has no <thead>; its header is as deep as the top-left
      // corner cell spans, while the first row already contains a <td>.
      const grid = matrix(table);
      const cornerDepth = grid.findIndex((row) => row[0] !== grid[0]?.[0]);
      const page = tableData(
        table,
        table.tHead?.rows.length || (cornerDepth > 0 ? cornerDepth : 0),
      );
      if (page.headers.length < 2)
        return incomplete("A requirement table has no semantic header.");
      const rows = page.rows
        .filter(
          (row) =>
            row.some(Boolean) && !explicitEmpty({ textContent: row.join(" ") }),
        )
        .map((row) => ({ label: row[0] || "", values: row.slice(1) }));
      if (!rows.some((row) => /(?:卒業|修了)要件|修得済単位/.test(row.label)))
        return incomplete("A requirement table has no recognized rows.");
      if (rows.some((row) => !row.label))
        return incomplete("A requirement row has no label.");
      tables.push({ headers: page.headers, rows });
    }
    if (tables.length)
      return complete("requirements-tables", { tables }, false);
    if (explicitEmpty(root))
      return complete("requirements-tables", { tables: [] }, true);
    return incomplete("The requirement page shape is unknown.");
  }

  const pageRoot = document.querySelector("#ctl00_ContentPlaceHolder1");
  const overallCandidates = [
    ...document.querySelectorAll("table.tblGPASummary.mediaPC"),
  ].map(matrix);
  const overallAt = overallCandidates.findIndex((table) => {
    const content = clean(table.flat().join(" "));
    return (
      content.includes("全ての科目") &&
      content.includes("GPA") &&
      content.includes("修得済単位")
    );
  });
  if (overallAt < 0) {
    if (explicitEmpty(pageRoot))
      return complete(
        "gpa-summary",
        { categories: [], overall: [], terms: [] },
        true,
      );
    return incomplete("The GPA page shape is unknown.");
  }
  const overall = [
    overallCandidates[overallAt],
    ...overallCandidates.filter((_, index) => index !== overallAt),
  ];
  const termTable = document.querySelector("table.tblQ");
  if (!termTable) return incomplete("The GPA term table is missing.");
  const termPage = tableData(termTable);
  if (
    !termPage.headers.some((header) => header.includes("GPA")) ||
    !termPage.headers.some((header) => /修得時期|年度|学期/.test(header))
  )
    return incomplete("The GPA term table is incomplete.");
  const terms = termPage.rows.filter((row) => row.some(Boolean));
  if (!terms.length && !explicitEmpty(termTable))
    return incomplete("The GPA term list has no explicit rows.");
  const categories = [...document.querySelectorAll('h3[id$="_h3"]')]
    .map((heading) => {
      const match = heading.id.match(/resultUnit_(\d+)_h3$/);
      return match
        ? { code: match[1], name: clean(heading.textContent) }
        : null;
    })
    .filter(Boolean);
  return complete("gpa-summary", { categories, overall, terms }, false);
})();
