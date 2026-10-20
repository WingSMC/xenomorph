<script lang="ts">
    import { hierarchy, tree } from 'd3-hierarchy';
    import { linkHorizontal } from 'd3-shape';
    import { SvelteSet } from 'svelte/reactivity';
    import type {
        AstNode,
        AstViewData,
        InspectPosition,
    } from '../../shared/protocol';
    import PanZoom from '../lib/PanZoom.svelte';
    import Shell from '../lib/Shell.svelte';
    import { post } from '../lib/vscode';

    let { data }: { data: AstViewData } = $props();

    const W = 176;
    const H = 44;
    // Documents with more nodes than this open with declarations collapsed.
    const INITIALLY_EXPANDED_LIMIT = 150;
    const link = linkHorizontal();

    const root: AstNode = $derived({
        kind: 'Document',
        label: `${data.ast.length} declaration${data.ast.length === 1 ? '' : 's'}`,
        children: data.ast,
    });

    let canvas: PanZoom;
    const collapsed = new SvelteSet<AstNode>(initiallyCollapsed());
    let selected = $state<AstNode>();

    // Laid out left to right: declarations stack vertically like an outline,
    // which stays readable for files with many declarations.
    const layout = $derived.by(() => {
        const nodes = tree<AstNode>()
            .nodeSize([H + 12, W + 56])(
                hierarchy(root, (node) =>
                    collapsed.has(node) ? null : node.children,
                ),
            )
            .descendants()
            .map((node) => ({ node, x: node.y, y: node.x }));
        const top = Math.min(...nodes.map((n) => n.y)) - H / 2;
        const bottom = Math.max(...nodes.map((n) => n.y)) + H / 2;
        const right = Math.max(...nodes.map((n) => n.x)) + W;
        // Shift so the content starts at (0, 0) and positions are node centres.
        for (const item of nodes) {
            item.x += W / 2;
            item.y -= top;
        }
        const position = new Map(nodes.map((item) => [item.node, item]));
        return { nodes, position, width: right, height: bottom - top };
    });

    function initiallyCollapsed(): AstNode[] {
        const count = (node: AstNode): number =>
            node.children.reduce((sum, child) => sum + count(child), 1);
        const total = data.ast.reduce((sum, node) => sum + count(node), 0);
        return total > INITIALLY_EXPANDED_LIMIT
            ? data.ast.filter((node) => node.children.length)
            : [];
    }

    function select(node: AstNode): void {
        selected = node;
        if (node.range) {
            post({ type: 'reveal', uri: data.uri, range: node.range });
        }
    }

    function toggle(node: AstNode): void {
        if (node.children.length && node !== root) {
            if (!collapsed.delete(node)) collapsed.add(node);
        }
    }

    function collapseAll(): void {
        const visit = (node: AstNode) => {
            if (node.children.length) collapsed.add(node);
            node.children.forEach(visit);
        };
        data.ast.forEach(visit);
    }

    const truncate = (text: string, length: number) =>
        text.length > length ? `${text.slice(0, length - 1)}…` : text;
    const position = (p: InspectPosition) => `${p.line + 1}:${p.character + 1}`;
</script>

<Shell title="AST" subtitle={data.label}>
    {#snippet toolbar()}
        <button onclick={() => collapsed.clear()}>Expand all</button>
        <button onclick={collapseAll}>Collapse all</button>
        <button title="Zoom out" onclick={() => canvas.zoomBy(0.8)}>−</button>
        <button title="Fit to view" onclick={() => canvas.fit()}>Fit</button>
        <button title="Zoom in" onclick={() => canvas.zoomBy(1.25)}>+</button>
    {/snippet}

    {#snippet sidebar()}
        <h2>Node</h2>
        {#if selected}
            <p class="kind">{selected.kind}</p>
            {#if selected.label}<p class="mono label">{selected.label}</p>{/if}
            <p class="muted mono">
                {selected.range
                    ? `${position(selected.range.start)} – ${position(selected.range.end)}`
                    : 'No source range'}
            </p>
        {:else}
            <p class="muted">
                Click a node to reveal its source. Double-click a parent to
                collapse or expand it.
            </p>
        {/if}

        <h2>Diagnostics ({data.diagnostics.length})</h2>
        {#each data.diagnostics as diagnostic, i (i)}
            <p class="diagnostic {diagnostic.severity}">
                <span class="mono">{position(diagnostic.range.start)}</span>
                {diagnostic.message}
            </p>
        {:else}
            <p class="muted">No diagnostics.</p>
        {/each}
    {/snippet}

    <PanZoom
        bind:this={canvas}
        width={layout.width}
        height={layout.height}
        minFitScale={0.7}
        label="Abstract syntax tree"
    >
        {#if !data.ast.length}
            <text class="empty" x={layout.width / 2} y={layout.height + 32}>
                No AST was produced. Check the diagnostics for syntax errors.
            </text>
        {/if}
        {#each layout.nodes as { node, x, y } (node.data)}
            {#if node.parent}
                {@const parent = layout.position.get(node.parent)!}
                <path
                    class="edge"
                    d={link({
                        source: [parent.x + W / 2, parent.y],
                        target: [x - W / 2, y],
                    })}
                />
            {/if}
        {/each}

        {#each layout.nodes as { node: { data: node }, x, y } (node)}
            <g
                class="node"
                class:selected={selected === node}
                transform="translate({x - W / 2} {y - H / 2})"
                role="button"
                tabindex="0"
                aria-label={node.kind}
                onclick={() => select(node)}
                ondblclick={() => toggle(node)}
                onkeydown={(e) =>
                    (e.key === 'Enter' || e.key === ' ') && select(node)}
            >
                <title
                    >{node.label
                        ? `${node.kind}: ${node.label}`
                        : node.kind}</title
                >
                <rect width={W} height={H} rx="4" />
                <text class="kind" x={W / 2} y={node.label ? 18 : 26}
                    >{truncate(node.kind, 24)}</text
                >
                {#if node.label}
                    <text class="label" x={W / 2} y="34"
                        >{truncate(node.label, 26)}</text
                    >
                {/if}
                {#if node.children.length && node !== root}
                    <text class="toggle" x={W - 10} y="14"
                        >{collapsed.has(node) ? '+' : '−'}</text
                    >
                {/if}
            </g>
        {/each}
    </PanZoom>
</Shell>

<style>
    .edge {
        fill: none;
        stroke: var(--vscode-editorLineNumber-foreground);
        opacity: 0.7;
    }

    .node {
        cursor: pointer;
    }

    .node:focus {
        outline: none;
    }

    .node rect {
        fill: var(--vscode-editorWidget-background);
        stroke: var(--vscode-editorWidget-border, var(--vscode-panel-border));
    }

    .node:hover rect,
    .node:focus-visible rect,
    .node.selected rect {
        fill: var(--vscode-list-hoverBackground);
        stroke: var(--vscode-focusBorder);
        stroke-width: 2;
    }

    text {
        text-anchor: middle;
        font-size: 12px;
    }

    .kind {
        fill: var(--vscode-symbolIcon-classForeground);
        font-weight: 600;
    }

    .label {
        fill: var(--vscode-foreground);
        font-family: var(--vscode-editor-font-family);
        font-size: 11px;
    }

    .toggle,
    .empty {
        fill: var(--vscode-descriptionForeground);
    }

    p {
        margin: 0 0 6px;
        overflow-wrap: anywhere;
    }

    p.kind {
        font-weight: 600;
    }

    p.label {
        color: var(--vscode-symbolIcon-stringForeground);
    }

    h2:not(:first-child) {
        margin-top: 20px;
    }

    .diagnostic {
        padding: 6px 8px;
        border-left: 3px solid var(--vscode-editorWarning-foreground);
        background: var(--vscode-textBlockQuote-background);
        font-size: 12px;
    }

    .diagnostic.error {
        border-color: var(--vscode-editorError-foreground);
    }
</style>
