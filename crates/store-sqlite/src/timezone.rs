use crate::{Result, StoreError};

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
const RANGE: StoreError = StoreError::Invalid("unrepresentable calendar boundary");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeZoneSpec {
    FixedOffset {
        offset_minutes: i32,
    },
    DstRule {
        std_offset_minutes: i32,
        dst_offset_minutes: i32,
        spring_transition_utc_ms: i64,
        fall_transition_utc_ms: i64,
    },
}

// March-based 400-year Gregorian eras (Howard Hinnant's civil calendar).
fn days(d: CivilDate) -> Result<i64> {
    if !(1..=12).contains(&d.month) || d.day == 0 || d.day > 31 {
        return Err(RANGE);
    }
    let year = i64::from(d.year) - i64::from(d.month <= 2);
    let era = year.div_euclid(400);
    let y = year.rem_euclid(400);
    let m = i64::from(d.month) + if d.month > 2 { -3 } else { 9 };
    let n = era * 146097 + y * 365 + y / 4 - y / 100 + (153 * m + 2) / 5 + i64::from(d.day)
        - 1
        - 719468;
    if civil(n) != d {
        return Err(RANGE);
    }
    Ok(n)
}
fn civil(n: i64) -> CivilDate {
    let z = n + 719468;
    let era = z.div_euclid(146097);
    let d = z.rem_euclid(146097);
    let y = (d - d / 1460 + d / 36524 - d / 146096) / 365;
    let doy = d - (365 * y + y / 4 - y / 100);
    let mp = (5 * doy + 2) / 153;
    let month = mp + if mp < 10 { 3 } else { -9 };
    CivilDate {
        year: (era * 400 + y + i64::from(month <= 2)) as i32,
        month: month as u8,
        day: (doy - (153 * mp + 2) / 5 + 1) as u8,
    }
}
pub fn civil_from_unix_ms(ms: i64) -> CivilDate {
    civil(ms.div_euclid(DAY_MS))
}
pub fn unix_ms_from_civil(d: CivilDate) -> Result<i64> {
    days(d)?.checked_mul(DAY_MS).ok_or(RANGE)
}

impl TimeZoneSpec {
    fn offset(self, t: i64) -> i32 {
        match self {
            Self::FixedOffset { offset_minutes } => offset_minutes,
            Self::DstRule {
                std_offset_minutes,
                dst_offset_minutes,
                spring_transition_utc_ms: s,
                fall_transition_utc_ms: f,
            } => {
                if (s < f && t >= s && t < f) || (s > f && (t >= s || t < f)) {
                    dst_offset_minutes
                } else {
                    std_offset_minutes
                }
            }
        }
    }
    fn local(self, t: i64) -> i128 {
        i128::from(t) + i128::from(self.offset(t)) * 60_000
    }
    fn midnight(self, n: i64) -> Result<i64> {
        let wall = i128::from(n) * i128::from(DAY_MS);
        let (a, b, transitions) = match self {
            Self::FixedOffset { offset_minutes: o } => (o, o, None),
            Self::DstRule {
                std_offset_minutes: a,
                dst_offset_minutes: b,
                spring_transition_utc_ms: s,
                fall_transition_utc_ms: f,
            } => (a, b, Some([s, f])),
        };
        // A repeated midnight starts at its first occurrence. A missing midnight
        // starts at the transition that crosses it.
        let regular = [a, b]
            .into_iter()
            .filter_map(|o| i64::try_from(wall - i128::from(o) * 60_000).ok())
            .filter(|&t| self.local(t) == wall);
        regular
            .chain(
                transitions
                    .into_iter()
                    .flatten()
                    .filter(|&t| t > i64::MIN && self.local(t - 1) < wall && self.local(t) >= wall),
            )
            .min()
            .ok_or(RANGE)
    }
}
pub fn local_day_bounds_utc(date: CivilDate, tz: TimeZoneSpec) -> Result<(i64, i64)> {
    let n = days(date)?;
    Ok((tz.midnight(n)?, tz.midnight(n + 1)?))
}
pub fn today_bounds_utc(now_ms: i64, tz: TimeZoneSpec) -> Result<(i64, i64)> {
    last_n_days_bounds_utc(now_ms, 1, tz)
}
pub fn last_n_days_bounds_utc(now_ms: i64, count: u32, tz: TimeZoneSpec) -> Result<(i64, i64)> {
    if count == 0 {
        return Err(StoreError::Invalid("empty calendar interval"));
    }
    let n = tz.local(now_ms).div_euclid(i128::from(DAY_MS)) as i64;
    Ok((tz.midnight(n + 1 - i64::from(count))?, tz.midnight(n + 1)?))
}
pub(crate) fn daily_bounds_utc(now_ms: i64, count: u32, tz: TimeZoneSpec) -> Result<Vec<(i64, i64)>> {
    if count == 0 || count > 366 {
        return Err(StoreError::Invalid("invalid heatmap day count"));
    }
    let n = tz.local(now_ms).div_euclid(i128::from(DAY_MS)) as i64;
    let start_day = n + 1 - i64::from(count);
    let mut bounds = Vec::with_capacity(count as usize);
    for day in start_day..=n {
        bounds.push((tz.midnight(day)?, tz.midnight(day + 1)?));
    }
    Ok(bounds)
}
pub(crate) fn month_start(ms: i64) -> Result<i64> {
    unix_ms_from_civil(CivilDate {
        day: 1,
        ..civil_from_unix_ms(ms)
    })
}
pub(crate) fn next_month(ms: i64) -> Result<i64> {
    let d = civil_from_unix_ms(ms);
    unix_ms_from_civil(CivilDate {
        year: d.year + i32::from(d.month == 12),
        month: d.month % 12 + 1,
        day: 1,
    })
}
