//! Minimum-cost bipartite matching via the Hungarian (Kuhn-Munkres) algorithm.
//!
//! Given a cost matrix `cost[i][j]` for `rows` row-agents and `cols`
//! column-agents, returns a Vec of `(row, col)` pairs representing the
//! optimal assignment that minimizes total cost. Handles rectangular
//! matrices by padding to square with `pad_cost`.
//!
//! Complexity: O(N^3) where N = max(rows, cols).

/// Compute the minimum-cost assignment for a (possibly rectangular) cost matrix.
///
/// `pad_cost` fills dummy entries when the matrix is padded to square.
/// It should be larger than any real cost in the matrix so that dummy
/// assignments are never preferred over real ones.
pub fn min_cost_assignment(
    cost: &[Vec<u32>],
    rows: usize,
    cols: usize,
    pad_cost: u32,
) -> Vec<(usize, usize)> {
    if rows == 0 || cols == 0 {
        return Vec::new();
    }

    let n = rows.max(cols);
    let big = pad_cost as i64;

    let mut c = vec![vec![0i64; n]; n];
    for i in 0..n {
        for j in 0..n {
            c[i][j] = if i < rows && j < cols {
                cost[i][j] as i64
            } else {
                big
            };
        }
    }

    let mut u = vec![0i64; n + 1];
    let mut v = vec![0i64; n + 1];
    let mut p = vec![0usize; n + 1];
    let mut way = vec![0usize; n + 1];

    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut min_v = vec![i64::MAX; n + 1];
        let mut used = vec![false; n + 1];

        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = i64::MAX;
            let mut j1 = 0usize;

            for j in 1..=n {
                if used[j] {
                    continue;
                }
                let cur = c[i0 - 1][j - 1] - u[i0] - v[j];
                if cur < min_v[j] {
                    min_v[j] = cur;
                    way[j] = j0;
                }
                if min_v[j] < delta {
                    delta = min_v[j];
                    j1 = j;
                }
            }

            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    min_v[j] -= delta;
                }
            }

            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }

        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }

    let mut result = Vec::with_capacity(rows.min(cols));
    for j in 1..=n {
        if p[j] != 0 && p[j] <= rows && j <= cols {
            result.push((p[j] - 1, j - 1));
        }
    }
    result
}
