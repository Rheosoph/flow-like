use std::collections::BTreeMap;

/// Result of a gated linear assignment.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// `(row, col)` pairs, sorted by row
    pub matches: Vec<(usize, usize)>,
    pub unmatched_rows: Vec<usize>,
    pub unmatched_cols: Vec<usize>,
}

/// Minimum-cost assignment of `rows` × `cols` where leaving an item unmatched is allowed.
///
/// Mirrors `lap.lapjv(cost, extend_cost=True, cost_limit=limit)` as used by ByteTrack: every row
/// and column may instead take a dummy partner at `limit / 2`, so a pair is only matched when it
/// is cheaper than leaving both sides unmatched. Pairs costing more than `limit`, and non-finite
/// costs, are never matched.
pub fn linear_assignment(
    rows: usize,
    cols: usize,
    limit: f32,
    cost: impl Fn(usize, usize) -> f32,
) -> Assignment {
    if rows == 0 || cols == 0 {
        return Assignment {
            matches: Vec::new(),
            unmatched_rows: (0..rows).collect(),
            unmatched_cols: (0..cols).collect(),
        };
    }

    let limit = f64::from(limit.max(0.0));
    let admissible: Vec<Option<f64>> = (0..rows)
        .flat_map(|r| (0..cols).map(move |c| (r, c)))
        .map(|(r, c)| {
            let value = f64::from(cost(r, c));
            (value.is_finite() && value <= limit).then_some(value)
        })
        .collect();

    // Leaving an item unmatched costs the same wherever it is, so the problem splits exactly into
    // the connected components of admissible pairs; each is solved on its own to keep the O(n³)
    // Hungarian small when many detections are far apart.
    let mut parent: Vec<usize> = (0..rows + cols).collect();
    for r in 0..rows {
        for c in 0..cols {
            if admissible[r * cols + c].is_some() {
                let (a, b) = (find_root(&mut parent, r), find_root(&mut parent, rows + c));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    let mut components: BTreeMap<usize, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for r in 0..rows {
        components
            .entry(find_root(&mut parent, r))
            .or_default()
            .0
            .push(r);
    }
    for c in 0..cols {
        components
            .entry(find_root(&mut parent, rows + c))
            .or_default()
            .1
            .push(c);
    }

    let mut matches = Vec::new();
    for (component_rows, component_cols) in components.values() {
        if !component_rows.is_empty() && !component_cols.is_empty() {
            matches.extend(solve_component(
                component_rows,
                component_cols,
                |r, c| admissible[r * cols + c],
                limit,
            ));
        }
    }
    matches.sort_unstable();

    let mut matched_rows = vec![false; rows];
    let mut matched_cols = vec![false; cols];
    for &(r, c) in &matches {
        matched_rows[r] = true;
        matched_cols[c] = true;
    }
    Assignment {
        matches,
        unmatched_rows: (0..rows).filter(|&r| !matched_rows[r]).collect(),
        unmatched_cols: (0..cols).filter(|&c| !matched_cols[c]).collect(),
    }
}

fn find_root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

fn solve_component(
    rows: &[usize],
    cols: &[usize],
    admissible: impl Fn(usize, usize) -> Option<f64>,
    limit: f64,
) -> Vec<(usize, usize)> {
    let forbidden = (limit + 1.0) * 1e4;
    let (n_rows, n_cols) = (rows.len(), cols.len());
    let extended = |r: usize, c: usize| -> f64 {
        match (r < n_rows, c < n_cols) {
            (true, true) => admissible(rows[r], cols[c]).unwrap_or(forbidden),
            (false, false) => 0.0,
            _ => limit / 2.0,
        }
    };
    hungarian(n_rows + n_cols, extended)
        .into_iter()
        .take(n_rows)
        .enumerate()
        .filter(|&(r, c)| c < n_cols && admissible(rows[r], cols[c]).is_some())
        .map(|(r, c)| (rows[r], cols[c]))
        .collect()
}

/// Square Hungarian algorithm (Kuhn–Munkres with potentials), O(n³). Returns the column assigned
/// to each row.
fn hungarian(n: usize, cost: impl Fn(usize, usize) -> f64) -> Vec<usize> {
    let mut u = vec![0.0f64; n + 1];
    let mut v = vec![0.0f64; n + 1];
    let mut row_of_col = vec![0usize; n + 1];
    let mut way = vec![0usize; n + 1];

    for row in 1..=n {
        row_of_col[0] = row;
        let mut col0 = 0usize;
        let mut min_slack = vec![f64::INFINITY; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[col0] = true;
            let row0 = row_of_col[col0];
            let mut delta = f64::INFINITY;
            let mut col1 = 0usize;
            for col in 1..=n {
                if used[col] {
                    continue;
                }
                let reduced = cost(row0 - 1, col - 1) - u[row0] - v[col];
                if reduced < min_slack[col] {
                    min_slack[col] = reduced;
                    way[col] = col0;
                }
                if min_slack[col] < delta {
                    delta = min_slack[col];
                    col1 = col;
                }
            }
            for col in 0..=n {
                if used[col] {
                    u[row_of_col[col]] += delta;
                    v[col] -= delta;
                } else {
                    min_slack[col] -= delta;
                }
            }
            col0 = col1;
            if row_of_col[col0] == 0 {
                break;
            }
        }
        loop {
            let col1 = way[col0];
            row_of_col[col0] = row_of_col[col1];
            col0 = col1;
            if col0 == 0 {
                break;
            }
        }
    }

    let mut col_for_row = vec![0usize; n];
    for col in 1..=n {
        if row_of_col[col] != 0 {
            col_for_row[row_of_col[col] - 1] = col - 1;
        }
    }
    col_for_row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solve(matrix: &[Vec<f32>], limit: f32) -> Assignment {
        let cols = matrix.first().map_or(0, Vec::len);
        linear_assignment(matrix.len(), cols, limit, |r, c| matrix[r][c])
    }

    fn brute_force(matrix: &[Vec<f32>], limit: f32) -> f32 {
        fn go(matrix: &[Vec<f32>], limit: f32, row: usize, used: &mut Vec<bool>) -> f32 {
            if row == matrix.len() {
                let free_cols = used.iter().filter(|u| !**u).count();
                return free_cols as f32 * limit / 2.0;
            }
            let mut best = limit / 2.0 + go(matrix, limit, row + 1, used);
            for col in 0..used.len() {
                let value = matrix[row][col];
                if !used[col] && value.is_finite() && value <= limit {
                    used[col] = true;
                    best = best.min(value + go(matrix, limit, row + 1, used));
                    used[col] = false;
                }
            }
            best
        }
        let cols = matrix.first().map_or(0, Vec::len);
        go(matrix, limit, 0, &mut vec![false; cols])
    }

    fn total(matrix: &[Vec<f32>], limit: f32, assignment: &Assignment) -> f32 {
        let matched: f32 = assignment.matches.iter().map(|&(r, c)| matrix[r][c]).sum();
        let unmatched = assignment.unmatched_rows.len() + assignment.unmatched_cols.len();
        matched + unmatched as f32 * limit / 2.0
    }

    #[test]
    fn empty_sides_are_unmatched() {
        let a = linear_assignment(0, 3, 0.5, |_, _| 0.0);
        assert!(a.matches.is_empty());
        assert_eq!(a.unmatched_cols, vec![0, 1, 2]);
        let a = linear_assignment(2, 0, 0.5, |_, _| 0.0);
        assert_eq!(a.unmatched_rows, vec![0, 1]);
    }

    #[test]
    fn picks_global_optimum_over_greedy() {
        let m = vec![vec![0.1, 0.2], vec![0.15, 0.9]];
        let a = solve(&m, 1.0);
        assert_eq!(a.matches, vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn respects_the_cost_limit() {
        let m = vec![vec![0.9, 0.95], vec![0.3, 0.99]];
        let a = solve(&m, 0.8);
        assert_eq!(a.matches, vec![(1, 0)]);
        assert_eq!(a.unmatched_rows, vec![0]);
        assert_eq!(a.unmatched_cols, vec![1]);
    }

    #[test]
    fn never_matches_non_finite_costs() {
        let m = vec![vec![f32::INFINITY, f32::NAN], vec![0.2, f32::INFINITY]];
        let a = solve(&m, 1.0);
        assert_eq!(a.matches, vec![(1, 0)]);
    }

    #[test]
    fn rectangular_matrices() {
        let m = vec![vec![0.5, 0.1, 0.7, 0.3]];
        assert_eq!(solve(&m, 1.0).matches, vec![(0, 1)]);
        let m = vec![vec![0.5], vec![0.1], vec![0.7]];
        let a = solve(&m, 1.0);
        assert_eq!(a.matches, vec![(1, 0)]);
        assert_eq!(a.unmatched_rows, vec![0, 2]);
    }

    #[test]
    fn large_sparse_frames_solve_per_component() {
        let n = 600;
        let a = linear_assignment(n, n, 0.8, |r, c| {
            if r == c {
                0.1
            } else if r / 3 == c / 3 {
                0.5
            } else {
                f32::INFINITY
            }
        });
        assert_eq!(a.matches.len(), n);
        assert!(a.matches.iter().all(|&(r, c)| r == c));
    }

    #[test]
    fn matches_brute_force_on_generated_matrices() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 10_000) as f32 / 10_000.0
        };
        for case in 0..200 {
            let rows = 1 + case % 5;
            let cols = 1 + (case / 5) % 5;
            let limit = 0.2 + next() * 0.8;
            let sparse = case % 2 == 1;
            let m: Vec<Vec<f32>> = (0..rows)
                .map(|_| {
                    (0..cols)
                        .map(|_| {
                            let value = next();
                            if sparse && next() < 0.5 {
                                f32::INFINITY
                            } else {
                                value
                            }
                        })
                        .collect()
                })
                .collect();
            let a = solve(&m, limit);
            let expected = brute_force(&m, limit);
            assert!(
                (total(&m, limit, &a) - expected).abs() < 1e-4,
                "case {case}: {m:?} limit {limit}"
            );
            assert_eq!(a.matches.len() + a.unmatched_rows.len(), rows);
            assert_eq!(a.matches.len() + a.unmatched_cols.len(), cols);
        }
    }
}
