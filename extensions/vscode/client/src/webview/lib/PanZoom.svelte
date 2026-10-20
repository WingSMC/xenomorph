<script lang="ts">
    import { select } from 'd3-selection';
    import { zoom, zoomIdentity, type ZoomTransform } from 'd3-zoom';
    import type { Snippet } from 'svelte';

    let {
        width,
        height,
        label,
        minFitScale = 0.1,
        defs,
        children,
    }: {
        /** Size of the content in its own coordinates, used by `fit`. */
        width: number;
        height: number;
        label: string;
        /** `fit` never zooms out further; overflowing content is aligned to the top left. */
        minFitScale?: number;
        defs?: Snippet;
        children: Snippet;
    } = $props();

    const PADDING = 32;
    const MAX_FIT_SCALE = 1.25;

    let svg: SVGSVGElement;
    let transform = $state<ZoomTransform>(zoomIdentity);
    const behavior = zoom<SVGSVGElement, unknown>()
        .scaleExtent([0.1, 3])
        // Leave double clicks to the content (e.g. collapsing AST nodes).
        .filter((event: Event) => event.type !== 'dblclick' && !(event as MouseEvent).button)
        .on('zoom', (event: { transform: ZoomTransform }) => (transform = event.transform));

    export function fit(): void {
        const { clientWidth, clientHeight } = svg;
        if (!width || !height || !clientWidth || !clientHeight) {
            return;
        }
        const scale = Math.max(
            minFitScale,
            Math.min(
                (clientWidth - 2 * PADDING) / width,
                (clientHeight - 2 * PADDING) / height,
                MAX_FIT_SCALE,
            ),
        );
        const offset = (available: number, size: number) =>
            Math.max(PADDING, (available - size * scale) / 2);
        const next = zoomIdentity
            .translate(offset(clientWidth, width), offset(clientHeight, height))
            .scale(scale);
        select(svg).call(behavior.transform, next);
    }

    export function zoomBy(factor: number): void {
        select(svg).call(behavior.scaleBy, factor);
    }

    $effect(() => {
        select(svg).call(behavior);
        const observer = new ResizeObserver(() => fit());
        observer.observe(svg);
        return () => {
            observer.disconnect();
            select(svg).on('.zoom', null);
        };
    });

    $effect(() => {
        // Refit whenever the content size changes.
        void [width, height];
        fit();
    });
</script>

<svg bind:this={svg} role="img" aria-label={label}>
    <defs>{@render defs?.()}</defs>
    <g transform={transform.toString()}>{@render children()}</g>
</svg>

<style>
    svg {
        display: block;
        width: 100%;
        height: 100%;
        cursor: grab;
        user-select: none;
    }

    svg:active {
        cursor: grabbing;
    }
</style>
