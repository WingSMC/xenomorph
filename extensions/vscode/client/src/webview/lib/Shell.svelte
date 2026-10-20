<script lang="ts">
    import type { Snippet } from 'svelte';

    let {
        title,
        subtitle,
        toolbar,
        sidebar,
        children,
    }: {
        title: string;
        subtitle: string;
        toolbar: Snippet;
        sidebar: Snippet;
        children: Snippet;
    } = $props();
</script>

<header>
    <strong>{title}</strong>
    <span class="muted subtitle" title={subtitle}>{subtitle}</span>
    <div class="toolbar">{@render toolbar()}</div>
</header>
<main>
    <div class="canvas">{@render children()}</div>
    <aside>{@render sidebar()}</aside>
</main>

<style>
    header {
        display: flex;
        align-items: center;
        gap: 12px;
        height: 44px;
        padding: 0 12px;
        border-bottom: 1px solid var(--vscode-panel-border);
    }

    .subtitle {
        flex: 1;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .toolbar {
        display: flex;
        gap: 4px;
    }

    main {
        display: grid;
        grid-template-columns: minmax(0, 1fr) 280px;
        height: calc(100vh - 44px);
    }

    .canvas {
        min-width: 0;
        min-height: 0;
    }

    aside {
        overflow: auto;
        padding: 12px;
        border-left: 1px solid var(--vscode-panel-border);
        background: var(--vscode-sideBar-background);
    }

    @media (max-width: 720px) {
        main {
            grid-template-columns: 1fr;
        }

        aside {
            display: none;
        }
    }
</style>
