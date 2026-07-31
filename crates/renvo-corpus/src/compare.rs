use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Named structured outcome from one compiler/run variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedObservation {
    /// Variant identifier.
    pub name: String,
    /// Run artifact or selected observation object.
    pub value: Value,
}

/// One unequal selected field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    /// JSON Pointer selecting the compared field.
    pub pointer: String,
    /// Baseline value, or null when absent.
    pub baseline: Value,
    /// Candidate value, or null when absent.
    pub candidate: Value,
}

/// Comparison against the first observation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// Baseline variant.
    pub baseline: String,
    /// Candidate variant.
    pub candidate: String,
    /// Selected unequal fields.
    pub differences: Vec<Difference>,
}

impl Comparison {
    /// True when every selected field is equivalent.
    pub fn equivalent(&self) -> bool {
        self.differences.is_empty()
    }
}

/// Compares every observation to the first using explicit JSON Pointers.
pub fn compare_observations(
    observations: &[NamedObservation],
    pointers: &[String],
) -> Vec<Comparison> {
    let Some(baseline) = observations.first() else {
        return Vec::new();
    };
    observations[1..]
        .iter()
        .map(|candidate| {
            let differences = pointers
                .iter()
                .filter_map(|pointer| {
                    let baseline_value = baseline.value.pointer(pointer).cloned();
                    let candidate_value = candidate.value.pointer(pointer).cloned();
                    (baseline_value != candidate_value).then(|| Difference {
                        pointer: pointer.clone(),
                        baseline: baseline_value.unwrap_or(Value::Null),
                        candidate: candidate_value.unwrap_or(Value::Null),
                    })
                })
                .collect();
            Comparison {
                baseline: baseline.name.clone(),
                candidate: candidate.name.clone(),
                differences,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_only_declared_observables() {
        let observations = [
            NamedObservation {
                name: "gcc".to_owned(),
                value: json!({"exit_code": 0, "time": 4}),
            },
            NamedObservation {
                name: "clang".to_owned(),
                value: json!({"exit_code": 1, "time": 4}),
            },
        ];
        let comparisons = compare_observations(&observations, &["/exit_code".to_owned()]);
        assert_eq!(comparisons[0].differences.len(), 1);
        assert_eq!(comparisons[0].differences[0].candidate, json!(1));
    }
}
