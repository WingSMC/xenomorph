import assert from 'node:assert/strict';
import { test } from 'node:test';
import { layoutGraph } from '../src/webview/graph/layout.ts';

const columns = (ids: string[], links: [string, string][]) => {
    const layout = layoutGraph(
        ids,
        links.map(([from, to]) => ({ from, to })),
    );
    return Object.fromEntries(layout.nodes.map((node) => [node.id, node.column]));
};

test('modules that are only imported share the rightmost column', () => {
    // app imports everything; core and util are imported but import nothing.
    const result = columns(
        ['app', 'api', 'models', 'core', 'util'],
        [
            ['app', 'api'],
            ['app', 'models'],
            ['app', 'core'],
            ['app', 'util'],
            ['api', 'models'],
            ['models', 'core'],
            ['api', 'util'],
        ],
    );
    assert.deepEqual(result, { app: 0, api: 1, models: 2, core: 3, util: 3 });
});

test('every importer is left of the modules it imports, even in cycles', () => {
    const links: [string, string][] = [
        ['a', 'b'],
        ['b', 'c'],
        ['c', 'a'],
        ['c', 'd'],
    ];
    const layout = layoutGraph(
        ['a', 'b', 'c', 'd'],
        links.map(([from, to]) => ({ from, to })),
    );
    const column = new Map(layout.nodes.map((n) => [n.id, n.column]));
    const cyclic = layout.edges.filter((edge) => edge.cyclic);
    assert.equal(cyclic.length, 1);
    for (const edge of layout.edges) {
        const forward = column.get(edge.from)! < column.get(edge.to)!;
        assert.equal(forward, !edge.cyclic, `${edge.from} -> ${edge.to}`);
    }
});

test('long edges are routed around intermediate columns without overlapping nodes', () => {
    const layout = layoutGraph(
        ['a', 'b', 'c', 'd'],
        [
            { from: 'a', to: 'b' },
            { from: 'b', to: 'c' },
            { from: 'c', to: 'd' },
            { from: 'a', to: 'd' },
        ],
    );
    const long = layout.edges.find((e) => e.from === 'a' && e.to === 'd')!;
    // Start, two points per crossed column, end.
    assert.equal(long.points.length, 6);
    for (const node of layout.nodes.filter((n) => n.id === 'b' || n.id === 'c')) {
        const passing = long.points.filter((p) => Math.abs(p.x - node.x) <= 100);
        for (const point of passing) {
            assert.ok(Math.abs(point.y - node.y) >= 30, `edge crosses ${node.id}`);
        }
    }
});

test('barycenter ordering removes avoidable crossings', () => {
    // Alphabetical order would cross: a1 -> z, a2 -> b.
    const layout = layoutGraph(
        ['root', 'a1', 'a2', 'b', 'z'],
        [
            { from: 'root', to: 'a1' },
            { from: 'root', to: 'a2' },
            { from: 'a1', to: 'z' },
            { from: 'a2', to: 'b' },
        ],
    );
    const y = new Map(layout.nodes.map((n) => [n.id, n.y]));
    assert.equal(y.get('a1')! < y.get('a2')!, y.get('z')! < y.get('b')!);
});

test('handles empty graphs and ignores unknown or duplicate edges', () => {
    assert.deepEqual(layoutGraph([], []), { nodes: [], edges: [], width: 0, height: 0 });
    const layout = layoutGraph(
        ['a', 'b'],
        [
            { from: 'a', to: 'b' },
            { from: 'a', to: 'b' },
            { from: 'a', to: 'missing' },
            { from: 'a', to: 'a' },
        ],
    );
    assert.equal(layout.edges.length, 1);
});
