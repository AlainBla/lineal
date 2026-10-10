//! Initiative-Hierarchie als gerichteter azyklischer Graph (docs/04 §4.3). Reine Funktionen auf
//! Kantenlisten `(parent, child)`; keine I/O.

use lineal_domain::ids::InitiativeId;
use std::collections::{HashMap, HashSet};

/// Verstoß gegen die DAG-Regeln.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// Zyklus (auch Selbstreferenz): Pfad `child → … → parent → child`.
    Cycle(Vec<InitiativeId>),
    /// Längster Pfad überschreitet das Workspace-Limit.
    MaxDepth { depth: usize, limit: usize },
}

type Edges = [(InitiativeId, InitiativeId)];

fn children_map(edges: &Edges) -> HashMap<InitiativeId, Vec<InitiativeId>> {
    let mut m: HashMap<InitiativeId, Vec<InitiativeId>> = HashMap::new();
    for (p, c) in edges {
        m.entry(*p).or_default().push(*c);
    }
    m
}

fn parents_map(edges: &Edges) -> HashMap<InitiativeId, Vec<InitiativeId>> {
    let mut m: HashMap<InitiativeId, Vec<InitiativeId>> = HashMap::new();
    for (p, c) in edges {
        m.entry(*c).or_default().push(*p);
    }
    m
}

/// Alle Nachfahren (ohne `root`), ohne Duplikate, in Tiefensuche-Reihenfolge.
pub fn descendants(edges: &Edges, root: InitiativeId) -> Vec<InitiativeId> {
    let kids = children_map(edges);
    let mut seen: HashSet<InitiativeId> = HashSet::new();
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        for c in kids.get(&n).into_iter().flatten() {
            if *c != root && seen.insert(*c) {
                out.push(*c);
                stack.push(*c);
            }
        }
    }
    out
}

/// Direkte Eltern.
pub fn parents_of(edges: &Edges, node: InitiativeId) -> Vec<InitiativeId> {
    parents_map(edges).remove(&node).unwrap_or_default()
}

/// Direkte Kinder.
pub fn children_of(edges: &Edges, node: InitiativeId) -> Vec<InitiativeId> {
    children_map(edges).remove(&node).unwrap_or_default()
}

fn longest(
    node: InitiativeId,
    next: &HashMap<InitiativeId, Vec<InitiativeId>>,
    memo: &mut HashMap<InitiativeId, usize>,
    visiting: &mut HashSet<InitiativeId>,
) -> usize {
    if let Some(v) = memo.get(&node) {
        return *v;
    }
    if !visiting.insert(node) {
        return 0; // Schutz gegen (ungültige) Zyklen im Bestand
    }
    let best = next
        .get(&node)
        .into_iter()
        .flatten()
        .map(|n| longest(*n, next, memo, visiting))
        .max()
        .unwrap_or(0);
    visiting.remove(&node);
    memo.insert(node, 1 + best);
    1 + best
}

/// Länge des längsten Pfads von `node` zu einer Wurzel (Wurzel = 1).
pub fn longest_path_to_root(edges: &Edges, node: InitiativeId) -> usize {
    longest(node, &parents_map(edges), &mut HashMap::new(), &mut HashSet::new())
}

/// Länge des längsten Pfads von `node` zu einem Blatt (Blatt = 1).
pub fn longest_path_to_leaf(edges: &Edges, node: InitiativeId) -> usize {
    longest(node, &children_map(edges), &mut HashMap::new(), &mut HashSet::new())
}

/// Findet einen Pfad `from → … → to` entlang der Kind-Kanten.
fn find_path(edges: &Edges, from: InitiativeId, to: InitiativeId) -> Option<Vec<InitiativeId>> {
    let kids = children_map(edges);
    let mut prev: HashMap<InitiativeId, InitiativeId> = HashMap::new();
    let mut seen: HashSet<InitiativeId> = HashSet::from([from]);
    let mut stack = vec![from];
    while let Some(n) = stack.pop() {
        if n == to {
            let mut path = vec![to];
            let mut cur = to;
            while let Some(p) = prev.get(&cur) {
                path.push(*p);
                cur = *p;
            }
            path.reverse();
            return Some(path);
        }
        for c in kids.get(&n).into_iter().flatten() {
            if seen.insert(*c) {
                prev.insert(*c, n);
                stack.push(*c);
            }
        }
    }
    None
}

/// Prüft, ob `parent` als Elternteil von `child` hinzugefügt werden darf (docs/04 §4.3 `addParent`).
/// `limit` ist die maximale Tiefe (Wurzel = Ebene 1). Eine bereits vorhandene Kante ist zulässig (No-op).
pub fn check_add_parent(
    edges: &Edges,
    child: InitiativeId,
    parent: InitiativeId,
    limit: usize,
) -> Result<(), GraphError> {
    if child == parent {
        return Err(GraphError::Cycle(vec![child, child]));
    }
    if edges.contains(&(parent, child)) {
        return Ok(());
    }
    if let Some(mut path) = find_path(edges, child, parent) {
        path.push(child);
        return Err(GraphError::Cycle(path));
    }
    let above = longest_path_to_root(edges, parent);
    let below = longest_path_to_leaf(edges, child);
    let depth = above + below;
    if depth > limit {
        return Err(GraphError::MaxDepth { depth, limit });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<InitiativeId> {
        (0..n).map(|_| InitiativeId::new_v7()).collect()
    }

    #[test]
    fn rejects_self_and_cycles_with_path() {
        let i = ids(3);
        assert_eq!(
            check_add_parent(&[], i[0], i[0], 5),
            Err(GraphError::Cycle(vec![i[0], i[0]]))
        );
        // I0 -> I1 -> I2; I2 als Elternteil von I0 wäre ein Zyklus
        let edges = [(i[0], i[1]), (i[1], i[2])];
        match check_add_parent(&edges, i[0], i[2], 5) {
            Err(GraphError::Cycle(p)) => assert_eq!(p, vec![i[0], i[1], i[2], i[0]]),
            other => panic!("expected cycle, got {other:?}"),
        }
    }

    #[test]
    fn allows_diamonds_and_existing_edges() {
        let i = ids(4);
        // A -> B, A -> C; D unter B und C (mehrere Eltern)
        let edges = [(i[0], i[1]), (i[0], i[2]), (i[1], i[3])];
        assert_eq!(check_add_parent(&edges, i[3], i[2], 5), Ok(()));
        assert_eq!(check_add_parent(&edges, i[3], i[1], 5), Ok(()));
    }

    #[test]
    fn depth_is_longest_path_through_both_sides() {
        let i = ids(6);
        // Kette 0->1->2 (Tiefe 3) und Kette 3->4->5 (Tiefe 3): 2 unter 3 hängen ergibt 6 Ebenen
        let edges = [(i[0], i[1]), (i[1], i[2]), (i[3], i[4]), (i[4], i[5])];
        assert_eq!(longest_path_to_root(&edges, i[2]), 3);
        assert_eq!(longest_path_to_leaf(&edges, i[3]), 3);
        assert_eq!(
            check_add_parent(&edges, i[3], i[2], 5),
            Err(GraphError::MaxDepth { depth: 6, limit: 5 })
        );
        assert_eq!(check_add_parent(&edges, i[3], i[2], 6), Ok(()));
    }

    #[test]
    fn descendants_are_deduplicated() {
        let i = ids(4);
        let edges = [(i[0], i[1]), (i[0], i[2]), (i[1], i[3]), (i[2], i[3])];
        let d = descendants(&edges, i[0]);
        assert_eq!(d.len(), 3);
        assert!(d.contains(&i[3]));
        assert_eq!(parents_of(&edges, i[3]).len(), 2);
        assert_eq!(children_of(&edges, i[0]).len(), 2);
    }
}
