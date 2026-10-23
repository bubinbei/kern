//! Versioned, piecewise-linear prototype algorithm. No device-specific scaling.
use kern_protocol::*;
use std::collections::BTreeSet;

pub fn validate(map: &CalibrationMap) -> Result<(), String> {
    if map.algorithm != "linear-v1" {
        return Err("Only the versioned linear-v1 algorithm is supported".into());
    }
    if map.parameters.len() > 32 {
        return Err("At most 32 parameters in this prototype".into());
    }
    let mut ids = BTreeSet::new();
    for p in &map.parameters {
        let id_ok = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        };
        if !id_ok(&p.id)
            || !ids.insert(&p.id)
            || !id_ok(&p.source)
            || p.channel >= CHANNEL_RESERVE
            || p.name.is_empty()
            || p.name.len() > 128
            || p.unit.len() > 32
            || p.name.chars().chain(p.unit.chars()).any(char::is_control)
            || !p.min.is_finite()
            || !p.max.is_finite()
            || p.min >= p.max
            || p.color.len() != 7
            || !p.color.starts_with('#')
            || !p.color[1..].bytes().all(|b| b.is_ascii_hexdigit())
            || !(2..=32).contains(&p.points.len())
            || p.points
                .iter()
                .any(|v| !v.raw.is_finite() || !v.value.is_finite())
            || p.points.windows(2).any(|w| w[0].raw >= w[1].raw)
        {
            return Err(format!("Invalid parameter/map: {}", p.id));
        }
    }
    Ok(())
}
pub fn convert(map: &CalibrationMap, batch: &Batch) -> Vec<ParameterSample> {
    map.parameters
        .iter()
        .filter(|p| p.source == batch.source)
        .filter_map(|p| {
            let sample = batch.samples.iter().find(|s| s.channel == p.channel)?;
            let raw = match sample.value.as_ref() {
                Some(Value::Float(v)) => Some(*v),
                Some(Value::Unsigned(v)) if *v <= 9_007_199_254_740_992 => Some(*v as f64),
                Some(Value::Signed(v)) if v.unsigned_abs() <= 9_007_199_254_740_992 => {
                    Some(*v as f64)
                }
                Some(Value::Boolean(v)) => Some(u8::from(*v) as f64),
                _ => None,
            };
            let value = raw
                .filter(|_| sample.quality == Quality::Good)
                .and_then(|x| {
                    let first = &p.points[0];
                    let last = p.points.last()?;
                    if !p.extrapolate && (x < first.raw || x > last.raw) {
                        return None;
                    }
                    let i = p
                        .points
                        .partition_point(|v| v.raw < x)
                        .clamp(1, p.points.len() - 1);
                    let a = &p.points[i - 1];
                    let b = &p.points[i];
                    let y = a.value + (x - a.raw) / (b.raw - a.raw) * (b.value - a.value);
                    y.is_finite().then_some(y)
                });
            Some(ParameterSample {
                id: p.id.clone(),
                value,
                quality: if value.is_some() {
                    Quality::Good
                } else if sample.quality == Quality::Missing {
                    Quality::Missing
                } else {
                    Quality::Bad
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn map() -> CalibrationMap {
        CalibrationMap {
            algorithm: "linear-v1".into(),
            parameters: vec![ParameterDef {
                id: "weight".into(),
                name: "Weight".into(),
                unit: "t".into(),
                source: "test".into(),
                channel: 0,
                points: vec![
                    CalibrationPoint {
                        raw: 1000.,
                        value: 0.,
                    },
                    CalibrationPoint {
                        raw: 8800.,
                        value: 1.,
                    },
                ],
                min: 0.,
                max: 2.,
                color: "#fabd2f".into(),
                extrapolate: false,
            }],
        }
    }
    #[test]
    fn points_missing_range_and_integer_precision() {
        let m = map();
        validate(&m).unwrap();
        for (raw, expected) in [
            (1000, Some(0.)),
            (4900, Some(0.5)),
            (8800, Some(1.)),
            (0, None),
            (u64::MAX, None),
        ] {
            let b = Batch {
                source: "test".into(),
                session: 1,
                sequence: 1,
                measured_unix_ms: 0,
                samples: vec![Sample {
                    channel: 0,
                    value: Some(Value::Unsigned(raw)),
                    quality: Quality::Good,
                }],
            };
            assert_eq!(convert(&m, &b)[0].value, expected);
        }
        let mut bad = m;
        bad.parameters[0].points[1].raw = 1000.;
        assert!(validate(&bad).is_err());
    }
}
