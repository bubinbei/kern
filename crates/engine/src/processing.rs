//! Bounded deterministic graph. Context is reconstructed from committed parameter events.
use kern_protocol::*;
use std::collections::{BTreeMap, BTreeSet};

pub fn inputs(p: &ParameterDef) -> &[String] {
    match &p.provider {
        Some(Provider::Calculation { inputs, .. }) => inputs,
        _ => &[],
    }
}
pub fn order(map: &CalibrationMap) -> Result<Vec<usize>, String> {
    fn visit(
        i: usize,
        map: &CalibrationMap,
        visiting: &mut BTreeSet<usize>,
        done: &mut BTreeSet<usize>,
        out: &mut Vec<usize>,
    ) -> Result<(), String> {
        if done.contains(&i) {
            return Ok(());
        }
        if !visiting.insert(i) {
            return Err(format!("Calculation cycle: {}", map.parameters[i].id));
        }
        for id in inputs(&map.parameters[i]) {
            let index = map
                .parameters
                .iter()
                .position(|p| &p.id == id)
                .ok_or_else(|| format!("Missing dependency: {id}"))?;
            visit(index, map, visiting, done, out)?;
        }
        visiting.remove(&i);
        done.insert(i);
        out.push(i);
        Ok(())
    }
    let (mut out, mut visiting, mut done) = (Vec::new(), BTreeSet::new(), BTreeSet::new());
    for i in 0..map.parameters.len() {
        visit(i, map, &mut visiting, &mut done, &mut out)?;
    }
    Ok(out)
}
fn affected_by_source(map: &CalibrationMap, source: &str, indices: &[usize]) -> BTreeSet<String> {
    let mut affected = BTreeSet::new();
    for &i in indices {
        let p = &map.parameters[i];
        if (p.provider.is_none() && p.source == source)
            || inputs(p).iter().any(|id| affected.contains(id))
        {
            affected.insert(p.id.clone());
        }
    }
    affected
}
pub fn validate(map: &CalibrationMap) -> Result<(), String> {
    if map.algorithm == "linear-v1" && map.parameters.iter().any(|p| p.provider.is_some()) {
        return Err("Providers require graph-v1".into());
    }
    let mut rates = 0;
    for p in &map.parameters {
        match &p.provider {
            Some(Provider::Manual { value }) if value.is_some_and(|v| !v.is_finite()) => {
                return Err("Manual value must be finite".into());
            }
            Some(Provider::Calculation {
                inputs,
                operation,
                max_age_ms,
            }) => {
                if inputs.is_empty() || inputs.len() > 16 || !(100..=3_600_000).contains(max_age_ms)
                {
                    return Err(format!("Invalid inputs/age: {}", p.id));
                }
                let count = inputs.len();
                let units: Vec<_> = inputs
                    .iter()
                    .filter_map(|id| {
                        map.parameters
                            .iter()
                            .find(|v| &v.id == id)
                            .map(|v| v.unit.as_str())
                    })
                    .collect();
                if units.len() != count {
                    return Err(format!("Missing dependency: {}", p.id));
                }
                let same = units.iter().all(|u| *u == p.unit);
                let valid = match operation {
                    Operation::Sum => same,
                    Operation::Difference => count == 2 && same,
                    Operation::Ratio { factor } => {
                        count == 2
                            && factor.is_finite()
                            && units[0] == units[1]
                            && matches!(p.unit.as_str(), "%" | "ед.")
                    }
                    Operation::Scale { factor, offset } => {
                        count == 1 && factor.is_finite() && offset.is_finite()
                    }
                    Operation::PumpFlow {
                        liters_per_stroke,
                        efficiency,
                    } => {
                        count == 1
                            && units[0] == "ход/мин"
                            && p.unit == "л/с"
                            && liters_per_stroke.is_finite()
                            && *liters_per_stroke > 0.
                            && efficiency.is_finite()
                            && *efficiency > 0.
                            && *efficiency <= 1.
                    }
                    Operation::Lookup { points } => {
                        count == 1
                            && ((!p.enabled && points.is_empty())
                                || (2..=32).contains(&points.len()))
                            && points
                                .iter()
                                .all(|x| x.raw.is_finite() && x.value.is_finite())
                            && points.windows(2).all(|w| w[0].raw < w[1].raw)
                    }
                    Operation::WeightOnBit { max_off_bottom } => {
                        count == 3
                            && units == ["т", "т", "м"]
                            && p.unit == "т"
                            && max_off_bottom.is_finite()
                            && *max_off_bottom >= 0.
                    }
                    Operation::Rate {
                        window_seconds,
                        per_hour,
                        ..
                    } => {
                        rates += 1;
                        count == 1
                            && (1..=300).contains(window_seconds)
                            && units[0] == "м"
                            && p.unit == if *per_hour { "м/ч" } else { "м/с" }
                    }
                };
                if !valid {
                    return Err(format!(
                        "Invalid operation, units or coefficients: {}",
                        p.id
                    ));
                }
            }
            _ => {}
        }
    }
    if rates > 8 {
        return Err("At most eight rate windows".into());
    }
    order(map)?;
    Ok(())
}
fn sample(
    id: &str,
    value: Option<f64>,
    reason: Option<&str>,
    time: Option<i64>,
) -> ParameterSample {
    let finite = value.filter(|v| v.is_finite());
    ParameterSample {
        id: id.into(),
        value: finite,
        quality: if finite.is_some() {
            Quality::Good
        } else if reason == Some("division_by_zero")
            || reason == Some("nonfinite")
            || reason == Some("clock_regression")
        {
            Quality::Bad
        } else {
            Quality::Missing
        },
        reason: reason.map(str::to_owned),
        input_time_ms: time,
    }
}
fn push_point(points: &mut Vec<HistoryPoint>, point: HistoryPoint, seconds: u16, max_age: u64) {
    if let Some(last) = points.last() {
        if point.time_ms == last.time_ms {
            return;
        }
        if point.time_ms < last.time_ms || point.time_ms - last.time_ms > max_age as i64 {
            points.clear();
        }
    }
    if let Some(last) = points.last_mut()
        && point.time_ms.div_euclid(1000) == last.time_ms.div_euclid(1000)
    {
        *last = point;
        return;
    }
    let cutoff = point.time_ms - i64::from(seconds) * 1000;
    while points.len() > 1 && points[1].time_ms <= cutoff {
        points.remove(0);
    }
    points.push(point);
    while points.len() > 302 {
        points.remove(0);
    }
}
pub fn evaluate(
    map: &CalibrationMap,
    batch: &Batch,
    time: i64,
    context: &ProcessingContext,
    revision: u64,
) -> Vec<ParameterSample> {
    evaluate_ordered(
        map,
        batch,
        time,
        context,
        revision,
        &order(map).unwrap_or_default(),
    )
}
pub fn evaluate_ordered(
    map: &CalibrationMap,
    batch: &Batch,
    time: i64,
    context: &ProcessingContext,
    revision: u64,
    indices: &[usize],
) -> Vec<ParameterSample> {
    if map.algorithm == "linear-v1" {
        return crate::calibration::convert(map, batch);
    }
    let mut values: BTreeMap<String, ContextValue> = if context.revision == revision {
        context.values.clone()
    } else {
        BTreeMap::new()
    };
    let new_session = context.source_sessions.get(&batch.source) != Some(&batch.session);
    let reset = if new_session {
        affected_by_source(map, &batch.source, indices)
    } else {
        BTreeSet::new()
    };
    if new_session {
        for p in &map.parameters {
            if p.provider.is_none() && p.source == batch.source {
                values.remove(&p.id);
            }
        }
    }
    let mut output = BTreeMap::new();
    for mut v in crate::calibration::convert(map, batch) {
        v.input_time_ms = Some(time);
        if v.value.is_none() {
            v.reason = Some("missing_input".into());
        }
        values.insert(
            v.id.clone(),
            ContextValue {
                sample: v.clone(),
                received_unix_ms: time,
            },
        );
        output.insert(v.id.clone(), v);
    }
    for &i in indices {
        let p = &map.parameters[i];
        let Some(provider) = &p.provider else {
            continue;
        };
        let v = if !p.enabled {
            sample(&p.id, None, Some("disabled"), Some(time))
        } else {
            match provider {
                Provider::Manual { value } => sample(
                    &p.id,
                    *value,
                    if value.is_none() {
                        Some("not_configured")
                    } else {
                        None
                    },
                    Some(time),
                ),
                Provider::Calculation {
                    inputs,
                    operation,
                    max_age_ms,
                } => {
                    let args: Vec<_> = inputs.iter().filter_map(|id| values.get(id)).collect();
                    if args.len() != inputs.len()
                        || inputs
                            .iter()
                            .any(|id| map.parameters.iter().any(|p| &p.id == id && !p.enabled))
                    {
                        sample(&p.id, None, Some("missing_input"), Some(time))
                    } else if args
                        .iter()
                        .any(|v| v.sample.quality != Quality::Good || v.sample.value.is_none())
                    {
                        sample(&p.id, None, Some("invalid_input"), Some(time))
                    } else if args.iter().any(|v| {
                        time < v.received_unix_ms || time - v.received_unix_ms > *max_age_ms as i64
                    }) {
                        sample(&p.id, None, Some("stale_input"), Some(time))
                    } else {
                        let a: Vec<_> = args.iter().map(|v| v.sample.value.unwrap()).collect();
                        let input_time = args
                            .iter()
                            .map(|v| v.received_unix_ms)
                            .min()
                            .unwrap_or(time);
                        let mut reason = None;
                        let value = match operation {
                            Operation::Sum => Some(a.iter().sum()),
                            Operation::Difference => Some(a[0] - a[1]),
                            Operation::Ratio { factor } => {
                                if a[1] == 0. {
                                    reason = Some("division_by_zero");
                                    None
                                } else {
                                    Some(a[0] / a[1] * factor)
                                }
                            }
                            Operation::Scale { factor, offset } => Some(a[0] * factor + offset),
                            Operation::PumpFlow {
                                liters_per_stroke,
                                efficiency,
                            } => Some(a[0] * liters_per_stroke * efficiency / 60.),
                            Operation::Lookup { points } => {
                                if a[0] < points[0].raw || a[0] > points.last().unwrap().raw {
                                    reason = Some("out_of_range");
                                    None
                                } else {
                                    let i = points
                                        .partition_point(|v| v.raw < a[0])
                                        .clamp(1, points.len() - 1);
                                    let (x, y) = (&points[i - 1], &points[i]);
                                    Some(
                                        x.value
                                            + (a[0] - x.raw) / (y.raw - x.raw)
                                                * (y.value - x.value),
                                    )
                                }
                            }
                            Operation::WeightOnBit { max_off_bottom } => {
                                if a[0] <= 0. {
                                    reason = Some("not_configured");
                                    None
                                } else {
                                    Some(if a[2] > *max_off_bottom {
                                        0.
                                    } else {
                                        a[0] - a[1]
                                    })
                                }
                            }
                            Operation::Rate {
                                window_seconds,
                                per_hour,
                                positive_only,
                            } => {
                                let mut h =
                                    if context.revision == revision && !reset.contains(&p.id) {
                                        context.history.get(&p.id).cloned().unwrap_or_default()
                                    } else {
                                        vec![]
                                    };
                                if h.last().is_some_and(|v| input_time < v.time_ms) {
                                    reason = Some("clock_regression");
                                    None
                                } else {
                                    push_point(
                                        &mut h,
                                        HistoryPoint {
                                            time_ms: input_time,
                                            value: a[0],
                                        },
                                        *window_seconds,
                                        *max_age_ms,
                                    );
                                    let target = input_time - i64::from(*window_seconds) * 1000;
                                    if h.len() < 2 || h[0].time_ms > target {
                                        reason = Some("warming_up");
                                        None
                                    } else {
                                        let n = h
                                            .partition_point(|v| v.time_ms <= target)
                                            .clamp(1, h.len() - 1);
                                        let (l, r) = (&h[n - 1], &h[n]);
                                        let at = l.value
                                            + (r.value - l.value) * (target - l.time_ms) as f64
                                                / (r.time_ms - l.time_ms) as f64;
                                        let delta = a[0] - at;
                                        Some(
                                            (if *positive_only { delta.max(0.) } else { delta })
                                                / f64::from(*window_seconds)
                                                * if *per_hour { 3600. } else { 1. },
                                        )
                                    }
                                }
                            }
                        };
                        sample(
                            &p.id,
                            value,
                            if value.is_some_and(|v| !v.is_finite()) {
                                Some("nonfinite")
                            } else {
                                reason
                            },
                            Some(input_time),
                        )
                    }
                }
            }
        };
        values.insert(
            p.id.clone(),
            ContextValue {
                received_unix_ms: v.input_time_ms.unwrap_or(time),
                sample: v.clone(),
            },
        );
        output.insert(p.id.clone(), v);
    }
    map.parameters
        .iter()
        .filter_map(|p| output.remove(&p.id))
        .collect()
}
/// Called only for a durably accepted result, both during replay and live operation.
pub fn advance(context: &mut ProcessingContext, map: &CalibrationMap, batch: &ParameterBatch) {
    if map.algorithm == "linear-v1" {
        *context = ProcessingContext::default();
        return;
    }
    if context.revision != batch.revision {
        *context = ProcessingContext {
            revision: batch.revision,
            ..Default::default()
        };
    }
    if context.source_sessions.get(&batch.source) != Some(&batch.source_session) {
        for p in &map.parameters {
            if p.provider.is_none() && p.source == batch.source {
                context.values.remove(&p.id);
            }
        }
        let affected = affected_by_source(map, &batch.source, &order(map).unwrap_or_default());
        context.history.retain(|id, _| !affected.contains(id));
    }
    context
        .source_sessions
        .insert(batch.source.clone(), batch.source_session);
    for value in &batch.values {
        context.values.insert(
            value.id.clone(),
            ContextValue {
                sample: value.clone(),
                received_unix_ms: value.input_time_ms.unwrap_or(batch.received_unix_ms),
            },
        );
    }
    for p in &map.parameters {
        if let Some(Provider::Calculation {
            inputs,
            operation: Operation::Rate { window_seconds, .. },
            max_age_ms,
        }) = &p.provider
        {
            let h = context.history.entry(p.id.clone()).or_default();
            if let Some(v) = context.values.get(&inputs[0]) {
                if v.sample.quality == Quality::Good
                    && let Some(value) = v.sample.value
                {
                    push_point(
                        h,
                        HistoryPoint {
                            time_ms: v.received_unix_ms,
                            value,
                        },
                        *window_seconds,
                        *max_age_ms,
                    );
                } else {
                    h.clear();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sensor(id: &str, source: &str, channel: u16, unit: &str) -> ParameterDef {
        ParameterDef {
            id: id.into(),
            name: id.into(),
            unit: unit.into(),
            source: source.into(),
            channel,
            points: vec![
                CalibrationPoint { raw: 0., value: 0. },
                CalibrationPoint {
                    raw: 10000.,
                    value: 10000.,
                },
            ],
            min: 0.,
            max: 10000.,
            color: "#fabd2f".into(),
            enabled: true,
            extrapolate: false,
            provider: None,
        }
    }
    fn calculation(id: &str, inputs: &[&str], unit: &str, operation: Operation) -> ParameterDef {
        let mut p = sensor(id, "unassigned", 0, unit);
        p.points.clear();
        p.provider = Some(Provider::Calculation {
            inputs: inputs.iter().map(|v| (*v).into()).collect(),
            operation,
            max_age_ms: 5000,
        });
        p
    }
    fn tick(
        map: &CalibrationMap,
        ctx: &mut ProcessingContext,
        t: i64,
        value: f64,
        session: u64,
    ) -> Vec<ParameterSample> {
        let raw = Batch {
            source: "test".into(),
            session,
            sequence: t as u64,
            measured_unix_ms: t,
            samples: vec![Sample {
                channel: 0,
                value: Some(Value::Float(value)),
                quality: Quality::Good,
            }],
        };
        let values = evaluate(map, &raw, t, ctx, 1);
        advance(
            ctx,
            map,
            &ParameterBatch {
                raw_event: t as u64,
                revision: 1,
                source: raw.source,
                source_session: session,
                source_sequence: raw.sequence,
                measured_unix_ms: t,
                received_unix_ms: t,
                values: values.clone(),
            },
        );
        values
    }
    #[test]
    fn catalog_and_all_presets_are_complete_and_fit_the_map_budget() {
        let library: serde_json::Value =
            serde_json::from_str(include_str!("../../processor/parameter-library.json")).unwrap();
        let parameters: Vec<ParameterDef> = library["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| serde_json::from_value(e["parameter"].clone()).unwrap())
            .collect();
        assert_eq!(parameters.len(), 81);
        crate::calibration::validate(&CalibrationMap {
            algorithm: "graph-v1".into(),
            parameters: parameters.clone(),
        })
        .unwrap();
        for preset in library["presets"].as_array().unwrap() {
            let list = preset["parameter_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| {
                    parameters
                        .iter()
                        .find(|p| p.id == id.as_str().unwrap())
                        .unwrap()
                        .clone()
                })
                .collect();
            crate::calibration::validate(&CalibrationMap {
                algorithm: "graph-v1".into(),
                parameters: list,
            })
            .unwrap();
        }
    }
    #[test]
    fn graph_rejects_cycles_missing_dependencies_units_and_bad_coefficients() {
        let mut m = CalibrationMap {
            algorithm: "graph-v1".into(),
            parameters: vec![
                sensor("a", "test", 0, "т"),
                calculation("b", &["a"], "т", Operation::Sum),
            ],
        };
        crate::calibration::validate(&m).unwrap();
        m.parameters[0] = calculation("a", &["b"], "т", Operation::Sum);
        assert!(crate::calibration::validate(&m).is_err());
        m.parameters[0] = sensor("a", "test", 0, "м");
        assert!(crate::calibration::validate(&m).is_err());
        m.parameters.pop();
        m.parameters
            .push(calculation("b", &["absent"], "м", Operation::Sum));
        assert!(crate::calibration::validate(&m).is_err());
        m.parameters[1] = calculation(
            "b",
            &["a"],
            "л/с",
            Operation::PumpFlow {
                liters_per_stroke: 1.,
                efficiency: 2.,
            },
        );
        assert!(crate::calibration::validate(&m).is_err());
    }
    #[test]
    fn exact_math_and_division_by_zero_never_fabricate_a_zero() {
        let mut manual = sensor("constant", "unassigned", 0, "т");
        manual.provider = Some(Provider::Manual { value: Some(100.) });
        let m = CalibrationMap {
            algorithm: "graph-v1".into(),
            parameters: vec![
                sensor("a", "test", 0, "т"),
                manual,
                calculation("sum", &["a", "constant"], "т", Operation::Sum),
                calculation(
                    "ratio",
                    &["constant", "a"],
                    "%",
                    Operation::Ratio { factor: 100. },
                ),
            ],
        };
        let mut ctx = ProcessingContext::default();
        let out = tick(&m, &mut ctx, 1000, 25., 1);
        assert_eq!(
            out.iter().find(|p| p.id == "sum").unwrap().value,
            Some(125.)
        );
        assert_eq!(
            out.iter().find(|p| p.id == "ratio").unwrap().value,
            Some(400.)
        );
        let out = tick(&m, &mut ctx, 2000, 0., 1);
        let ratio = out.iter().find(|p| p.id == "ratio").unwrap();
        assert_eq!(ratio.value, None);
        assert_eq!(ratio.quality, Quality::Bad);
        assert_eq!(ratio.reason.as_deref(), Some("division_by_zero"));
    }
    #[test]
    fn window_rate_restores_exactly_from_committed_results_and_resets_on_session_change() {
        let m = CalibrationMap {
            algorithm: "graph-v1".into(),
            parameters: vec![
                sensor("depth", "test", 0, "м"),
                calculation(
                    "rate",
                    &["depth"],
                    "м/ч",
                    Operation::Rate {
                        window_seconds: 10,
                        per_hour: true,
                        positive_only: true,
                    },
                ),
            ],
        };
        let (mut ctx, mut restored) = (ProcessingContext::default(), ProcessingContext::default());
        for n in 1..=12 {
            let t = n * 1000;
            let result = tick(&m, &mut ctx, t, n as f64 * 0.5, 1);
            if n < 11 {
                assert_eq!(result[1].reason.as_deref(), Some("warming_up"));
            } else {
                assert!((result[1].value.unwrap() - 1800.).abs() < 1e-9);
            }
            restored = serde_json::from_slice(&serde_json::to_vec(&ctx).unwrap()).unwrap();
        }
        assert_eq!(
            tick(&m, &mut ctx, 13000, 6.5, 1),
            tick(&m, &mut restored, 13000, 6.5, 1)
        );
        assert_eq!(
            tick(&m, &mut ctx, 14000, 7., 2)[1].reason.as_deref(),
            Some("warming_up")
        );
        assert_eq!(
            tick(&m, &mut ctx, 13000, 7.5, 2)[1].reason.as_deref(),
            Some("clock_regression")
        );
    }
    #[test]
    fn separate_sources_use_recorded_age_and_missing_inputs_propagate() {
        let m = CalibrationMap {
            algorithm: "graph-v1".into(),
            parameters: vec![
                sensor("a", "test", 0, "м"),
                sensor("b", "other", 0, "м"),
                calculation("difference", &["a", "b"], "м", Operation::Difference),
            ],
        };
        let mut ctx = ProcessingContext::default();
        assert_eq!(tick(&m, &mut ctx, 1000, 10., 1)[1].value, None);
        let raw = Batch {
            source: "other".into(),
            session: 1,
            sequence: 1,
            measured_unix_ms: 2000,
            samples: vec![Sample {
                channel: 0,
                value: Some(Value::Float(3.)),
                quality: Quality::Good,
            }],
        };
        let out = evaluate(&m, &raw, 2000, &ctx, 1);
        assert_eq!(out[1].value, Some(7.));
        let out = evaluate(&m, &raw, 8000, &ctx, 1);
        assert_eq!(out[1].reason.as_deref(), Some("stale_input"));
        let out = evaluate(&m, &raw, 2000, &ctx, 2);
        assert_eq!(out[1].value, None);
    }
    #[test]
    fn eight_maximum_windows_and_full_parameter_context_fit_reply_bound() {
        let mut p = vec![sensor("depth", "test", 0, "м")];
        for n in 0..8 {
            p.push(calculation(
                &format!("rate{n}"),
                &["depth"],
                "м/с",
                Operation::Rate {
                    window_seconds: 300,
                    per_hour: false,
                    positive_only: false,
                },
            ));
        }
        let m = CalibrationMap {
            algorithm: "graph-v1".into(),
            parameters: p,
        };
        let mut ctx = ProcessingContext::default();
        for n in 1..=315 {
            tick(&m, &mut ctx, n * 1000, n as f64, 1);
        }
        assert!(ctx.history.values().all(|v| v.len() <= 302));
        assert!(
            serde_json::to_vec(&Reply::ProcessingContext {
                position: 1,
                context: ctx
            })
            .unwrap()
            .len()
                < MAX_REPLY
        );
    }
}
