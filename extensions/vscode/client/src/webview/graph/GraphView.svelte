<script lang="ts">
    import { curveBumpX, line } from 'd3-shape';
    import type { ModuleGraph, ModuleNode } from '../../shared/protocol';
    import PanZoom from '../lib/PanZoom.svelte';
    import Shell from '../lib/Shell.svelte';
    import { post } from '../lib/vscode';
    import { defaultLayoutOptions, layoutGraph, type Point } from './layout';

    let { graph }: { graph: ModuleGraph } = $props();

    const { nodeWidth: W, nodeHeight: H } = defaultLayoutOptions;
    const path = line<Point>((p) => p.x, (p) => p.y).curve(curveBumpX);

    const modules = $derived(new Map(graph.modules.map((m) => [m.path, m])));
    const layout = $derived(
        layoutGraph(
            graph.modules.map((m) => m.path),
            graph.imports.map((edge) => ({ from: edge.importer, to: edge.imported })),
        ),
    );

    let canvas: PanZoom;
    let selected = $state<string>();
    let hovered = $state<string>();
    const focus = $derived(hovered ?? selected);

    const importsOf = (id: string) => graph.imports.filter((e) => e.importer === id).map((e) => e.imported);
    const importersOf = (id: string) => graph.imports.filter((e) => e.imported === id).map((e) => e.importer);

    const related = $derived(
        focus ? new Set([focus, ...importsOf(focus), ...importersOf(focus)]) : undefined,
    );
    const current = $derived(selected ? modules.get(selected) : undefined);

    function choose(module: ModuleNode): void {
        selected = module.path;
        post({ type: 'open', path: module.absolutePath });
    }

    function severity({ diagnostics }: ModuleNode): string {
        return diagnostics.errors ? 'error' : diagnostics.warnings ? 'warning' : '';
    }

    function splitPath(id: string): [string, string] {
        const slash = id.lastIndexOf('/');
        return [id.slice(0, slash + 1), id.slice(slash + 1)];
    }

    function plural(count: number, noun: string): string {
        return `${count} ${noun}${count === 1 ? '' : 's'}`;
    }

    function details({ declarations, diagnostics }: ModuleNode): string {
        const parts = [plural(declarations, 'declaration')];
        if (diagnostics.errors) parts.push(plural(diagnostics.errors, 'error'));
        if (diagnostics.warnings) parts.push(plural(diagnostics.warnings, 'warning'));
        return parts.join(' · ');
    }
</script>

<Shell
    title="Module graph"
    subtitle="{plural(graph.moduleCount, 'module')} · {plural(graph.importCount, 'import')} · entry {graph.entry}"
>
    {#snippet toolbar()}
        <button title="Zoom out" onclick={() => canvas.zoomBy(0.8)}>−</button>
        <button title="Fit to view" onclick={() => canvas.fit()}>Fit</button>
        <button title="Zoom in" onclick={() => canvas.zoomBy(1.25)}>+</button>
    {/snippet}

    {#snippet sidebar()}
        {#if current}
            <h2>Module</h2>
            <p class="name">{current.path}{current.entry ? ' (entry)' : ''}</p>
            <p class="muted mono file">{current.absolutePath}</p>
            <p>
                {plural(current.declarations, 'declaration')} ·
                {plural(current.diagnostics.errors, 'error')} ·
                {plural(current.diagnostics.warnings, 'warning')}
            </p>
            {#each [['Imports', importsOf(current.path)], ['Imported by', importersOf(current.path)]] as const as [heading, ids] (heading)}
                <h2>{heading} ({ids.length})</h2>
                <ul>
                    {#each ids as id (id)}
                        <li>
                            <button class="link" onclick={() => choose(modules.get(id)!)}>{id}</button>
                        </li>
                    {/each}
                </ul>
            {/each}
        {:else}
            <p class="muted">
                Click a module to open it. Arrows point from a module to the modules it imports, so
                modules that are only imported sit in the rightmost column.
            </p>
        {/if}
    {/snippet}

    <PanZoom bind:this={canvas} width={layout.width} height={layout.height} label="Module dependency graph">
        {#snippet defs()}
            <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto">
                <path d="M0,0 L10,5 L0,10 z" />
            </marker>
        {/snippet}

        {#each layout.edges as edge (`${edge.from}>${edge.to}`)}
            <path
                class="edge"
                class:cyclic={edge.cyclic}
                class:dim={related && !(edge.from === focus || edge.to === focus)}
                class:lit={edge.from === focus || edge.to === focus}
                d={path(edge.points)}
                marker-end="url(#arrow)"
            />
        {/each}

        {#if !layout.nodes.length}
            <text class="muted-text" x="0" y="0">No modules were loaded.</text>
        {/if}

        {#each layout.nodes as node (node.id)}
            {@const module = modules.get(node.id)!}
            {@const [dir, name] = splitPath(node.id)}
            <g
                class="node {severity(module)}"
                class:entry={module.entry}
                class:selected={selected === node.id}
                class:dim={related && !related.has(node.id)}
                transform="translate({node.x - W / 2} {node.y - H / 2})"
                role="button"
                tabindex="0"
                aria-label={node.id}
                onclick={() => choose(module)}
                onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && choose(module)}
                onpointerenter={() => (hovered = node.id)}
                onpointerleave={() => (hovered = undefined)}
            >
                <title>{module.absolutePath}</title>
                <rect width={W} height={H} rx="4" />
                <text class="dir" x="12" y="20">{dir || (module.entry ? 'entry' : '')}</text>
                <text class="name" x="12" y="38">{name}</text>
                <text class="meta" x="12" y="52">{details(module)}</text>
            </g>
        {/each}
    </PanZoom>
</Shell>

<style>
    marker path {
        fill: var(--vscode-editorLineNumber-foreground);
    }

    .edge {
        fill: none;
        stroke: var(--vscode-editorLineNumber-foreground);
        stroke-width: 1.2;
        opacity: 0.7;
        transition: opacity 120ms;
    }

    .edge.cyclic {
        stroke: var(--vscode-editorWarning-foreground);
        stroke-dasharray: 4 3;
    }

    .edge.lit {
        stroke: var(--vscode-focusBorder);
        stroke-width: 2;
        opacity: 1;
    }

    .edge.dim,
    .node.dim {
        opacity: 0.2;
    }

    .node {
        cursor: pointer;
        transition: opacity 120ms;
    }

    .node:focus {
        outline: none;
    }

    .node rect {
        fill: var(--vscode-editorWidget-background);
        stroke: var(--vscode-editorWidget-border, var(--vscode-panel-border));
    }

    .node.entry rect {
        stroke: var(--vscode-textLink-foreground);
        stroke-width: 2;
    }

    .node.warning rect {
        stroke: var(--vscode-editorWarning-foreground);
        stroke-width: 2;
    }

    .node.error rect {
        stroke: var(--vscode-editorError-foreground);
        stroke-width: 2;
    }

    .node:hover rect,
    .node:focus-visible rect,
    .node.selected rect {
        fill: var(--vscode-list-hoverBackground);
        stroke: var(--vscode-focusBorder);
        stroke-width: 2;
    }

    .node text {
        font-size: 11px;
    }

    .dir,
    .meta,
    .muted-text {
        fill: var(--vscode-descriptionForeground);
    }

    .node .name {
        fill: var(--vscode-foreground);
        font-family: var(--vscode-editor-font-family);
        font-size: 13px;
        font-weight: 600;
    }

    p.name {
        margin: 0 0 4px;
        font-weight: 600;
        overflow-wrap: anywhere;
    }

    .file {
        overflow-wrap: anywhere;
    }

    ul {
        margin: 0 0 16px;
        padding: 0;
        list-style: none;
    }

    .link {
        padding: 2px 0;
        border: none;
        color: var(--vscode-textLink-foreground);
        background: none;
        text-align: left;
        font-family: var(--vscode-editor-font-family);
    }

    .link:hover {
        text-decoration: underline;
        background: none;
    }
</style>
