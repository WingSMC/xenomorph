<script lang="ts">
    import type { AstViewData, ModuleGraph, ViewKind } from '../shared/protocol';
    import AstView from './ast/AstView.svelte';
    import GraphView from './graph/GraphView.svelte';
    import { onRender } from './lib/vscode';

    let { view }: { view: ViewKind } = $props();
    let data = $state.raw<unknown>();

    onRender((next) => (data = next));
</script>

<!-- Re-create the view for every payload so per-view state (selection, collapsed nodes) resets. -->
{#key data}
    {#if data && view === 'ast'}
        <AstView data={data as AstViewData} />
    {:else if data && view === 'graph'}
        <GraphView graph={data as ModuleGraph} />
    {/if}
{/key}
