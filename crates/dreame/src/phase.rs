//! What the robot is doing, in the terms an app's home screen uses: a phase, a short sentence,
//! which buttons make sense, and what to tell the user when the status changes. Pure, so it is
//! tested without a robot. The desktop app and the Android app (through `dreame-ffi`) share it.
//!
//! Text is in the current `dreame-i18n` language.

use dreame_model::enums::{CodeEnum, Open, State, TaskStatus};
use dreame_i18n::{tr, trf};
use dreame_model::Status;

/// The robot's situation as the user sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No status read yet.
    Unknown,
    /// Moving and cleaning, or driving to a point.
    Cleaning,
    /// A job is paused.
    Paused,
    /// Driving back to the station.
    Returning,
    /// At the station, busy with the mop, bin or water.
    AtStation,
    /// On the dock, charging or charged.
    Docked,
    /// Off the dock and not working.
    Idle,
    /// Stopped by a fault.
    Error,
}

/// The robot's phase from a status.
pub fn phase(s: &Status) -> Phase {
    let Some(state) = s.state else { return Phase::Unknown };
    let Open::Known(state) = state else { return Phase::Idle };
    use State as S;
    match state {
        S::Error => Phase::Error,
        S::Sweeping
        | S::Mopping
        | S::SweepingAndMopping
        | S::SecondCleaning
        | S::SpotCleaning
        | S::ExtraCleaning
        | S::HeadingToExtraCleaning
        | S::Building
        | S::CleanSummon
        | S::InitialDeepCleaning
        | S::Sanitizing
        | S::SanitizingWithDry
        | S::FloorMaintaining
        | S::Monitoring => Phase::Cleaning,
        S::Paused
        | S::WashingPaused
        | S::DustBagDryingPaused
        | S::InitialDeepCleaningPaused
        | S::ChangingMopPaused
        | S::FloorMaintainingPaused
        | S::MonitoringPaused
        | S::FindingPetPaused => Phase::Paused,
        S::Returning | S::ReturningToWash | S::ReturningAutoEmpty | S::ReturningToDrain => Phase::Returning,
        S::Charging | S::ChargingCompleted | S::SmartCharging => Phase::Docked,
        S::Washing
        | S::Drying
        | S::AutoEmptying
        | S::Emptying
        | S::StationCleaning
        | S::Draining
        | S::AutoWaterDraining
        | S::DustBagDrying
        | S::StationReset
        | S::WaterCheck
        | S::CleanAddWater
        | S::ChangingMop => Phase::AtStation,
        _ => Phase::Idle,
    }
}

/// True while a job is under way, paused or not, including trips to the station in the middle
/// of it. A job ends when the task status goes back to "completed".
pub fn job_active(s: &Status) -> bool {
    match s.task_status {
        Some(Open::Known(TaskStatus::Completed)) | None => false,
        Some(_) => true,
    }
}

/// The error the robot reports, if any, as a short sentence.
pub fn error_text(s: &Status) -> Option<String> {
    let e = s.error.filter(|e| e.code() != 0)?;
    Some(match e.label() {
        Some(l) => trf("{error} (error {code})", &[("error", &capitalise(tr(l))), ("code", &e.code())]),
        None => trf("Error {code}", &[("code", &e.code())]),
    })
}

/// One line for the status card, such as "Cleaning" or "Charging, 64%".
pub fn headline(s: &Status) -> String {
    let p = phase(s);
    let state = s.state.and_then(|x| x.label()).map(|l| capitalise(tr(l)));
    match p {
        Phase::Unknown => tr("Connecting").into(),
        Phase::Docked => match (s.state, s.battery) {
            (Some(Open::Known(State::ChargingCompleted)), _) | (_, Some(100)) => tr("Charged").into(),
            _ => tr("Charging").into(),
        },
        Phase::Cleaning if !job_active(s) => state.unwrap_or_else(|| tr("Cleaning").into()),
        Phase::Cleaning => task_text(s).unwrap_or_else(|| tr("Cleaning").into()),
        Phase::Paused => tr("Paused").into(),
        Phase::Returning if job_active(s) => {
            trf("{state} (job continues)", &[("state", &state.unwrap_or_else(|| tr("Returning").into()))])
        }
        Phase::Returning => tr("Returning to the dock").into(),
        Phase::Error => tr("Needs attention").into(),
        Phase::AtStation | Phase::Idle => state.unwrap_or_else(|| tr("Idle").into()),
    }
}

/// The kind of job, such as "Cleaning rooms", from the task status.
fn task_text(s: &Status) -> Option<String> {
    let Open::Known(t) = s.task_status? else { return None };
    use TaskStatus as T;
    Some(
        tr(match t {
            T::AutoCleaning => "Cleaning the whole home",
            T::ZoneCleaning => "Cleaning a zone",
            T::SegmentCleaning => "Cleaning rooms",
            T::SpotCleaning => "Spot cleaning",
            T::FastMapping => "Mapping",
            T::CruisingPoint => "Going to the point",
            _ => return Some(capitalise(tr(t.label()))),
        })
        .to_string(),
    )
}

/// What the main button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primary {
    /// Start a new job of the chosen kind.
    Start,
    Pause,
    /// Continue the paused or interrupted job.
    Resume,
}

pub fn primary(s: &Status) -> Primary {
    match phase(s) {
        Phase::Cleaning | Phase::Returning if job_active(s) => Primary::Pause,
        Phase::Cleaning => Primary::Pause,
        _ if job_active(s) => Primary::Resume,
        _ => Primary::Start,
    }
}

pub fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// One notice for the user: a title and a line of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    /// Good news, such as a finished job, rather than a problem.
    pub good: bool,
}

/// What changed between `before` and `after` that the user should hear about: a new error, or
/// a job that ended.
pub fn notices(before: &Status, after: &Status) -> Vec<Notice> {
    let mut out = Vec::new();
    let error = |s: &Status| s.error.map(|e| e.code()).filter(|c| *c != 0);
    if let (Some(text), true) = (error_text(after), error(after) != error(before)) {
        out.push(Notice { title: tr("The robot needs attention").into(), body: text, good: false });
    }
    let went_to_point = matches!(before.task_status, Some(Open::Known(TaskStatus::CruisingPoint | TaskStatus::CruisingPointPaused)));
    if went_to_point && !job_active(after) && error(after).is_none() {
        out.push(Notice { title: tr("Go to point finished").into(), body: tr("The robot stopped at the point.").into(), good: true });
    } else if job_active(before) && !job_active(after) && error(after).is_none() {
        let mut parts = Vec::new();
        if let Some(a) = before.cleaned_area_m2.filter(|a| *a > 0) {
            parts.push(trf("{area} m²", &[("area", &a)]));
        }
        if let Some(t) = before.cleaning_time_min.filter(|t| *t > 0) {
            parts.push(trf("{minutes} min", &[("minutes", &t)]));
        }
        let body = match parts.as_slice() {
            [] => tr("The robot finished its job.").to_string(),
            [one] => trf("Cleaned {what}.", &[("what", one)]),
            [area, time] => trf("Cleaned {area} in {time}.", &[("area", area), ("time", time)]),
            _ => unreachable!("at most two parts"),
        };
        out.push(Notice { title: tr("Cleaning finished").into(), body, good: true });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_core::{PropId, RawValue};

    fn status(state: i64, task: i64, battery: i64, error: i64) -> Status {
        let mut s = Status::default();
        s.update(PropId::new(2, 1), &RawValue::Int(state));
        s.update(PropId::new(4, 7), &RawValue::Int(task));
        s.update(PropId::new(3, 1), &RawValue::Int(battery));
        s.update(PropId::new(2, 2), &RawValue::Int(error));
        s
    }

    #[test]
    fn docked_and_idle() {
        let s = status(6, 0, 64, 0);
        assert_eq!(phase(&s), Phase::Docked);
        assert_eq!(headline(&s), "Charging");
        assert_eq!(primary(&s), Primary::Start);
        assert_eq!(headline(&status(13, 0, 100, 0)), "Charged");
        assert_eq!(phase(&Status::default()), Phase::Unknown);
    }

    #[test]
    fn a_room_job_runs_pauses_and_resumes() {
        let s = status(1, 3, 80, 0);
        assert_eq!(phase(&s), Phase::Cleaning);
        assert!(job_active(&s));
        assert_eq!(headline(&s), "Cleaning rooms");
        assert_eq!(primary(&s), Primary::Pause);
        let s = status(3, 8, 80, 0);
        assert_eq!(phase(&s), Phase::Paused);
        assert_eq!(primary(&s), Primary::Resume);
        // Back to the station to wash the mop in the middle of a job.
        let s = status(10, 3, 70, 0);
        assert_eq!(phase(&s), Phase::Returning);
        assert_eq!(primary(&s), Primary::Pause);
        assert!(headline(&s).ends_with("(job continues)"), "{}", headline(&s));
    }

    #[test]
    fn going_to_a_point() {
        // State 98 monitoring, task 22 cruising point.
        let s = status(98, 22, 80, 0);
        assert_eq!(phase(&s), Phase::Cleaning);
        assert_eq!(headline(&s), "Going to the point");
        assert_eq!(primary(&s), Primary::Pause);
        let n = notices(&s, &status(2, 0, 80, 0));
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "Go to point finished");
    }

    #[test]
    fn errors_are_named() {
        let s = status(4, 3, 50, 1);
        assert_eq!(phase(&s), Phase::Error);
        let e = error_text(&s).unwrap();
        assert!(e.ends_with("(error 1)"), "{e}");
        assert_eq!(error_text(&status(6, 0, 50, 0)), None);
    }

    fn job(state: i64, task: i64, error: i64, area: i64, minutes: i64) -> Status {
        let mut s = Status::default();
        for (id, v) in [((2, 1), state), ((4, 7), task), ((2, 2), error), ((4, 3), area), ((4, 2), minutes)] {
            s.update(PropId::new(id.0, id.1), &RawValue::Int(v));
        }
        s
    }

    #[test]
    fn a_finished_job_and_an_error_are_told_once() {
        let running = job(1, 3, 0, 19, 25);
        let docked = job(6, 0, 0, 0, 0);
        let n = notices(&running, &docked);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "Cleaning finished");
        assert!(n[0].good);
        assert_eq!(n[0].body, "Cleaned 19 m² in 25 min.");
        assert!(notices(&docked, &docked).is_empty());
        let stuck = job(4, 3, 1, 19, 25);
        let n = notices(&running, &stuck);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].title, "The robot needs attention");
        assert!(notices(&stuck, &stuck).is_empty());
    }
}
