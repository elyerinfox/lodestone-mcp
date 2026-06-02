//! Optimization / operations research skill — TSP 2-opt, max-flow on a
//! capacitated graph, shortest path. Wire-thin wrappers over `pathfinding`.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TspArgs {
    /// Pairwise distance matrix (symmetric, zero diagonal).
    distances: Vec<Vec<f64>>,
}

pub struct OptTsp2opt;
impl Skill for OptTsp2opt {
    fn name(&self) -> &'static str {
        "opt_tsp_2opt"
    }
    fn description(&self) -> &'static str {
        "Solve a Traveling Salesperson tour heuristically via 2-opt local \
        search seeded with nearest-neighbor. Returns the visit order and \
        the total tour length. Practical up to ~500 nodes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TspArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TspArgs>()?;
            let n = a.distances.len();
            if n < 2 {
                return Err(invalid("need ≥ 2 nodes"));
            }
            for r in &a.distances {
                if r.len() != n {
                    return Err(invalid("distance matrix must be square"));
                }
            }
            // Nearest-neighbour start.
            let mut tour: Vec<usize> = Vec::with_capacity(n);
            let mut visited = vec![false; n];
            tour.push(0);
            visited[0] = true;
            for _ in 1..n {
                let last = *tour.last().unwrap();
                let mut best = usize::MAX;
                let mut best_d = f64::INFINITY;
                for (j, &v) in visited.iter().enumerate() {
                    if !v && a.distances[last][j] < best_d {
                        best_d = a.distances[last][j];
                        best = j;
                    }
                }
                tour.push(best);
                visited[best] = true;
            }
            // 2-opt improvement.
            let mut improved = true;
            while improved {
                improved = false;
                for i in 1..n - 1 {
                    for j in i + 1..n {
                        let a_idx = tour[i - 1];
                        let b_idx = tour[i];
                        let c_idx = tour[j];
                        let d_idx = tour[(j + 1) % n];
                        let delta = (a.distances[a_idx][c_idx] + a.distances[b_idx][d_idx])
                            - (a.distances[a_idx][b_idx] + a.distances[c_idx][d_idx]);
                        if delta < -1e-12 {
                            tour[i..=j].reverse();
                            improved = true;
                        }
                    }
                }
            }
            let total: f64 = (0..n)
                .map(|i| a.distances[tour[i]][tour[(i + 1) % n]])
                .sum();
            Ok(text_result(
                json!({ "tour": tour, "total_distance": total }).to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ShortestPathArgs {
    /// Directed edges as [from, to, weight].
    edges: Vec<(usize, usize, f64)>,
    start: usize,
    goal: usize,
}

pub struct OptShortestPath;
impl Skill for OptShortestPath {
    fn name(&self) -> &'static str {
        "opt_shortest_path"
    }
    fn description(&self) -> &'static str {
        "Dijkstra shortest path on a directed weighted graph (non-negative \
        weights only). Returns the path and total cost."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ShortestPathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use std::collections::HashMap;
            let (_s, a) = ctx.parse::<ShortestPathArgs>()?;
            let mut adj: HashMap<usize, Vec<(usize, i64)>> = HashMap::new();
            for (from, to, w) in &a.edges {
                if *w < 0.0 {
                    return Err(invalid("dijkstra: negative weights not allowed"));
                }
                adj.entry(*from)
                    .or_default()
                    .push((*to, (w * 1_000_000.0).round() as i64));
            }
            let result = pathfinding::directed::dijkstra::dijkstra(
                &a.start,
                |n| adj.get(n).cloned().unwrap_or_default(),
                |n| *n == a.goal,
            );
            match result {
                Some((path, cost)) => Ok(text_result(
                    json!({ "path": path, "total_cost": cost as f64 / 1_000_000.0 }).to_string(),
                )),
                None => Err(invalid("no path from start to goal")),
            }
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(OptTsp2opt), Box::new(OptShortestPath)]
}
