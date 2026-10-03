use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde_json::Value;

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

fn compact_date(value: &str) -> Result<String, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map(|date| date.format("%Y%m%d").to_string())
        .map_err(|_| format!("invalid calendar date {value}"))
}

fn weekday_number(day: Weekday) -> u64 {
    day.number_from_monday() as u64
}

fn date_for_weekday(mut date: NaiveDate, day: u64) -> NaiveDate {
    while weekday_number(date.weekday()) != day {
        date += Duration::days(1);
    }
    date
}

fn clock_for_periods(first: u64, last: u64) -> (&'static str, &'static str) {
    let start = match first {
        1..=2 => "08:50",
        3..=4 => "10:45",
        5..=6 => "13:30",
        7..=8 => "15:25",
        _ => "17:15",
    };
    let end = match last {
        1..=2 => "10:30",
        3..=4 => "12:25",
        5..=6 => "15:10",
        7..=8 => "17:05",
        _ => "18:55",
    };
    (start, end)
}

fn quarter_range(workspace: &Value, term: &str) -> Result<(NaiveDate, NaiveDate), String> {
    let (year, quarter) = term
        .split_once('-')
        .ok_or_else(|| "term must be YEAR-QUARTER".to_string())?;
    let year_number: i32 = year
        .parse()
        .map_err(|_| "term year is invalid".to_string())?;
    let profile = workspace
        .pointer("/academicProfile/years")
        .and_then(Value::as_array)
        .and_then(|years| {
            years.iter().find(|item| {
                item.get("academicYear").and_then(Value::as_i64) == Some(year_number as i64)
            })
        });
    let fallback = match quarter {
        "1Q" => (format!("{year}-04-01"), format!("{year}-05-31")),
        "2Q" => (format!("{year}-06-01"), format!("{year}-09-30")),
        "3Q" => (format!("{year}-10-01"), format!("{year}-11-30")),
        "4Q" => (
            format!("{year}-12-01"),
            format!("{}-03-31", year_number + 1),
        ),
        _ => return Err("term quarter is invalid".into()),
    };
    let subscribed = workspace.get("academicCalendar").and_then(Value::as_array);
    let subscribed_boundary = |kind: &str| {
        subscribed
            .into_iter()
            .flatten()
            .filter(|entry| entry.get("type").and_then(Value::as_str) == Some(kind))
            .find(|entry| {
                entry.get("quarter").and_then(Value::as_str) == Some(quarter)
                    || entry
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id.contains(&format!(":{quarter}:")))
                    || subscribed.is_some_and(|rows| {
                        rows.iter()
                            .filter(|row| row.get("type").and_then(Value::as_str) == Some(kind))
                            .count()
                            == 1
                    })
            })
            .and_then(|entry| entry.get("date").and_then(Value::as_str))
    };
    let start = profile
        .and_then(|value| value.pointer(&format!("/quarterStarts/{quarter}")))
        .and_then(Value::as_str)
        .or_else(|| subscribed_boundary("quarter-start"))
        .unwrap_or(&fallback.0);
    let next = match quarter {
        "1Q" => "2Q",
        "2Q" => "3Q",
        "3Q" => "4Q",
        _ => "",
    };
    let end = if let Some(boundary) = subscribed_boundary("quarter-end") {
        boundary.to_string()
    } else if next.is_empty() {
        fallback.1
    } else if let Some(next_start) = profile
        .and_then(|value| value.pointer(&format!("/quarterStarts/{next}")))
        .and_then(Value::as_str)
    {
        (NaiveDate::parse_from_str(next_start, "%Y-%m-%d")
            .map_err(|_| "quarter start is invalid".to_string())?
            - Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
    } else {
        fallback.1
    };
    Ok((
        NaiveDate::parse_from_str(start, "%Y-%m-%d")
            .map_err(|_| "quarter start is invalid".to_string())?,
        NaiveDate::parse_from_str(&end, "%Y-%m-%d")
            .map_err(|_| "quarter end is invalid".to_string())?,
    ))
}

fn push_event(
    output: &mut String,
    uid: &str,
    title: &str,
    room: &str,
    date: &str,
    start: &str,
    end: &str,
    extra: &[String],
) -> Result<(), String> {
    let date = compact_date(date)?;
    output.push_str("BEGIN:VEVENT\r\n");
    output.push_str(&format!("UID:{}@campus-life.local\r\n", escape(uid)));
    output.push_str(&format!("SUMMARY:{}\r\n", escape(title)));
    if !room.is_empty() {
        output.push_str(&format!("LOCATION:{}\r\n", escape(room)));
    }
    output.push_str(&format!(
        "DTSTART;TZID=Asia/Tokyo:{}T{}00\r\n",
        date,
        start.replace(':', "")
    ));
    output.push_str(&format!(
        "DTEND;TZID=Asia/Tokyo:{}T{}00\r\n",
        date,
        end.replace(':', "")
    ));
    for line in extra {
        output.push_str(line);
        output.push_str("\r\n");
    }
    output.push_str("END:VEVENT\r\n");
    Ok(())
}

pub fn export_quarter(workspace: &Value, term: &str) -> Result<String, String> {
    let (quarter_start, quarter_end) = quarter_range(workspace, term)?;
    let mut output = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Campus Life//Science Tokyo//EN\r\nCALSCALE:GREGORIAN\r\nBEGIN:VTIMEZONE\r\nTZID:Asia/Tokyo\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:+0900\r\nTZOFFSETTO:+0900\r\nTZNAME:JST\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n",
    );
    let courses = workspace
        .get("courses")
        .and_then(Value::as_array)
        .ok_or_else(|| "workspace courses are unavailable".to_string())?;
    let active: std::collections::HashSet<_> = workspace
        .get("memberships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            workspace
                .get("planItems")
                .and_then(Value::as_array)
                .into_iter()
                .flatten(),
        )
        .filter(|row| row.get("deletedAt").is_none())
        .filter_map(|row| row.get("courseId").and_then(Value::as_str))
        .collect();
    let calendar = workspace
        .get("academicCalendar")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for course in courses.iter().filter(|course| {
        course.get("termId").and_then(Value::as_str) == Some(term)
            && course.get("deletedAt").is_none()
            && course
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| active.contains(id))
    }) {
        let id = course["id"].as_str().unwrap_or_default();
        let title = course["title"].as_str().unwrap_or("Course");
        for (index, meeting) in course
            .get("meetings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let start_time = meeting["startTime"].as_str().unwrap_or("00:00");
            let end_time = meeting["endTime"].as_str().unwrap_or(start_time);
            let room = meeting["room"].as_str().unwrap_or_default();
            for day in meeting
                .get("days")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
            {
                let first = date_for_weekday(quarter_start, day);
                let byday = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
                    [(day.saturating_sub(1) as usize).min(6)];
                let mut extra = vec![format!(
                    "RRULE:FREQ=WEEKLY;BYDAY={byday};UNTIL={}T145959Z",
                    quarter_end.format("%Y%m%d")
                )];
                let exclusions = calendar
                    .iter()
                    .filter(|entry| {
                        matches!(
                            entry.get("type").and_then(Value::as_str),
                            Some("holiday" | "no-class")
                        )
                    })
                    .filter_map(|entry| entry.get("date").and_then(Value::as_str))
                    .filter_map(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
                    .filter(|date| {
                        *date >= quarter_start
                            && *date <= quarter_end
                            && weekday_number(date.weekday()) == day
                    })
                    .map(|date| {
                        format!(
                            "EXDATE;TZID=Asia/Tokyo:{}T{}00",
                            date.format("%Y%m%d"),
                            start_time.replace(':', "")
                        )
                    })
                    .collect::<Vec<_>>();
                extra.extend(exclusions);
                push_event(
                    &mut output,
                    &format!("{id}-weekly-{index}-{day}"),
                    title,
                    room,
                    &first.format("%Y-%m-%d").to_string(),
                    start_time,
                    end_time,
                    &extra,
                )?;
            }
        }

        // Replacement teaching days are explicit one-offs. The ordinary
        // weekday remains excluded above when it is a holiday/no-class day.
        for replacement in calendar
            .iter()
            .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("replacement"))
        {
            let Some(date) = replacement.get("date").and_then(Value::as_str) else {
                continue;
            };
            let Some(replacement_day) = replacement.get("replacementDay").and_then(Value::as_u64)
            else {
                continue;
            };
            let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|_| "replacement date is invalid".to_string())?;
            if parsed < quarter_start || parsed > quarter_end {
                continue;
            }
            for meeting in course
                .get("meetings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if !meeting
                    .get("days")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|day| day.as_u64() == Some(replacement_day))
                {
                    continue;
                }
                push_event(
                    &mut output,
                    &format!("{id}-replacement-{date}"),
                    title,
                    meeting["room"].as_str().unwrap_or_default(),
                    date,
                    meeting["startTime"].as_str().unwrap_or("00:00"),
                    meeting["endTime"].as_str().unwrap_or("00:00"),
                    &[],
                )?;
            }
        }
    }

    for intensive in workspace
        .get("intensiveSchedules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("deletedAt").is_none())
    {
        let Some(course_id) = intensive.get("courseId").and_then(Value::as_str) else {
            continue;
        };
        let Some(course) = courses.iter().find(|course| {
            course.get("id").and_then(Value::as_str) == Some(course_id)
                && course.get("termId").and_then(Value::as_str) == Some(term)
        }) else {
            continue;
        };
        if !active.contains(course_id) {
            continue;
        }
        for day in intensive
            .get("days")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(date) = day.get("date").and_then(Value::as_str) else {
                continue;
            };
            let parsed = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|_| "intensive date is invalid".to_string())?;
            if parsed < quarter_start || parsed > quarter_end {
                continue;
            }
            let Some(periods) = day.get("periods").and_then(Value::as_array) else {
                continue;
            };
            let Some(first) = periods.first().and_then(Value::as_u64) else {
                continue;
            };
            let Some(last) = periods.get(1).and_then(Value::as_u64) else {
                continue;
            };
            let (start, end) = clock_for_periods(first, last);
            push_event(
                &mut output,
                &format!("{course_id}-intensive-{date}"),
                course
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Course"),
                intensive
                    .get("room")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                date,
                start,
                end,
                &[],
            )?;
        }
    }

    for plan in workspace
        .get("plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|plan| plan.get("deletedAt").is_none())
    {
        let start = plan["start"].as_str().unwrap_or_default();
        let date = start.get(..10).unwrap_or(start);
        if date < quarter_start.format("%Y-%m-%d").to_string().as_str()
            || date > quarter_end.format("%Y-%m-%d").to_string().as_str()
        {
            continue;
        }
        let time = start.get(11..16).unwrap_or("00:00");
        let minutes = plan["duration"].as_i64().unwrap_or(60);
        let parsed_time = chrono::NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|_| "plan time is invalid".to_string())?;
        let end = (parsed_time + Duration::minutes(minutes))
            .format("%H:%M")
            .to_string();
        let mut extra = Vec::new();
        if let Some(rule) = plan.get("rrule").and_then(Value::as_str) {
            extra.push(format!("RRULE:{}", crate::plans::validate_rrule(rule)?));
        }
        for exdate in plan
            .get("exdates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            extra.push(format!(
                "EXDATE;TZID=Asia/Tokyo:{}T{}00",
                compact_date(exdate)?,
                time.replace(':', "")
            ));
        }
        push_event(
            &mut output,
            plan["id"].as_str().unwrap_or("plan"),
            plan["title"].as_str().unwrap_or("Plan"),
            plan["room"].as_str().unwrap_or_default(),
            date,
            time,
            &end,
            &extra,
        )?;
    }
    output.push_str("END:VCALENDAR\r\n");
    Ok(output)
}
