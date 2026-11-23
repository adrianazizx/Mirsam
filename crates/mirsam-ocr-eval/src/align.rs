/// Levenshtein distance between two sequences (unit costs), in O(min(n, m)) memory.
pub fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut row: Vec<usize> = (0..=short.len()).collect();
    for (i, x) in long.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in short.iter().enumerate() {
            let substitute = diagonal + usize::from(x != y);
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(diagonal + 1).min(row[j] + 1);
        }
    }
    row[short.len()]
}

/// One step of an optimal alignment between a reference and a hypothesis sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// `reference[r] == hypothesis[h]`.
    Match { r: usize, h: usize },
    /// `reference[r]` was replaced by `hypothesis[h]`.
    Substitute { r: usize, h: usize },
    /// `reference[r]` has no counterpart.
    Delete { r: usize },
    /// `hypothesis[h]` has no counterpart.
    Insert { h: usize },
}

/// An optimal (minimum edit distance) alignment, in sequence order.
pub(crate) fn align<T: PartialEq>(reference: &[T], hypothesis: &[T]) -> Vec<Op> {
    let (n, m) = (reference.len(), hypothesis.len());
    let width = m + 1;
    // Full table: backtracking needs every row. Word sequences of a page are
    // small enough (thousands) that n * m u32 cells is fine.
    let mut table = vec![0u32; (n + 1) * width];
    for (j, cell) in table.iter_mut().enumerate().take(width) {
        *cell = j as u32;
    }
    for i in 1..=n {
        table[i * width] = i as u32;
        for j in 1..=m {
            let substitute =
                table[(i - 1) * width + j - 1] + u32::from(reference[i - 1] != hypothesis[j - 1]);
            let delete = table[(i - 1) * width + j] + 1;
            let insert = table[i * width + j - 1] + 1;
            table[i * width + j] = substitute.min(delete).min(insert);
        }
    }

    let mut ops = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        let here = table[i * width + j];
        if i > 0 && j > 0 {
            let equal = reference[i - 1] == hypothesis[j - 1];
            if here == table[(i - 1) * width + j - 1] + u32::from(!equal) {
                ops.push(if equal {
                    Op::Match { r: i - 1, h: j - 1 }
                } else {
                    Op::Substitute { r: i - 1, h: j - 1 }
                });
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && here == table[(i - 1) * width + j] + 1 {
            ops.push(Op::Delete { r: i - 1 });
            i -= 1;
        } else {
            ops.push(Op::Insert { h: j - 1 });
            j -= 1;
        }
    }
    ops.reverse();
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn distance_matches_textbook_cases() {
        assert_eq!(edit_distance(&chars("kitten"), &chars("sitting")), 3);
        assert_eq!(edit_distance(&chars(""), &chars("abc")), 3);
        assert_eq!(edit_distance(&chars("abc"), &chars("abc")), 0);
        assert_eq!(edit_distance(&chars("كتاب"), &chars("كتب")), 1);
    }

    #[test]
    fn alignment_cost_equals_distance() {
        let (a, b) = (chars("the quick brown fox"), chars("teh quick brwn foxes"));
        let ops = align(&a, &b);
        let cost = ops.iter().filter(|op| !matches!(op, Op::Match { .. })).count();
        assert_eq!(cost, edit_distance(&a, &b));
        // Every reference and hypothesis position is covered exactly once.
        let refs = ops.iter().filter(|op| !matches!(op, Op::Insert { .. })).count();
        let hyps = ops.iter().filter(|op| !matches!(op, Op::Delete { .. })).count();
        assert_eq!((refs, hyps), (a.len(), b.len()));
    }
}
