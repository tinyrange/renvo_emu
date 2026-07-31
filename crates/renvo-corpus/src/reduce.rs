use serde::{Deserialize, Serialize};

/// Result of deterministic delta debugging over an ordered sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReductionResult<T> {
    /// Smallest sequence found by the deterministic partition strategy.
    pub items: Vec<T>,
    /// Number of predicate evaluations.
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
}
