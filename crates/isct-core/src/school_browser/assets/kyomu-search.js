// Owned hidden search capture. POSTs only search and pagination controls on
// this exact read-only search route; never registration controls.
(async () => {
  const path =
    "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0_1.aspx";
  const clean = (value) =>
    String(value || "")
      .normalize("NFKC")
      .replace(/\s+/g, " ")
      .trim();
  const result = (status, data = {}, message = "") => ({
    status,
    kind: "catalog",
    sourceUrl: location.href,
    data,
    message,
  });
  const maintenance = globalThis.__isctKyomuMaintenance?.(document);
  if (location.origin === "https://kyomu0.gakumu.titech.ac.jp" && maintenance)
    return result("maintenance", {}, maintenance);
  if (
    location.origin !== "https://kyomu0.gakumu.titech.ac.jp" ||
    location.pathname.toLowerCase() !== path.toLowerCase()
  )
    return result("error", {}, "The course search page is not approved.");
  const request =
    globalThis.__ISCT_COMPANION_KYOMU_HIDDEN__ === true
      ? globalThis.__ISCT_COMPANION_KYOMU_PAGE__
      : null;
  const filtersKey = JSON.stringify([
    request?.controls || [],
    request?.groups || [],
    request?.requirementGroups || [],
    Boolean(request?.recommended),
  ]);
  const prefix = "isct-course-page:";
  const now = Date.now();
  const pageSize = 100;
  let page = document;
  let offset = 0;
  let loaded = 0;
  let cachedDepartment = "";
  let previousKey = "";
  const post = async (body) => {
    const response = await fetch(location.href, {
      method: "POST",
      credentials: "same-origin",
      body,
      signal: AbortSignal.timeout(45000),
    });
    const html = await response.text();
    if (html.length > 8 * 1024 * 1024)
      throw new Error("Course search is too large.");
    const parsed = new DOMParser().parseFromString(html, "text/html");
    const notice =
      new URL(response.url).origin === location.origin &&
      globalThis.__isctKyomuMaintenance?.(parsed);
    if (notice) throw { maintenance: notice };
    if (
      !response.ok ||
      new URL(response.url).origin !== location.origin ||
      new URL(response.url).pathname.toLowerCase() !== path.toLowerCase()
    )
      throw new Error("Course search session expired.");
    return parsed;
  };
  // ASP.NET checkboxes have value="on", not the query-string enum value.
  // Keep actual control IDs/values and let the university serialize its search.
  const publicOptions = (root) => {
    const options = [];
    const search = root.querySelector('[id$="btnSearch"]');
    // Group identities describe semantics only. Labels, available choices and
    // submitted values always come from this account's current official form.
    const families = [
      ["KamokuCodeSeries", "level"],
      ["HisshuKbn", "requirement"],
      ["Youbi", "delivery"],
      ["KaisiJigen", "start"],
      ["Quarter", "quarter"],
      ["Lang", "language"],
      ["Gengo", "language"],
      ["KamokuKbnMidashi", "category"],
      ["Suisho", "recommended"],
      ["Zengaku", "category"],
      ["Senmon", "category"],
    ];
    for (const input of root.querySelectorAll(
      'input[type="checkbox"],input[type="radio"],select',
    )) {
      if (
        !input.id ||
        input.matches(":disabled") ||
        input.closest('[id*="gvGrid"]')
      )
        continue;
      if (
        search &&
        !(
          input.compareDocumentPosition(search) &
          Node.DOCUMENT_POSITION_FOLLOWING
        )
      )
        continue;
      const group = input.closest("fieldset,[role=group],table[id],div[id]");
      const identity = `${input.id} ${input.name} ${group?.id || ""}`;
      const family =
        families.find(([key]) =>
          identity.toLowerCase().includes(key.toLowerCase()),
        )?.[1] || "other";
      const label = clean(
        root.querySelector(`label[for="${input.id}"]`)?.textContent ||
          input.closest("label")?.textContent,
      );
      if (input.tagName === "SELECT") {
        for (const option of input.options) {
          if (
            option.value &&
            !option.disabled &&
            !option.parentElement?.disabled
          )
            options.push({
              id: input.id,
              value: option.value,
              label: clean(option.textContent),
              family,
            });
        }
      } else if (label)
        options.push({ id: input.id, value: input.value, label, family });
    }
    return options;
  };
  let searchOptions = publicOptions(page);
  const selections = [...(request?.controls || [])];
  const addSelection = (family, label) => {
    const option = searchOptions.find(
      (o) => o.family === family && o.label === label,
    );
    if (!option) return false;
    selections.push(option);
    return true;
  };
  if (request && !request.cursor) {
    for (const [family, labels] of [
      ["level", request.groups],
      ["requirement", request.requirementGroups],
    ]) {
      for (const label of labels || []) {
        if (!addSelection(family, label))
          return result(
            "error",
            {},
            "This filter is not offered by the current official search form.",
          );
      }
    }
    // Resolve typed shortcuts against live labels, never ASP.NET enum IDs.
    const selectMatching = (family, predicate, requested) => {
      if (!requested?.length) return true;
      const matches = searchOptions.filter(
        (option) =>
          option.family === family &&
          requested.some((value) => predicate(option.label, value)),
      );
      if (
        requested.some(
          (value) => !matches.some((option) => predicate(option.label, value)),
        )
      )
        return false;
      selections.push(...matches);
      return true;
    };
    const quartersIn = (label) => {
      const digits = [...label.matchAll(/[1-4]/g)].map((match) =>
        Number(match[0]),
      );
      if (digits.length === 2 && /[~〜～-]/.test(label))
        return Array.from(
          { length: digits[1] - digits[0] + 1 },
          (_, i) => digits[0] + i,
        );
      return digits;
    };
    const days = ["日", "月", "火", "水", "木", "金", "土"];
    if (
      !selectMatching(
        "quarter",
        (label, quarter) => quartersIn(label).includes(quarter),
        request.quarters,
      ) ||
      !selectMatching(
        "delivery",
        (label, day) => label === days[day % 7],
        request.days,
      ) ||
      !selectMatching(
        "start",
        (label, period) => period === Number(label.match(/^\d+/)?.[0]),
        request.periodStarts,
      ) ||
      !selectMatching(
        "language",
        (label, language) =>
          language === "ja"
            ? /日本語|Japanese/i.test(label)
            : /英語|English/i.test(label),
        request.language ? [request.language] : [],
      )
    )
      return result(
        "error",
        {},
        "This filter is not offered by the current official search form.",
      );
    if (request.recommended) {
      const option = searchOptions.find((o) => o.family === "recommended");
      if (!option)
        return result(
          "error",
          {},
          "The current official form does not offer recommendations.",
        );
      selections.push(option);
    }
  }
  if (request && !request.cursor && selections.length) {
    const form = page.querySelector("form");
    const search = page.querySelector('[id$="btnSearch"]');
    if (!form || !search)
      return result("error", {}, "The official search controls changed.");
    const selected = [];
    for (const selection of selections.filter(
      (option, index, all) =>
        all.findIndex(
          (other) => other.id === option.id && other.value === option.value,
        ) === index,
    )) {
      const option = searchOptions.find(
        (o) => o.id === selection.id && o.value === selection.value,
      );
      if (!option)
        return result(
          "error",
          {},
          "This filter is not offered by the current official search form.",
        );
      const input = page.getElementById(option.id);
      selected.push({ option, input });
    }
    // Let the live school's recommendation handler set its own default
    // requirement selection; undergraduate and graduate defaults may differ.
    if (!selected.some(({ option }) => option.family === "requirement")) {
      for (const { option } of selected.filter(
        ({ option }) => option.family === "recommended",
      )) {
        const control = page.getElementById(option.id);
        if (control && !control.checked) control.click();
      }
    }
    const body = new URLSearchParams(new FormData(form));
    const families = new Set(selected.map((s) => s.option.family));
    // Replace an entire checkbox group, retaining unrelated URL filters.
    for (const option of searchOptions.filter((o) => families.has(o.family))) {
      const input = page.getElementById(option.id);
      if (input?.name) body.delete(input.name);
    }
    for (const { option, input } of selected) {
      if (input.type === "radio") body.set(input.name, option.value);
      else body.append(input.name, option.value);
      if (input.tagName === "SELECT" && input.id.includes("KamokuKbnMidashi")) {
        const radio = page.querySelector(
          input.id.endsWith("Zengaku")
            ? '[id$="rdoZengaku"]'
            : '[id$="rdoSenmon"]',
        );
        if (radio) body.set(radio.name, radio.value);
      }
    }
    body.set("__EVENTTARGET", search.id.replaceAll("_", "$"));
    body.set("__EVENTARGUMENT", "");
    page = await post(body);
    searchOptions = publicOptions(page);
  }
  if (request?.cursor) {
    const cached = JSON.parse(
      sessionStorage.getItem(prefix + request.cursor) || "null",
    );
    if (
      !cached ||
      cached.url !== location.href ||
      cached.filtersKey !== filtersKey ||
      cached.expires < now
    )
      return result(
        "error",
        {},
        "The course search page expired. Refresh the search.",
      );
    searchOptions = cached.searchOptions || searchOptions;
    loaded = cached.loaded;
    previousKey = cached.previousKey || "";
    cachedDepartment = cached.department || "";
    if (cached.html) {
      page = new DOMParser().parseFromString(cached.html, "text/html");
      offset = cached.offset;
    } else {
      if (
        !/^ctl00\$ContentPlaceHolder1\$gvGrid\$ctl\d+\$ctl\d+$/.test(
          cached.target,
        )
      )
        return result("error", {}, "The course page control is invalid.");
      const body = new URLSearchParams(cached.fields);
      body.set("__EVENTTARGET", cached.target);
      body.set("__EVENTARGUMENT", "");
      page = await post(body);
    }
  } else if (request) {
    const size = page.querySelector(
      '#ctl00_ContentPlaceHolder1_gvGrid select[id$="_drpPageSize"]',
    );
    if (size && Number(size.value) !== pageSize) {
      if (
        !size.form ||
        ![...size.options].some((option) => option.value === String(pageSize))
      )
        return result("error", {}, "The search paging controls changed.");
      const body = new URLSearchParams(new FormData(size.form));
      body.set(size.name, String(pageSize));
      body.set("__EVENTTARGET", size.name);
      body.set("__EVENTARGUMENT", "");
      page = await post(body);
    }
  }
  const grid = page.querySelector("#ctl00_ContentPlaceHolder1_gvGrid");
  if (!grid) {
    const empty = page.querySelector("#ctl00_ContentPlaceHolder1_cvKamoku");
    if (
      empty &&
      /指定した条件の科目はありません/.test(clean(empty.textContent)) &&
      !/display:\s*none|visibility:\s*hidden/i.test(
        empty.getAttribute("style") || "",
      )
    )
      return result("ok", {
        complete: true,
        shape: "course-search",
        empty: true,
        courses: [],
        searchOptions,
        truncated: false,
      });
    return result("error", {}, "The course search result is incomplete.");
  }
  const headers = [...grid.querySelectorAll("th")].map((cell) =>
    clean(cell.textContent),
  );
  const at = (name) => headers.findIndex((header) => header.startsWith(name));
  if (
    [
      "Q",
      "曜日・時限",
      "科目コード",
      "授業科目名",
      "単位",
      "授業担当教員",
      "備考",
    ].some((name) => at(name) < 0)
  )
    return result("error", {}, "The course search columns changed.");
  const department =
    cachedDepartment ||
    clean(page.body.textContent).match(/推奨\s*\[([^\]]+)\]/)?.[1] ||
    "";
  const quartersOf = (raw) => {
    const normalized = clean(raw).replace(/Q/g, "");
    const range = normalized.match(/^([1-4])[~〜～-]([1-4])$/);
    return range
      ? Array.from(
          { length: Number(range[2]) - Number(range[1]) + 1 },
          (_, i) => `${Number(range[1]) + i}Q`,
        )
      : [...new Set(normalized.match(/[1-4]/g) || [])].map((q) => `${q}Q`);
  };
  const courses = [];
  const seen = new Set();
  const courseRows = [...grid.rows].filter((row) =>
    row.querySelector('a[href*="jwc="]'),
  );
  for (const row of courseRows.slice(offset, offset + pageSize)) {
    const link = row.querySelector('a[href*="jwc="]');
    if (!link) continue;
    const id = new URL(
      link.getAttribute("href"),
      location.href,
    ).searchParams.get("jwc");
    const cell = (name) => row.cells[at(name)];
    const value = (name) => clean(cell(name)?.textContent);
    const credits = value("単位").match(/^([\d.]+)-([\d.]+)-([\d.]+)$/);
    const quarters = quartersOf(value("Q"));
    if (
      !/^\d{9}$/.test(id || "") ||
      seen.has(id) ||
      !credits ||
      !quarters.length
    )
      return result("error", {}, "A course search row is incomplete.");
    seen.add(id);
    const copy = cell("曜日・時限").cloneNode(true);
    copy.querySelectorAll("br").forEach((br) => br.replaceWith("\n"));
    const rawSlots = copy.textContent.split("\n").map(clean).filter(Boolean);
    const schedule = rawSlots.map((raw) => {
      const match = raw.match(
        /^([月火水木金土日])\s*(\d+)-(\d+)(?:\s*\((.*)\))?$/,
      );
      if (match)
        return {
          kind: "weekly",
          day: "月火水木金土日".indexOf(match[1]) + 1,
          periods: [Number(match[2]), Number(match[3])],
          room: { ja: match[4] || "" },
        };
      return {
        kind: raw.includes("申告専用")
          ? "registration-only"
          : raw.includes("オンデマンド")
            ? "on-demand"
            : raw.includes("集中")
              ? "intensive"
              : "tba",
        room: { ja: "" },
      };
    });
    if (
      schedule.some(
        (slot) =>
          slot.kind === "weekly" &&
          (slot.periods[0] < 1 ||
            slot.periods[1] > 12 ||
            slot.periods[0] > slot.periods[1]),
      )
    )
      return result("error", {}, "A course search period is invalid.");
    const mark = value("備考").match(/^([A-Z])\s*\((◎|○|選択)\)/);
    courses.push({
      id,
      year: Number(id.slice(0, 4)),
      quarters,
      code: value("科目コード"),
      title: { ja: clean(link.textContent) },
      instructors: value("授業担当教員")
        .split(/,\s*/)
        .filter(Boolean)
        .map((ja) => ({ ja })),
      credits: {
        lecture: Number(credits[1]),
        exercise: Number(credits[2]),
        lab: Number(credits[3]),
      },
      instructionLanguage: value("開講言語").includes("英語")
        ? "en"
        : value("開講言語").includes("日本語")
          ? "ja"
          : "other",
      format: {
        kinds: [
          Number(credits[1]) > 0 ? "lecture" : null,
          Number(credits[2]) > 0 ? "exercise" : null,
          Number(credits[3]) > 0 ? "experiment" : null,
        ].filter(Boolean),
      },
      media: {},
      academicUnit: {},
      intensive: schedule.some((s) => s.kind === "intensive"),
      schedule,
      syllabusUrl: `https://syllabus.s.isct.ac.jp/courses?jwc=${id}`,
      fetchedAt: {},
      kyomu: {
        department,
        group: mark?.[1] || "",
        mark:
          mark?.[2] === "◎"
            ? "required"
            : mark?.[2] === "○"
              ? "recommended"
              : mark
                ? "elective"
                : "",
        category: value("科目区分"),
        prerequisite: value("備考").includes("履修前提条件"),
        fetchedAt: new Date().toISOString(),
      },
    });
  }
  if (
    !courses.length &&
    !/指定した条件の科目はありません|対象データはありません/.test(
      clean(grid.textContent),
    )
  )
    return result("error", {}, "The empty course result is not explicit.");
  const pageKey = courses.map((course) => course.id).join(",");
  if (courses.length && pageKey === previousKey)
    return result(
      "error",
      {},
      "The official course page did not advance. Refresh the search.",
    );
  let nextCursor;
  const remaining = offset + courses.length < courseRows.length;
  const nextLink = [...grid.querySelectorAll("a")].find((link) =>
    /^(>>|次へ|次のページ)$/.test(clean(link.textContent)),
  );
  if (request && (remaining || nextLink)) {
    const cached = {
      url: location.href,
      filtersKey,
      expires: now + 5 * 60 * 1000,
      loaded: loaded + courses.length,
      department,
      searchOptions,
      previousKey: pageKey,
    };
    if (remaining) {
      // A session may have a larger page size saved by the university UI.
      // Keep only its public grid in this transport cache and emit 100 per call.
      cached.html = grid.outerHTML;
      cached.offset = offset + courses.length;
    } else {
      const href = nextLink.getAttribute("href") || "";
      const target = href.match(
        /(?:__doPostBack\(|WebForm_PostBackOptions\()["']([^"']+)["']/,
      )?.[1];
      const form = page.querySelector("form");
      if (
        !form ||
        !target ||
        !/^ctl00\$ContentPlaceHolder1\$gvGrid\$ctl\d+\$ctl\d+$/.test(target)
      )
        return result("error", {}, "The next course page control changed.");
      cached.fields = [...new FormData(form)];
      cached.target = target;
    }
    // Opaque form state never crosses the native/FFI boundary. It expires in
    // its authenticated origin and is bounded to 16 transport cursors.
    const keys = Object.keys(sessionStorage).filter((key) =>
      key.startsWith(prefix),
    );
    for (const key of keys) {
      try {
        if (JSON.parse(sessionStorage.getItem(key)).expires < now)
          sessionStorage.removeItem(key);
      } catch {
        sessionStorage.removeItem(key);
      }
    }
    for (const key of keys.slice(0, Math.max(0, keys.length - 15)))
      sessionStorage.removeItem(key);
    sessionStorage.setItem(prefix + request.nextCursor, JSON.stringify(cached));
    // A larger saved grid must not be copied once per page until origin
    // storage fills up. Retire the consumed cursor after its successor is safe.
    if (request.cursor) sessionStorage.removeItem(prefix + request.cursor);
    nextCursor = request.nextCursor;
  }
  return result("ok", {
    complete: true,
    shape: "course-search",
    empty: !courses.length,
    courses,
    searchOptions,
    nextCursor,
    truncated:
      !nextCursor &&
      (remaining || Boolean(nextLink) || loaded + courses.length >= 1000),
  });
})().catch((error) => ({
  status: error?.maintenance ? "maintenance" : "error",
  kind: "catalog",
  sourceUrl: location.href,
  data: {},
  message:
    error?.maintenance || "The official course search could not be read.",
}));
