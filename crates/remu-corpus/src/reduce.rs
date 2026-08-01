use serde::{Deserialize, Serialize};

/// Result of deterministic delta debugging over an ordered sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReductionResult<T> {
    /// Smallest sequence found by the deterministic partition strategy.
    pub items: Vec<T>,
    /// Number of predicate evaluations.
    pub evaluations: u64,
}

/// Source, compiler-flag, and external-input axes for one reproducible case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReductionCandidate {
    /// Independently removable source fragments.
    pub source: Vec<String>,
    /// Independently removable compiler arguments.
    pub flags: Vec<String>,
    /// Independently removable integer stimuli.
    pub inputs: Vec<u32>,
}

/// Result of reducing all three corpus axes in a stable order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseReductionResult {
    /// Original discrepancy-producing case.
    pub original: ReductionCandidate,
    /// Smallest case found by source, then flag, then input reduction.
    pub minimized: ReductionCandidate,
    /// Total predicate evaluations across all axes.
    pub evaluations: u64,
}

/// Reduces an ordered sequence while `interesting` remains true.
///
/// The predicate should be deterministic and must be true for the initial
/// sequence. Chunks are considered from lowest index to highest index.
pub fn reduce_sequence<T: Clone>(
    items: Vec<T>,
    mut interesting: impl FnMut(&[T]) -> bool,
) -> ReductionResult<T> {
    let mut evaluations = 1;
    if !interesting(&items) {
        return ReductionResult { items, evaluations };
    }
    let mut current = items;
    let mut partitions = 2_usize;
    while current.len() >= 2 {
        let chunk = current.len().div_ceil(partitions);
        let mut reduced = false;
        let mut start = 0;
        while start < current.len() {
            let end = (start + chunk).min(current.len());
            let candidate = current[..start]
                .iter()
                .chain(&current[end..])
                .cloned()
                .collect::<Vec<_>>();
            evaluations += 1;
            if interesting(&candidate) {
                current = candidate;
                partitions = partitions.saturating_sub(1).max(2);
                reduced = true;
                break;
            }
            start = end;
        }
        if !reduced {
            if partitions >= current.len() {
                break;
            }
            partitions = (partitions * 2).min(current.len());
        }
    }
    ReductionResult {
        items: current,
        evaluations,
    }
}

/// Fallible form of [`reduce_sequence`] for compilers and emulators.
pub fn try_reduce_sequence<T: Clone, E>(
    items: Vec<T>,
    mut interesting: impl FnMut(&[T]) -> Result<bool, E>,
) -> Result<ReductionResult<T>, E> {
    let mut evaluations = 1;
    if !interesting(&items)? {
        return Ok(ReductionResult { items, evaluations });
    }
    let mut current = items;
    let mut partitions = 2_usize;
    while current.len() >= 2 {
        let chunk = current.len().div_ceil(partitions);
        let mut reduced = false;
        let mut start = 0;
        while start < current.len() {
            let end = (start + chunk).min(current.len());
            let candidate = current[..start]
                .iter()
                .chain(&current[end..])
                .cloned()
                .collect::<Vec<_>>();
            evaluations += 1;
            if interesting(&candidate)? {
                current = candidate;
                partitions = partitions.saturating_sub(1).max(2);
                reduced = true;
                break;
            }
            start = end;
        }
        if !reduced {
            if partitions >= current.len() {
                break;
            }
            partitions = (partitions * 2).min(current.len());
        }
    }
    Ok(ReductionResult {
        items: current,
        evaluations,
    })
}

/// Reduces source fragments, compiler flags, and inputs while preserving a
/// caller-defined discrepancy.
pub fn reduce_case<E>(
    original: ReductionCandidate,
    mut interesting: impl FnMut(&ReductionCandidate) -> Result<bool, E>,
) -> Result<CaseReductionResult, E> {
    let mut current = original.clone();
    let source = try_reduce_sequence(current.source.clone(), |source| {
        let candidate = ReductionCandidate {
            source: source.to_vec(),
            ..current.clone()
        };
        interesting(&candidate)
    })?;
    current.source = source.items;

    let flags = try_reduce_sequence(current.flags.clone(), |flags| {
        let candidate = ReductionCandidate {
            flags: flags.to_vec(),
            ..current.clone()
        };
        interesting(&candidate)
    })?;
    current.flags = flags.items;

    let inputs = try_reduce_sequence(current.inputs.clone(), |inputs| {
        let candidate = ReductionCandidate {
            inputs: inputs.to_vec(),
            ..current.clone()
        };
        interesting(&candidate)
    })?;
    current.inputs = inputs.items;

    Ok(CaseReductionResult {
        original,
        minimized: current,
        evaluations: source.evaluations + flags.evaluations + inputs.evaluations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolates_required_tokens_in_stable_order() {
        let result = reduce_sequence(vec!["noise", "left", "more", "right"], |items| {
            items.contains(&"left") && items.contains(&"right")
        });
        assert_eq!(result.items, ["left", "right"]);
        assert!(result.evaluations > 1);
    }

    #[test]
    fn reduces_source_flags_and_inputs_in_declared_order() {
        let original = ReductionCandidate {
            source: vec!["noise".to_owned(), "source-trigger".to_owned()],
            flags: vec!["flag-trigger".to_owned(), "noise".to_owned()],
            inputs: vec![0, 7, 0],
        };
        let result = reduce_case(original, |candidate| {
            Ok::<_, ()>(
                candidate.source.iter().any(|item| item == "source-trigger")
                    && candidate.flags.iter().any(|item| item == "flag-trigger")
                    && candidate.inputs.contains(&7),
            )
        })
        .unwrap();
        assert_eq!(result.minimized.source, ["source-trigger"]);
        assert_eq!(result.minimized.flags, ["flag-trigger"]);
        assert_eq!(result.minimized.inputs, [7]);
    }
}
