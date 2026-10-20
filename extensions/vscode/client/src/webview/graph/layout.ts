// Layered layout for directed graphs (Sugiyama-style), oriented left to right.
//
// 1. Cycles are broken by reversing DFS back edges.
// 2. Each node is ranked by the longest path to a sink. Nodes without outgoing
//    edges (modules that are only imported) end up in the rightmost column;
//    every importer sits strictly left of everything it imports.
// 3. Edges spanning several columns get a dummy node per crossed column so they
//    can be routed around nodes and take part in crossing reduction.
// 4. Columns are reordered with alternating barycenter sweeps, keeping the
//    order with the fewest crossings.
// 5. Vertical positions are relaxed towards the neighbours' average while
//    keeping the column order and minimum spacing.

export interface LayoutOptions {
    nodeWidth: number;
    nodeHeight: number;
    columnGap: number;
    rowGap: number;
    /** Vertical space reserved for an edge passing through a column. */
    edgeGap: number;
}

export interface Point {
    x: number;
    y: number;
}

export interface PlacedNode {
    id: string;
    column: number;
    /** Centre of the node. */
    x: number;
    y: number;
}

export interface PlacedEdge {
    from: string;
    to: string;
    /** Route from `from` to `to`, ordered along the edge direction. */
    points: Point[];
    /** The edge closes a cycle and therefore points right to left. */
    cyclic: boolean;
}

export interface GraphLayout {
    nodes: PlacedNode[];
    edges: PlacedEdge[];
    width: number;
    height: number;
}

export const defaultLayoutOptions: LayoutOptions = {
    nodeWidth: 200,
    nodeHeight: 60,
    columnGap: 120,
    rowGap: 24,
    edgeGap: 14,
};

const ORDERING_SWEEPS = 24;
const RELAXATION_SWEEPS = 12;

interface Vertex {
    /** Index into the real node list, or -1 for edge dummies. */
    node: number;
    column: number;
    up: number[];
    down: number[];
    y: number;
}

export function layoutGraph(
    ids: readonly string[],
    links: readonly { from: string; to: string }[],
    options: LayoutOptions = defaultLayoutOptions,
): GraphLayout {
    const index = new Map(ids.map((id, i) => [id, i]));
    const edges = uniqueEdges(links, index);
    const reversed = backEdges(ids.length, edges);
    const acyclic = edges.map(([from, to], i) =>
        reversed.has(i) ? [to, from] : [from, to],
    );
    const columns = assignColumns(ids.length, acyclic);

    // Real nodes keep their index; dummies are appended after them.
    const vertices: Vertex[] = columns.map((column, node) => ({
        node,
        column,
        up: [],
        down: [],
        y: 0,
    }));
    const chains = acyclic.map(([from, to]) => {
        const chain = [from];
        for (let column = columns[from] + 1; column < columns[to]; column++) {
            chain.push(vertices.length);
            vertices.push({ node: -1, column, up: [], down: [], y: 0 });
        }
        chain.push(to);
        for (let i = 1; i < chain.length; i++) {
            vertices[chain[i - 1]].down.push(chain[i]);
            vertices[chain[i]].up.push(chain[i - 1]);
        }
        return chain;
    });

    const layers = orderLayers(vertices, ids);
    const height = assignY(vertices, layers, options);

    const step = options.nodeWidth + options.columnGap;
    const half = options.nodeWidth / 2;
    const xOf = (column: number) => column * step + half;

    const nodes = ids.map((id, i) => ({
        id,
        column: columns[i],
        x: xOf(columns[i]),
        y: vertices[i].y,
    }));
    const placedEdges = edges.map(([from, to], i) => {
        const chain = chains[i];
        const points: Point[] = [];
        chain.forEach((vertex, position) => {
            const { column, y } = vertices[vertex];
            const x = xOf(column);
            if (position === 0) {
                points.push({ x: x + half, y });
            } else if (position === chain.length - 1) {
                points.push({ x: x - half, y });
            } else {
                // Dummies span the column width so edges run straight past nodes.
                points.push({ x: x - half, y }, { x: x + half, y });
            }
        });
        if (reversed.has(i)) {
            points.reverse();
        }
        return { from: ids[from], to: ids[to], points, cyclic: reversed.has(i) };
    });

    const columnCount = Math.max(0, ...columns) + 1;
    return {
        nodes,
        edges: placedEdges,
        width: ids.length ? columnCount * step - options.columnGap : 0,
        height,
    };
}

function uniqueEdges(
    links: readonly { from: string; to: string }[],
    index: Map<string, number>,
): [number, number][] {
    const seen = new Set<string>();
    const result: [number, number][] = [];
    for (const { from, to } of links) {
        const a = index.get(from);
        const b = index.get(to);
        const key = `${a}>${b}`;
        if (a === undefined || b === undefined || a === b || seen.has(key)) {
            continue;
        }
        seen.add(key);
        result.push([a, b]);
    }
    return result;
}

/** Returns the indices of edges that close a cycle in a DFS. */
function backEdges(count: number, edges: [number, number][]): Set<number> {
    const outgoing: number[][] = Array.from({ length: count }, () => []);
    edges.forEach(([from], i) => outgoing[from].push(i));

    const state = new Uint8Array(count); // 0 = new, 1 = on stack, 2 = done
    const result = new Set<number>();
    for (let root = 0; root < count; root++) {
        if (state[root] !== 0) {
            continue;
        }
        // Iterative DFS: each frame is [node, next outgoing edge position].
        const stack: [number, number][] = [[root, 0]];
        state[root] = 1;
        while (stack.length) {
            const frame = stack[stack.length - 1];
            const [node, position] = frame;
            if (position === outgoing[node].length) {
                state[node] = 2;
                stack.pop();
                continue;
            }
            frame[1]++;
            const edge = outgoing[node][position];
            const next = edges[edge][1];
            if (state[next] === 1) {
                result.add(edge);
            } else if (state[next] === 0) {
                state[next] = 1;
                stack.push([next, 0]);
            }
        }
    }
    return result;
}

/**
 * Column of each node: sinks go to the last column and every other node is
 * placed one column left of its leftmost dependency. Nodes without any edges
 * are put in the first column next to the other roots.
 */
function assignColumns(count: number, edges: number[][]): number[] {
    const outgoing: number[][] = Array.from({ length: count }, () => []);
    const incoming = new Array<number>(count).fill(0);
    for (const [from, to] of edges) {
        outgoing[from].push(to);
        incoming[to]++;
    }

    // Distance to the farthest sink, computed in reverse topological order.
    const rank = new Array<number>(count).fill(0);
    const pending = outgoing.map((targets) => targets.length);
    const incomingOf: number[][] = Array.from({ length: count }, () => []);
    for (const [from, to] of edges) {
        incomingOf[to].push(from);
    }
    const queue = pending.flatMap((n, node) => (n === 0 ? [node] : []));
    while (queue.length) {
        const node = queue.shift()!;
        for (const source of incomingOf[node]) {
            rank[source] = Math.max(rank[source], rank[node] + 1);
            if (--pending[source] === 0) {
                queue.push(source);
            }
        }
    }

    const maxRank = Math.max(0, ...rank);
    return rank.map((r, node) =>
        outgoing[node].length === 0 && incoming[node] === 0 ? 0 : maxRank - r,
    );
}

function orderLayers(vertices: Vertex[], ids: readonly string[]): number[][] {
    const layers: number[][] = [];
    vertices.forEach((vertex, i) => (layers[vertex.column] ??= []).push(i));
    for (let c = 0; c < layers.length; c++) {
        layers[c] ??= [];
    }
    // Sorting by path keeps sibling modules together as a starting point.
    const name = (v: number) => (vertices[v].node >= 0 ? ids[vertices[v].node] : '');
    for (const layer of layers) {
        layer.sort((a, b) => name(a).localeCompare(name(b)));
    }

    let best = layers.map((layer) => [...layer]);
    let bestCrossings = crossings(vertices, layers);
    for (let sweep = 0; sweep < ORDERING_SWEEPS && bestCrossings > 0; sweep++) {
        const forward = sweep % 2 === 0;
        const range = forward
            ? layers.keys()
            : [...layers.keys()].reverse().values();
        for (const c of range) {
            const reference = forward ? layers[c - 1] : layers[c + 1];
            if (!reference) {
                continue;
            }
            const position = new Map(reference.map((v, i) => [v, i]));
            const own = new Map(layers[c].map((v, i) => [v, i]));
            const key = new Map(
                layers[c].map((v) => {
                    const neighbours = forward ? vertices[v].up : vertices[v].down;
                    if (neighbours.length === 0) {
                        // Unconnected on this side: keep its relative position.
                        return [v, (own.get(v)! / layers[c].length) * reference.length];
                    }
                    const sum = neighbours.reduce((s, n) => s + position.get(n)!, 0);
                    return [v, sum / neighbours.length];
                }),
            );
            layers[c].sort((a, b) => key.get(a)! - key.get(b)!);
        }
        const count = crossings(vertices, layers);
        if (count < bestCrossings) {
            bestCrossings = count;
            best = layers.map((layer) => [...layer]);
        }
    }
    return best;
}

function crossings(vertices: Vertex[], layers: number[][]): number {
    let total = 0;
    for (let c = 0; c + 1 < layers.length; c++) {
        const position = new Map(layers[c + 1].map((v, i) => [v, i]));
        const segments: [number, number][] = [];
        layers[c].forEach((v, i) => {
            for (const target of vertices[v].down) {
                segments.push([i, position.get(target)!]);
            }
        });
        for (let a = 0; a < segments.length; a++) {
            for (let b = a + 1; b < segments.length; b++) {
                const [a1, a2] = segments[a];
                const [b1, b2] = segments[b];
                if ((a1 - b1) * (a2 - b2) < 0) {
                    total++;
                }
            }
        }
    }
    return total;
}

/** Sets `vertex.y` for every vertex and returns the total height. */
function assignY(
    vertices: Vertex[],
    layers: number[][],
    options: LayoutOptions,
): number {
    const extent = (v: number) =>
        vertices[v].node >= 0 ? options.nodeHeight : options.edgeGap;
    // Minimum distance between the centres of two vertices stacked in a column.
    const gap = (a: number, b: number) =>
        (extent(a) + extent(b)) / 2 +
        (vertices[a].node >= 0 && vertices[b].node >= 0 ? options.rowGap : 0);

    const tallest = Math.max(
        0,
        ...layers.map((layer) =>
            layer.reduce((sum, v, i) => sum + (i ? gap(layer[i - 1], v) : 0), 0),
        ),
    );
    for (const layer of layers) {
        let y = 0;
        layer.forEach((v, i) => {
            y += i ? gap(layer[i - 1], v) : 0;
            vertices[v].y = y;
        });
        const offset = (tallest - y) / 2;
        layer.forEach((v) => (vertices[v].y += offset));
    }

    for (let sweep = 0; sweep < RELAXATION_SWEEPS; sweep++) {
        const order = sweep % 2 === 0 ? layers : [...layers].reverse();
        for (const layer of order) {
            const desired = layer.map((v) => {
                const { up, down, y } = vertices[v];
                const neighbours = [...up, ...down];
                return neighbours.length
                    ? neighbours.reduce((s, n) => s + vertices[n].y, 0) / neighbours.length
                    : y;
            });
            // Closest positions to `desired` that respect order and spacing:
            // the mean of a top-down and a bottom-up packing.
            const down = [...desired];
            for (let i = 1; i < layer.length; i++) {
                down[i] = Math.max(down[i], down[i - 1] + gap(layer[i - 1], layer[i]));
            }
            const up = [...desired];
            for (let i = layer.length - 2; i >= 0; i--) {
                up[i] = Math.min(up[i], up[i + 1] - gap(layer[i], layer[i + 1]));
            }
            layer.forEach((v, i) => (vertices[v].y = (down[i] + up[i]) / 2));
        }
    }

    let top = Infinity;
    let bottom = -Infinity;
    for (const layer of layers) {
        for (const v of layer) {
            top = Math.min(top, vertices[v].y - extent(v) / 2);
            bottom = Math.max(bottom, vertices[v].y + extent(v) / 2);
        }
    }
    if (top === Infinity) {
        return 0;
    }
    for (const vertex of vertices) {
        vertex.y -= top;
    }
    return bottom - top;
}
