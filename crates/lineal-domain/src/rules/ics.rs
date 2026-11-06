//! iCalendar-Ausgabe (RFC 5545) für den Kalender-Feed: ganztägige Termine, reine Textfunktion.

use chrono::{DateTime, Duration, NaiveDate, Utc};

/// Ein ganztägiger Termin.
#[derive(Debug, Clone, PartialEq)]
pub struct IcsEvent {
    pub uid: String,
    pub date: NaiveDate,
    pub summary: String,
    pub url: String,
    pub stamp: DateTime<Utc>,
    /// Steigt mit jeder Änderung, damit Kalender den Termin ersetzen.
    pub sequence: i32,
}

/// Maskiert Text für einen Eigenschaftswert (RFC 5545 §3.3.11).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Bricht eine Zeile nach höchstens 75 Oktetten um, ohne ein Zeichen zu teilen (RFC 5545 §3.1).
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += c.len_utf8();
    }
    out.push_str("\r\n");
}

/// Der vollständige Kalender.
pub fn calendar(name: &str, events: &[IcsEvent]) -> String {
    let mut out = String::new();
    let mut line = |l: String| fold(&l, &mut out);
    line("BEGIN:VCALENDAR".into());
    line("VERSION:2.0".into());
    line("PRODID:-//Lineal//Milestones//EN".into());
    line("CALSCALE:GREGORIAN".into());
    line("METHOD:PUBLISH".into());
    line(format!("X-WR-CALNAME:{}", escape(name)));
    line("X-PUBLISHED-TTL:PT1H".into());
    for e in events {
        line("BEGIN:VEVENT".into());
        line(format!("UID:{}", escape(&e.uid)));
        line(format!("DTSTAMP:{}", e.stamp.format("%Y%m%dT%H%M%SZ")));
        line(format!("DTSTART;VALUE=DATE:{}", e.date.format("%Y%m%d")));
        // DTEND ist exklusiv: ein ganzer Tag endet am Folgetag.
        line(format!(
            "DTEND;VALUE=DATE:{}",
            (e.date + Duration::days(1)).format("%Y%m%d")
        ));
        line(format!("SEQUENCE:{}", e.sequence));
        line(format!("SUMMARY:{}", escape(&e.summary)));
        line(format!("URL:{}", e.url));
        line("TRANSP:TRANSPARENT".into());
        line("END:VEVENT".into());
    }
    line("END:VCALENDAR".into());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn event(summary: &str) -> IcsEvent {
        IcsEvent {
            uid: "m1@lineal".into(),
            date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            summary: summary.into(),
            url: "https://x.test/a".into(),
            stamp: Utc.with_ymd_and_hms(2026, 11, 6, 8, 0, 0).unwrap(),
            sequence: 3,
        }
    }

    #[test]
    fn all_day_event_ends_on_the_next_day_across_the_year() {
        let ics = calendar("Milestones", &[event("Go-Live")]);
        assert!(ics.contains("DTSTART;VALUE=DATE:20261231\r\n"));
        assert!(ics.contains("DTEND;VALUE=DATE:20270101\r\n"));
        assert!(ics.contains("DTSTAMP:20261106T080000Z\r\n"));
        assert!(ics.contains("SEQUENCE:3\r\n"));
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
    }

    #[test]
    fn text_is_escaped_so_it_cannot_add_properties() {
        let ics = calendar("A;B", &[event("a,b;c\\d\r\nEND:VEVENT")]);
        assert!(ics.contains("SUMMARY:a\\,b\\;c\\\\d\\nEND:VEVENT\r\n"));
        assert!(ics.contains("X-WR-CALNAME:A\\;B\r\n"));
        assert_eq!(ics.split("\r\n").filter(|l| *l == "END:VEVENT").count(), 1);
    }

    #[test]
    fn long_lines_fold_on_character_boundaries() {
        let ics = calendar("M", &[event(&"ä".repeat(100))]);
        for l in ics.split("\r\n") {
            assert!(l.len() <= 75, "{l}");
        }
        let unfolded = ics.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:{}\r\n", "ä".repeat(100))));
    }
}
