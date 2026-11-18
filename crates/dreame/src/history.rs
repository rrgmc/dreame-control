//! Past cleaning jobs. A robot sends a cleaning record (event 4.e1) when a job ends; the cloud
//! keeps them, and [`jobs`] reads them back through the event history API. Read-only.

use crate::CloudRobot;
use anyhow::{anyhow, Result};
use dreame_core::RawValue;
use dreame_model::codec::record::CleaningRecord;
use dreame_model::enums::{CleanLogStatus, Open};
use serde_json::Value;

/// The cleaning-record event: service 4, event 1.
const RECORD: (u32, u32) = (4, 1);
/// The earliest time to ask for, in Unix seconds (2023-06-17); the cloud keeps less.
const SINCE: i64 = 1_687_019_188;

/// One past job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub record: CleaningRecord,
    /// When the cloud stored the record, Unix seconds, if the entry says.
    pub stored_at: Option<i64>,
}

impl Job {
    /// When the job started, Unix seconds: the record's start time, else when it was stored.
    pub fn started(&self) -> Option<i64> {
        self.record.start_time.or(self.stored_at)
    }
}

/// The last `limit` jobs, newest first.
pub async fn jobs(robot: &CloudRobot, limit: u32) -> Result<Vec<Job>> {
    let entries = robot.client.event_history(&robot.device.did, RECORD.0, RECORD.1, SINCE, limit).await?;
    let mut jobs: Vec<Job> = entries.iter().filter_map(|e| job_from_entry(e).ok()).collect();
    jobs.sort_by_key(|j| std::cmp::Reverse(j.started()));
    Ok(jobs)
}

/// The map the robot saved at the end of a job, with the job's path, or `None` when the record
/// names none. Read-only.
pub async fn job_map(robot: &CloudRobot, job: &Job) -> Result<Option<dreame_map::Map>> {
    let Some(obj) = &job.record.log_object else { return Ok(None) };
    let text = crate::cloud::download_object(&robot.client, &robot.device, &obj.object).await?;
    let frame = dreame_map::Frame::decode(&text, obj.key.as_deref(), None)?;
    Ok(Some(dreame_map::Map::from_frame(&frame, crate::mapedit::MAP_VERSION, false)?))
}

/// Decodes one history entry. The arguments sit under `history` (as on r2579a), `value` or
/// `arguments`, as a list or as JSON text holding one; the time under `createTime` (r2579a),
/// `time` or `timestamp`, in seconds or milliseconds.
pub fn job_from_entry(e: &Value) -> Result<Job> {
    Ok(Job { record: record_from_entry(e)?, stored_at: entry_time(e) })
}

/// The cleaning record in one history entry.
pub fn record_from_entry(e: &Value) -> Result<CleaningRecord> {
    let args = ["history", "value", "arguments", "args"].iter().find_map(|k| e.get(*k)).ok_or_else(|| anyhow!("no arguments"))?;
    let list = match args {
        Value::String(s) => serde_json::from_str(s)?,
        other => other.clone(),
    };
    // Some entries wrap the list: {"arguments": [...]}.
    let list = match list {
        Value::Object(o) => o.get("arguments").cloned().ok_or_else(|| anyhow!("no arguments list"))?,
        other => other,
    };
    Ok(CleaningRecord::from_json(&list)?)
}

fn entry_time(e: &Value) -> Option<i64> {
    let t = ["time", "createTime", "timestamp", "updateTime"].iter().find_map(|k| match e.get(*k)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })?;
    // Milliseconds after 2001-09-09 have 13 digits.
    Some(if t > 99_999_999_999 { t / 1000 } else { t })
}

/// How the job ended, in a few words.
pub fn outcome(r: &CleaningRecord) -> &'static str {
    match r.result {
        Some(Open::Known(CleanLogStatus::Completed)) => "completed",
        Some(Open::Known(CleanLogStatus::ManuallyEnded)) => "ended by hand",
        Some(Open::Known(CleanLogStatus::Interrupted)) => "interrupted",
        Some(Open::Known(CleanLogStatus::Failed)) => "failed",
        _ => "ended",
    }
}

/// One line about a job, with the start in UTC: for logs and the command line.
pub fn summary(r: &CleaningRecord) -> String {
    let when = r.start_time.map(utc).unwrap_or_else(|| "unknown time".into());
    let area = r.cleaned_area_m2.map(|a| format!("{a} m²")).unwrap_or_default();
    let mins = r.cleaning_time_min.map(|m| format!("{m} min")).unwrap_or_default();
    format!("{when} UTC  {mins:>7}  {area:>7}  {}", outcome(r))
}

/// `YYYY-MM-DD HH:MM` in UTC for Unix seconds.
pub fn utc(t: i64) -> String {
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

/// The raw value of an argument, for fields the record does not decode.
pub fn other(r: &CleaningRecord, piid: u32) -> Option<&RawValue> {
    r.other.get(&piid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entries_decode_in_either_form() {
        let args = json!([
            {"piid": 2, "value": 25},
            {"piid": 3, "value": 19},
            {"piid": 8, "value": 1_794_940_000},
            {"piid": 13, "value": 1}
        ]);
        for e in [
            json!({"value": args.to_string(), "time": 1_794_941_500_000_i64}),
            json!({"arguments": args.clone(), "createTime": "1794941500"}),
        ] {
            let j = job_from_entry(&e).unwrap();
            assert_eq!(j.record.cleaning_time_min, Some(25));
            assert_eq!(j.record.cleaned_area_m2, Some(19));
            assert_eq!(j.stored_at, Some(1_794_941_500));
            assert_eq!(j.started(), Some(1_794_940_000));
            assert_eq!(outcome(&j.record), "completed");
        }
        assert!(job_from_entry(&json!({"other": 1})).is_err());
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00");
        assert_eq!(utc(1_794_940_000), "2026-11-17 18:26");
    }
}
