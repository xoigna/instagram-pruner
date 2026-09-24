use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateBound {
    Before,

    After,
}

pub fn parse_to_datetime(input: &str, bound: DateBound) -> Option<DateTime<Utc>> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(input, fmt) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        let naive = match bound {
            DateBound::After => date.and_hms_opt(0, 0, 0)?,

            DateBound::Before => date.and_hms_nano_opt(23, 59, 59, 999_999_999)?,
        };
        return Some(Utc.from_utc_datetime(&naive));
    }
    None
}

pub fn validate_date_input(s: &str) -> Result<Option<DateTime<Utc>>, String> {
    if s.trim().is_empty() {
        return Ok(None);
    }

    parse_to_datetime(s, DateBound::After)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "invalid date {:?}; expected RFC3339 (e.g. 2024-12-31T23:59:00Z) or YYYY-MM-DD",
                s
            )
        })
}

pub fn media_taken_at(secs: i64) -> Option<DateTime<Utc>> {
    if secs <= 0 {
        return None;
    }
    DateTime::<Utc>::from_timestamp(secs, 0)
}

pub fn dm_timestamp_us(us: i64) -> Option<DateTime<Utc>> {
    if us <= 0 {
        return None;
    }
    let secs = us / 1_000_000;
    let nsecs = ((us % 1_000_000) * 1000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nsecs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rfc3339() {
        let dt = parse_to_datetime("2024-06-15T12:00:00Z", DateBound::Before).unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2024-06-15");
    }

    #[test]
    fn bare_date_after_is_midnight() {
        let d = parse_to_datetime("2024-01-02", DateBound::After).unwrap();
        assert_eq!(d.timestamp(), 1_704_153_600);
    }

    #[test]
    fn bare_date_before_is_end_of_day() {
        let d = parse_to_datetime("2024-01-02", DateBound::Before).unwrap();

        assert_eq!(
            d.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2024-01-02 23:59:59"
        );
        let start = parse_to_datetime("2024-01-02", DateBound::After).unwrap();
        assert!(d > start);

        let next = parse_to_datetime("2024-01-03", DateBound::After).unwrap();
        assert!(d < next);
    }

    #[test]
    fn validate_empty_is_none() {
        assert_eq!(validate_date_input("").unwrap(), None);
        assert_eq!(validate_date_input("   ").unwrap(), None);
    }

    #[test]
    fn validate_rejects_garbage() {
        assert!(validate_date_input("not-a-date").is_err());
    }
}
